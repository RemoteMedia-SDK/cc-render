// CC5 RLHead / RLSkin approximation on top of Bevy's StandardMaterial.
//
// Adds, from the maps CC5 exports alongside the character (see cc5_materials/mod.rs):
//   - tiled micro normal detail, weighted by the micro normal mask;
//   - CC5's roughness: the map scaled towards rough by Original Roughness Strength,
//     plus per-region offsets (head: cheek / forehead / upper lip / chin,
//     nose / mouth / upper lid / inner lid, neck / ear; body: RGBA area mask),
//     micro roughness, and rougher at grazing angles (Edge Roughness Multiplier);
//   - mouth cavity / nostril / lip gap occlusion;
//   - specular mask;
//   - soft skin, as CC5's subsurface blur: diffuse is lit on a softened normal (the
//     geometric normal with only part of the normal map's detail, no micro normal),
//     specular on the full-detail normal with two lobes (sharp + broad);
//   - subsurface scattering approximated as wrapped diffuse past the terminator,
//     tinted by the SSS falloff colour and scaled by the SSS map and region scatter scales;
//   - transmission from the transmission map: light from behind thin parts (ears,
//     nose wings) shining through towards the viewer, deep red; none into cavities.
#import cc5::common::{PI, view_z, directional_shadow}
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, calculate_tbn_mikktspace},
    mesh_view_bindings::{view, lights},
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

struct Cc5Skin {
    rough_a: vec4<f32>,
    rough_b: vec4<f32>,
    rough_c: vec4<f32>,
    scatter_a: vec4<f32>,
    scatter_b: vec4<f32>,
    scatter_c: vec4<f32>,
    // rgb: SSS falloff colour (linear), w: scatter strength
    falloff: vec4<f32>,
    // x: tiling, y: strength, z: flip y (0/1), w: micro roughness scale
    micro: vec4<f32>,
    // x: specular, y: unmasked roughness scale, z: unmasked scatter scale, w: transmission
    misc: vec4<f32>,
    // x: inner mouth, y: lips gap, z: nostril occlusion strength, w: wrap
    ao: vec4<f32>,
    // x: original roughness strength, y: edge roughness multiplier, z: 1 = remap the map (0: keep it)
    rough: vec4<f32>,
    // bit i: texture i is present
    present: u32,
    // bit i: texture i was decoded as sRGB by the loader (data maps: undo it)
    srgb: u32,
    // bit c: channel c of mask C is a region
    mask_c_channels: u32,
    _pad1: u32,
}

@group(2) @binding(100) var<uniform> skin: Cc5Skin;
@group(2) @binding(101) var micro_normal_tex: texture_2d<f32>;
@group(2) @binding(102) var micro_normal_smp: sampler;
@group(2) @binding(103) var micro_mask_tex: texture_2d<f32>;
@group(2) @binding(104) var micro_mask_smp: sampler;
@group(2) @binding(105) var sss_tex: texture_2d<f32>;
@group(2) @binding(106) var sss_smp: sampler;
@group(2) @binding(107) var trans_tex: texture_2d<f32>;
@group(2) @binding(108) var trans_smp: sampler;
@group(2) @binding(109) var mask_a_tex: texture_2d<f32>;
@group(2) @binding(110) var mask_a_smp: sampler;
@group(2) @binding(111) var mask_b_tex: texture_2d<f32>;
@group(2) @binding(112) var mask_b_smp: sampler;
@group(2) @binding(113) var mask_c_tex: texture_2d<f32>;
@group(2) @binding(114) var mask_c_smp: sampler;
@group(2) @binding(115) var mnao_tex: texture_2d<f32>;
@group(2) @binding(116) var mnao_smp: sampler;
@group(2) @binding(117) var spec_tex: texture_2d<f32>;
@group(2) @binding(118) var spec_smp: sampler;

