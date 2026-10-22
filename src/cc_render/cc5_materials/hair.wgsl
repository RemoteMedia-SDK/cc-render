// CC5 RLHair approximation on top of Bevy's StandardMaterial (forward only).
//
// From the maps and variables CC5 exports with each hair / brow / lash material:
//   - colour: the diffuse map tinted by the root -> tip gradient (root map: 0 at
//     the root, 1 at the tip; RootColor / TipColor with their strengths), then the
//     two "horizontal dyes" picked by ID-map level (a start / peak / end ramp),
//     optionally masked by the root map (CC5 bakes this into a base colour map);
//   - per-strand variation from the ID map (brightness and highlight shift);
//   - strand tangent from the flow map (UV-space direction) or TangentVectorColor,
//     in a tangent frame from screen-space derivatives (hair exports no tangents);
//   - lighting: Bevy's diffuse (environment, directional lights, shadows) without
//     its GGX specular, plus per directional light a Kajiya-Kay / Marschner-style
//     primary highlight (R, white, shifted towards the tip), a broader secondary
//     highlight (TRT, hair-coloured, shifted towards the root) and transmission
//     (TT) for back light; strengths from Specular / Secondary Specular /
//     Transmission Strength, sharpness from the roughness map;
//   - alpha: CC5 alpha-blends hair into a soft mass. Here scalp hair is drawn with
//     dithered coverage (hair_types.wgsl: a soft ramp around the cutoff, a slightly
//     blurred opacity map) that the camera's TAA resolves into partial coverage;
//     brows / lashes stay alpha-tested (TAA smooths their edges).
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, lights, clusterable_objects},
    mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_view_types::POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE,
    clustered_forward as clustering,
    lighting::getDistanceAttenuation,
    shadows::{fetch_point_shadow, fetch_spot_shadow},
}
#import cc5::common::{PI, data, luminance, to_srgb, to_linear, cotangent_frame, view_z, directional_shadow, directional_radiance}
#import cc5::hair_types::{
    hair, root_tex, root_smp, id_tex, id_smp, flow_tex, flow_smp, spec_tex, spec_smp,
    hair_alpha, hair_coverage, hair_keep,
}

const ROOT: u32 = 0u;
const ID: u32 = 1u;
const FLOW: u32 = 2u;
const SPEC: u32 = 3u;

// Weight of the root/tip tint in the soft-light blend (calibrated against CC5's
// viewport: its baked colour stays close to the diffuse map's).
const ROOT_TIP_TINT: f32 = 0.5;
// Diffuse albedo scale (calibrated against CC5's viewport). The occlusion towards
// the roots (inner layers of the hair volume, which CC5 shades darker than single
// cards lit in isolation) is hair.tint.w (Cc5Materials::hair_root_occlusion).
const DIFFUSE_SCALE: f32 = 0.55;
// Strand highlights: CC5's are broad and soft, never crisp white strands.
const MIN_ROUGHNESS: f32 = 0.3;
const MAX_EXPONENT: f32 = 160.0;
const PRIMARY_SCALE: f32 = 0.15;
// Broad isotropic sheen (Bevy's specular, environment included).
const SHEEN: f32 = 0.3;
const SHEEN_ROUGHNESS: f32 = 0.7;
// CC5's hair reads ash next to its diffuse map: saturation of the diffuse colour.
const SATURATION: f32 = 0.65;

fn has(i: u32) -> bool {
    return (hair.present & (1u << i)) != 0u;
}

// CC5's colour blend modes (a 0..7 combobox whose names are not exported):
// 0 behaves as a soft-light tint that keeps the diffuse map's strand detail,
// 1 as normal (the colour replaces the map); the others are treated as multiply,
// screen and overlay. Blends in sRGB space, like an image editor.
fn blend(mode: f32, base: vec3<f32>, layer: vec3<f32>) -> vec3<f32> {
    let a = to_srgb(vec4(base, 1.0)).rgb;
    let b = to_srgb(vec4(layer, 1.0)).rgb;
    var c: vec3<f32>;
    if mode < 0.5 {
        c = (1.0 - 2.0 * b) * a * a + 2.0 * a * b;
    } else if mode < 1.5 {
        c = b;
    } else if mode < 2.5 {
        c = a * b;
    } else if mode < 3.5 {
        c = 1.0 - (1.0 - a) * (1.0 - b);
    } else {
        c = select(1.0 - 2.0 * (1.0 - a) * (1.0 - b), 2.0 * a * b, a < vec3(0.5));
    }
    return to_linear(clamp(c, vec3(0.0), vec3(1.0)));
}

// Weight of a dye at ID-map level `id`: rises from range.x to range.y, falls to range.z.
fn dye_weight(range: vec4<f32>, id: f32, root: f32) -> f32 {
    var w: f32;
    if id <= range.y {
        w = clamp((id - range.x) / max(range.y - range.x, 1e-3), 0.0, 1.0);
    } else {
        w = 1.0 - clamp((id - range.y) / max(range.z - range.y, 1e-3), 0.0, 1.0);
    }
    w = w * w * (3.0 - 2.0 * w);
    if range.w > 1.5 {
        w *= 1.0 - root;
    } else if range.w > 0.5 {
        w *= root;
    }
    return w;
}

// Kajiya-Kay strand highlight: the tangent shifted along the normal, a lobe
// around the cone of mirror directions, normalised like Blinn-Phong for its exponent.
// One light's strand highlights (primary, hair-coloured TRT, transmission) per
// unit radiance.
fn strand_light(L: vec3<f32>, V: vec3<f32>, N: vec3<f32>, T: vec3<f32>, jitter: f32, e1: f32, e2: f32, spec_mask: f32, tint: vec3<f32>, col: vec3<f32>) -> vec3<f32> {
    let H = normalize(L + V);
    // cards: light the strand, not the card's side (soft wrap on the card normal)
    let facing = clamp(dot(N, L) * 0.5 + 0.5, 0.0, 1.0);
    // hair's Fresnel reflectance at the cuticle
    let f0 = 0.046;
    let r = strand_lobe(T, N, H, hair.lobes.x + jitter, e1) * f0 * hair.light.y * spec_mask * PRIMARY_SCALE;
    let trt = strand_lobe(T, N, H, hair.lobes.y - jitter, e2) * f0 * hair.light.z * spec_mask * 0.5;
    let tt = pow(clamp(dot(-L, V), 0.0, 1.0), 6.0) * 0.3 * hair.light.w;
    return facing * (vec3(r) + tint * trt) + tint * col * tt;
}