const MICRO_NORMAL: u32 = 0u;
const MICRO_MASK: u32 = 1u;
const SSS: u32 = 2u;
const TRANS: u32 = 3u;
const MASK_A: u32 = 4u;
const MASK_B: u32 = 5u;
const MASK_C: u32 = 6u;
const MNAO: u32 = 7u;
const SPEC: u32 = 8u;
// CC5's micro normal strength -> normal perturbation (calibrated: fine pores, no
// orange peel).
const MICRO_NORMAL_SCALE: f32 = 0.45;
// Share of the normal map's detail in the diffuse normal (the rest: geometric
// normal): light diffused under the skin smooths pores and fine wrinkles away.
const DIFFUSE_DETAIL: f32 = 0.6;
// Dual-lobe specular: the broad lobe's roughness (scale, offset) and weight.
const BROAD_ROUGHNESS: vec2<f32> = vec2<f32>(1.5, 0.12);
const BROAD_WEIGHT: f32 = 0.45;
// Share of the baked ambient occlusion applied to direct light too.
const DIRECT_AO: f32 = 0.7;
// The wrapped light is CC5's SSS falloff colour, softened (pure falloff red blotches).
const SSS_DESATURATE: f32 = 0.35;
// Extra wrap of light past the terminator on top of the SSS radius.
const EXTRA_WRAP: f32 = 0.08;
// Strength of the reddened back-light transmission.
const TRANSMISSION: f32 = 4.0;

fn has(i: u32) -> bool {
    return (skin.present & (1u << i)) != 0u;
}