fn strand_lobe(T: vec3<f32>, N: vec3<f32>, H: vec3<f32>, shift: f32, exponent: f32) -> f32 {
    let Ts = normalize(T + N * shift);
    let th = dot(Ts, H);
    let s = sqrt(max(1.0 - th * th, 0.0));
    return pow(s, exponent) * (exponent + 2.0) / (8.0 * PI);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // derivatives in uniform control flow, before the alpha test
    let dp1 = dpdx(in.world_position.xyz);
    let dp2 = dpdy(in.world_position.xyz);
#ifdef VERTEX_UVS_A
    let uv = in.uv;
#else
    let uv = vec2(0.5);
#endif
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let raw_alpha = hair_alpha(uv);

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    // the material is alpha-tested at 0.5: keep / drop exactly as the prepass does
    pbr_input.material.base_color.a = select(0.0, 1.0, hair_keep(raw_alpha, in.position.xy));
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    let alpha = hair_coverage(raw_alpha);

    var root = 1.0;
    if has(ROOT) { root = data(hair.srgb, ROOT, textureSample(root_tex, root_smp, uv)).r; }
    if hair.color.y > 0.5 { root = 1.0 - root; }
    var id = 0.5;
    if has(ID) { id = data(hair.srgb, ID, textureSample(id_tex, id_smp, uv)).r; }

    // --- colour ---
    let map = pbr_input.material.base_color.rgb;
    var col = mix(vec3(1.0), map, hair.color.z);
    let rt = mix(hair.root_color.rgb, hair.tip_color.rgb, root);
    let rt_w = mix(hair.root_color.w, hair.tip_color.w, root);
    let mode = hair.color.x;
    if mode < 0.5 {
        col = mix(col, blend(mode, col, rt), rt_w * ROOT_TIP_TINT);
    } else {
        col = mix(col, blend(mode, col, rt), rt_w);
    }
    let dye1_mode = hair.color.w % 8.0;
    let dye2_mode = floor(hair.color.w / 8.0);
    col = mix(col, blend(dye1_mode, col, hair.first_dye.rgb), hair.first_dye.w * dye_weight(hair.first_dye_range, id, root));
    col = mix(col, blend(dye2_mode, col, hair.second_dye.rgb), hair.second_dye.w * dye_weight(hair.second_dye_range, id, root));
    // per-strand brightness variation
    col *= 1.0 + (id - 0.5) * hair.lobes.w;
    col = mix(vec3(luminance(col)), col, SATURATION);
    col *= hair.tint.rgb;

    // --- strand frame ---
    let N = pbr_input.N;
    let V = pbr_input.V;
    let TBN = cotangent_frame(N, dp1, dp2, duv1, duv2);
    var dir = hair.tangent.xyz;
    if has(FLOW) {
        let f = data(hair.srgb, FLOW, textureSample(flow_tex, flow_smp, uv)).rg * 2.0 - 1.0;
        if dot(f, f) > 0.01 {
            // image-space direction, green up (-v)
            dir = vec3(f.x, -f.y * hair.maps.z, 0.0);
        }
    }
    var T = TBN * dir;
    T = T - N * dot(N, T);
    if dot(T, T) < 1e-8 { T = TBN[1]; }
    T = normalize(T);

    // --- highlights ---
    // roughness map (glTF metallic-roughness G) scaled by its strength; Blinn-Phong
    // exponent of the GGX alpha; the secondary lobe is broader.
    let rough = clamp(pbr_input.material.perceptual_roughness * hair.maps.y, MIN_ROUGHNESS, 1.0);
    let a = rough * rough;
    let e1 = clamp(2.0 / (a * a) - 2.0, 4.0, MAX_EXPONENT);
    let e2 = max(e1 * 0.25, 2.0);
    var spec_mask = 1.0;
    if has(SPEC) {
        spec_mask = mix(1.0, data(hair.srgb, SPEC, textureSample(spec_tex, spec_smp, uv)).r, hair.maps.x);
    }
    let jitter = (id - 0.5) * hair.lobes.z;
    let tint = min(mix(vec3(luminance(col)), col, 0.6) * 3.0, vec3(1.0));
    let ao = pbr_input.diffuse_occlusion.x;
    // thin strand edges (partial coverage) and occluded strands highlight less: no
    // sub-pixel glitter on lashes and wisps
    let spec_ao = mix(mix(1.0, ao, 0.5), ao, hair.maps.w) * alpha * hair.tangent.w
        * mix(1.0 - hair.tint.w, 1.0, smoothstep(0.0, 0.7, root));
    let z = view_z(in.world_position);
    var spec = vec3(0.0);
    for (var i = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let L = lights.directional_lights[i].direction_to_light;
        let shadow = directional_shadow(i, in.world_position, in.world_normal, z);
        spec += directional_radiance(i) * shadow * strand_light(L, V, N, T, jitter, e1, e2, spec_mask, tint, col);
    }
    // the same for point and spot lights (CC5's lights are spot lights)
    let cluster = clustering::fragment_cluster_index(pbr_input.frag_coord.xy, z, pbr_input.is_orthographic);
    let oc = clustering::unpack_offset_and_counts(cluster);
    for (var i = oc[0]; i < oc[0] + oc[1] + oc[2]; i = i + 1u) {
        let lid = clustering::get_clusterable_object_id(i);
        let light = &clusterable_objects.data[lid];
        let to_light = (*light).position_radius.xyz - in.world_position.xyz;
        let L = normalize(to_light);
        var radiance = (*light).color_inverse_square_range.rgb
            * getDistanceAttenuation(dot(to_light, to_light), (*light).color_inverse_square_range.w);
        let is_spot = i >= oc[0] + oc[1];
        if is_spot {
            var spot_dir = vec3((*light).light_custom_data.x, 0.0, (*light).light_custom_data.y);
            spot_dir.y = sqrt(max(0.0, 1.0 - spot_dir.x * spot_dir.x - spot_dir.z * spot_dir.z));
            if ((*light).flags & POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE) != 0u {
                spot_dir.y = -spot_dir.y;
            }
            let a = saturate(dot(-spot_dir, L) * (*light).light_custom_data.z + (*light).light_custom_data.w);
            radiance *= a * a;
        }
        if ((*light).flags & POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            if is_spot {
                radiance *= fetch_spot_shadow(lid, in.world_position, in.world_normal, (*light).shadow_map_near_z);
            } else {
                radiance *= fetch_point_shadow(lid, in.world_position, in.world_normal);
            }
        }
        spec += radiance * view.exposure * strand_light(L, V, N, T, jitter, e1, e2, spec_mask, tint, col);
    }

    // --- diffuse plus a broad sheen: Bevy's lighting (environment included) with a
    // rough isotropic specular; the strand lobes above add the anisotropic highlights ---
    let volume = mix(1.0 - hair.tint.w, 1.0, smoothstep(0.0, 0.7, root));
    pbr_input.material.base_color = vec4(col * hair.light.x * DIFFUSE_SCALE * volume, 1.0);
    pbr_input.material.reflectance = SHEEN * hair.light.y * spec_mask;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.perceptual_roughness = SHEEN_ROUGHNESS;
    pbr_input.specular_occlusion *= spec_ao;
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = vec4(out.color.rgb + spec * spec_ao, out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