fn to_srgb(c: vec4<f32>) -> vec4<f32> {
    let lo = c.rgb * 12.92;
    let hi = 1.055 * pow(max(c.rgb, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return vec4(select(hi, lo, c.rgb <= vec3(0.0031308)), c.a);
}

// The stored texel values (what the map's author wrote), whatever the loader decoded.
fn data(i: u32, c: vec4<f32>) -> vec4<f32> {
    if (skin.srgb & (1u << i)) != 0u {
        return to_srgb(c);
    }
    return c;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    // softened normal for the diffuse term (before the micro normal goes in)
    var n_geo = normalize(in.world_normal);
    if !is_front { n_geo = -n_geo; }
    let n_diffuse = normalize(mix(n_geo, pbr_input.N, DIFFUSE_DETAIL));

#ifdef VERTEX_UVS_A
    let uv = in.uv;

    // Region masks: each channel one face / body region.
    var ma = vec4(0.0);
    var mb = vec4(0.0);
    var mc = vec4(0.0);
    if has(MASK_A) { ma = data(MASK_A, textureSample(mask_a_tex, mask_a_smp, uv)); }
    if has(MASK_B) { mb = data(MASK_B, textureSample(mask_b_tex, mask_b_smp, uv)); }
    if has(MASK_C) {
        mc = data(MASK_C, textureSample(mask_c_tex, mask_c_smp, uv));
        mc *= vec4<f32>((vec4<u32>(1u, 2u, 4u, 8u) & vec4(skin.mask_c_channels)) != vec4(0u));
    }
    let unmasked = 1.0 - clamp(dot(ma, vec4(1.0)) + dot(mb, vec4(1.0)) + dot(mc, vec4(1.0)), 0.0, 1.0);

    // CC5 (unless "Keep Original Roughness"): the map scaled towards fully rough
    var rough = pbr_input.material.perceptual_roughness;
    rough = mix(rough, rough * skin.rough.x + (1.0 - skin.rough.x), skin.rough.z);
    rough += dot(ma, skin.rough_a) + dot(mb, skin.rough_b) + dot(mc, skin.rough_c) + unmasked * skin.misc.y;

    var micro_mask = 1.0;
    if has(MICRO_MASK) { micro_mask = data(MICRO_MASK, textureSample(micro_mask_tex, micro_mask_smp, uv)).r; }

#ifdef VERTEX_TANGENTS
    if has(MICRO_NORMAL) {
        var mn = data(MICRO_NORMAL, textureSample(micro_normal_tex, micro_normal_smp, uv * skin.micro.x)).xyz * 2.0 - 1.0;
        if skin.micro.z > 0.5 { mn.y = -mn.y; }
        var TBN = calculate_tbn_mikktspace(in.world_normal, in.world_tangent);
        if !is_front { mn = -mn; }
        let s = skin.micro.y * micro_mask * MICRO_NORMAL_SCALE;
        pbr_input.N = normalize(pbr_input.N + (mn.x * TBN[0] + mn.y * TBN[1]) * s);
    }
#endif
    rough += skin.micro.w * micro_mask;
    // rougher towards the silhouette
    rough += skin.rough.y * (1.0 - max(dot(pbr_input.N, pbr_input.V), 0.0));
    pbr_input.material.perceptual_roughness = clamp(rough, 0.089, 1.0);

    // Mouth cavity (r), mouth / lips gap (g), nostril (b) occlusion: 1 = open.
    var cavity = 1.0;
    if has(MNAO) {
        let m = data(MNAO, textureSample(mnao_tex, mnao_smp, uv));
        let o = clamp(vec3(1.0) - (vec3(1.0) - m.rgb) * skin.ao.xyz, vec3(0.0), vec3(1.0));
        cavity = o.x * o.y * o.z;
        pbr_input.diffuse_occlusion *= cavity;
        pbr_input.specular_occlusion *= cavity;
    }

    var spec = 1.0;
    if has(SPEC) { spec = data(SPEC, textureSample(spec_tex, spec_smp, uv)).r; }
    pbr_input.material.reflectance = skin.misc.x * spec;

    var scatter = dot(ma, skin.scatter_a) + dot(mb, skin.scatter_b) + dot(mc, skin.scatter_c) + unmasked * skin.misc.z;
    if has(SSS) { scatter *= data(SSS, textureSample(sss_tex, sss_smp, uv)).r; }

    // (no light passes through into cavities: nostrils, mouth)
    var trans = 0.0;
    if has(TRANS) { trans = data(TRANS, textureSample(trans_tex, trans_smp, uv)).r * cavity * skin.misc.w; }
    // only open, thin parts (ear rims, nose wings), not occluded hollows like the nostrils
    trans *= smoothstep(0.8, 0.95, pbr_input.diffuse_occlusion.x);
#else
    let scatter = 0.0;
    let trans = 0.0;
#endif

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    // diffuse on the softened normal, no specular (F0 = 0)
    var diffuse_input = pbr_input;
    diffuse_input.N = n_diffuse;
    diffuse_input.material.reflectance = 0.0;
    let lit = apply_pbr_lighting(diffuse_input);
    // specular on the detailed normal: sharp lobe + broad lobe
    var spec_input = pbr_input;
    spec_input.material.base_color = vec4(0.0, 0.0, 0.0, pbr_input.material.base_color.a);
    spec_input.material.emissive = vec4(0.0);
    let sharp = apply_pbr_lighting(spec_input);
    spec_input.material.perceptual_roughness = clamp(
        pbr_input.material.perceptual_roughness * BROAD_ROUGHNESS.x + BROAD_ROUGHNESS.y, 0.089, 1.0);
    let broad = apply_pbr_lighting(spec_input);
    // the baked AO (and cavity masks) also shade direct light: nostrils and creases
    // stay dark where shadow-map bias lets the key leak in
    let direct_occ = mix(vec3(1.0), smoothstep(vec3(0.45), vec3(0.95), pbr_input.diffuse_occlusion), DIRECT_AO);
    out.color = vec4((lit.rgb + mix(sharp.rgb, broad.rgb, BROAD_WEIGHT)) * direct_occ, lit.a);

    // Subsurface scattering: light wraps past the shading terminator, reddened.
    // Transmission: light from behind through thin parts (ears, nostrils), deep red.
    let w = skin.ao.w + EXTRA_WRAP;
    var sss = vec3(0.0);
    var back = vec3(0.0);
    let z = view_z(in.world_position);
    for (var i = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let l = lights.directional_lights[i];
        let ndl = dot(n_diffuse, l.direction_to_light);
        let band = clamp((ndl + w) / (1.0 + w), 0.0, 1.0) - clamp(ndl, 0.0, 1.0);
        // scattered light still needs the light to reach the skin (nostrils, under the nose)
        let lit_by = directional_shadow(i, in.world_position, in.world_normal, z);
        sss += l.color.rgb * max(band, 0.0) * lit_by;
        if trans > 0.0 {
            // light arriving from behind the surface and shining on towards the viewer
            let behind = clamp((0.15 - dot(in.world_normal, l.direction_to_light)) / 1.15, 0.0, 1.0);
            let toward = smoothstep(-0.2, 0.6, dot(pbr_input.V, -l.direction_to_light));
            let through = behind * toward;
            let s = directional_shadow(i, in.world_position - vec4(in.world_normal * 0.004, 0.0), -in.world_normal, z);
            back += l.color.rgb * through * s;
        }
    }
    let diffuse = pbr_input.material.base_color.rgb * (1.0 - pbr_input.material.metallic);
    let red = skin.falloff.rgb * skin.falloff.rgb;
    out.color = vec4(
        out.color.rgb
            + (sss * mix(skin.falloff.rgb, vec3(1.0), SSS_DESATURATE) * skin.falloff.w * scatter * pbr_input.diffuse_occlusion
                + back * red * trans * TRANSMISSION) * diffuse * view.exposure / PI,
        out.color.a,
    );
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
