// CC5 RLEye approximation on the cornea mesh, which then draws the whole eye
// opaquely (the eyeball mesh beneath carries the same layout; see mod.rs).
// Forward only.
//
// In cornea UV (eye centre at 0.5, 0.5):
//   - iris: the base colour map's iris, scaled to Iris UV Radius, seen through
//     the cornea: refracted (IoR) parallax
//     into an iris plane that lies deeper towards the centre (Iris Depth Scale),
//     pupil resized (Pupil Scale), tinted (Iris Color * Iris Color Brightness),
//     lit with the iris normal map;
//   - limbus: a darker ring at the iris edge (Limbus UV Width / Limbus Dark Scale);
//   - sclera: the Sclera map * ScleraBrightness, with the sclera normal map tiled
//     (Sclera Normal UV Scale), darkened towards the corners (Shadow Radius /
//     Hardness, Eye Corner Darkness Color) and by the baked AO;
//   - the wet cornea: a sharp Fresnel specular (IoR, Iris / Sclera Roughness,
//     Specular Scale) on the surface normal domed over the iris (the mesh barely
//     bulges; a real cornea gathers the highlights): the catch-light.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_bindings::{base_color_texture, base_color_sampler},
    forward_io::{VertexOutput, FragmentOutput},
}
#import bevy_pbr::mesh_view_bindings::{lights, view}
#import cc5::common::{data, cotangent_frame, directional_radiance}

struct Cc5Eye {
    // x: iris radius (UV), y: depth scale, z: pupil scale, w: 1 / IoR
    iris: vec4<f32>,
    // rgb: iris tint (linear), w: inner iris scale
    iris_color: vec4<f32>,
    // x: limbus half width (UV), y: limbus dark scale, z: iris UV scale,
    // w: iris radius in the base colour map (UV)
    limbus: vec4<f32>,
    // x: brightness, y: normal strength (Sclera Flatten Normal), z: normal tiling, w: sclera UV radius
    sclera: vec4<f32>,
    // rgb: corner darkness multiplier, w: shadow radius
    corner: vec4<f32>,
    // x: specular scale, y: iris roughness, z: sclera roughness, w: shadow hardness
    spec: vec4<f32>,
    present: u32,
    srgb: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(2) @binding(100) var<uniform> eye: Cc5Eye;
@group(2) @binding(101) var sclera_tex: texture_2d<f32>;
@group(2) @binding(102) var sclera_smp: sampler;
@group(2) @binding(103) var iris_n_tex: texture_2d<f32>;
@group(2) @binding(104) var iris_n_smp: sampler;
@group(2) @binding(105) var sclera_n_tex: texture_2d<f32>;
@group(2) @binding(106) var sclera_n_smp: sampler;
@group(2) @binding(107) var iris_mask_tex: texture_2d<f32>;
@group(2) @binding(108) var iris_mask_smp: sampler;

const SCLERA: u32 = 0u;
const IRIS_N: u32 = 1u;
const SCLERA_N: u32 = 2u;
const IRIS_MASK: u32 = 3u;

// Pupil radius in CC's eye textures, as a fraction of the iris radius.
const PUPIL_FRACTION: f32 = 0.36;
// Depth of the iris plane under the cornea apex, in UV units (the anterior
// chamber, ~2.5 mm of a ~24 mm eye).
const IRIS_DEPTH: f32 = 0.06;
// Extra curvature of the cornea over the eyeball: normal tilt per UV of offset.
const CORNEA_BULGE: f32 = 1.2;
// How far the corners darken towards Eye Corner Darkness Color (the occlusion
// overlay adds the lids' shadow on top).
const CORNER_SHADE: f32 = 0.6;
// The iris catches light focused by the cornea: brighter than plain diffuse.
const IRIS_LIGHT: f32 = 1.9;
// Catch-lights: each light reflected as a softbox disc of this angular radius
// (cosine) and edge, at this brightness (portrait lights are area lights)...
const CATCH_COS: f32 = 0.996;
const CATCH_EDGE: f32 = 0.004;
const CATCH_GAIN: f32 = 25.0;
// ...plus CC5's eye highlight: a softbox fixed relative to the camera (up and to
// the right of it), as bright as the brightest light, always in the iris.
const VIEW_CATCH_DIR: vec3<f32> = vec3<f32>(0.9, 0.45, 1.0);
const VIEW_CATCH_COS: f32 = 0.992;

fn has(i: u32) -> bool {
    return (eye.present & (1u << i)) != 0u;
}

fn sample_normal(t: texture_2d<f32>, s: sampler, i: u32, uv: vec2<f32>) -> vec3<f32> {
    return data(eye.srgb, i, textureSample(t, s, uv)).xyz * 2.0 - 1.0;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let dp1 = dpdx(in.world_position.xyz);
    let dp2 = dpdy(in.world_position.xyz);
#ifdef VERTEX_UVS_A
    let uv = in.uv;
#else
    let uv = vec2(0.5);
#endif
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let N = pbr_input.N;
    let V = pbr_input.V;
    let TBN = cotangent_frame(N, dp1, dp2, duv1, duv2);

    let R = eye.iris.x;
    let w = max(eye.limbus.x, 1e-3);
    let d = uv - 0.5;
    let r = length(d);

    // --- refracted iris lookup ---
    let refr = refract(-V, N, eye.iris.w);
    let ts = vec3(dot(refr, TBN[0]), dot(refr, TBN[1]), dot(refr, TBN[2]));
    let depth = IRIS_DEPTH * eye.iris.y * max(1.0 - (r / R) * (r / R), 0.0);
    var iuv = uv + ts.xy / max(-ts.z, 0.25) * depth;
    // stay inside the iris disc
    var id = iuv - 0.5;
    var ir = length(id);
    if ir > R * 0.98 {
        id *= R * 0.98 / ir;
        ir = R * 0.98;
    }
    // pupil scale: remap the radius so the pupil grows / shrinks, the iris edge stays
    let p0 = R * PUPIL_FRACTION;
    let p = p0 * eye.iris.z;
    var src: f32;
    if ir < p {
        src = ir * p0 / max(p, 1e-4);
    } else {
        src = p0 + (ir - p) * (R - p0) / max(R - p, 1e-4);
    }
    // into the base colour map, whose iris is drawn at radius limbus.w
    iuv = 0.5 + id * (src / max(ir, 1e-5)) * (eye.limbus.w / R);
    var iris = textureSample(base_color_texture, base_color_sampler, iuv).rgb * eye.iris_color.rgb * IRIS_LIGHT;
    // limbal ring: darker band around the iris edge
    let ring = exp(-pow((r - R) / w, 2.0));
    iris *= 1.0 - ring * clamp(eye.limbus.y * 0.1, 0.0, 0.85);

    // --- sclera ---
    var sclera = textureSample(base_color_texture, base_color_sampler, uv).rgb;
    if has(SCLERA) {
        let suv = d / max(eye.sclera.w, 0.1) + 0.5;
        sclera = textureSample(sclera_tex, sclera_smp, suv).rgb;
    }
    sclera *= eye.sclera.x;
    // darkening towards the corners of the visible eye
    let radius = eye.corner.w;
    let width = 0.25 * (1.0 - clamp(eye.spec.w, -1.0, 1.0)) + 0.02;
    let shade = smoothstep(radius - width, radius + width, r * 2.0);
    sclera *= mix(vec3(1.0), eye.corner.rgb, shade * CORNER_SHADE);
    // the limbal ring also fades the sclera into the iris
    sclera *= 1.0 - ring * clamp(eye.limbus.y * 0.06, 0.0, 0.6);

    let iris_m = 1.0 - smoothstep(R - w * 0.5, R + w * 0.25, r);
    let color = mix(sclera, iris, iris_m);

    // --- normals: iris map in the iris, tiled sclera map outside; the cornea's
    // own surface stays smooth for the specular ---
    var n_ts = vec3(0.0, 0.0, 1.0);
    if has(IRIS_N) {
        let n_i = sample_normal(iris_n_tex, iris_n_smp, IRIS_N, iuv);
        n_ts = mix(n_ts, n_i, iris_m);
    }
    if has(SCLERA_N) {
        let n_s = sample_normal(sclera_n_tex, sclera_n_smp, SCLERA_N, uv * eye.sclera.z);
        n_ts = n_ts + vec3(n_s.xy * eye.sclera.y * 0.5 * (1.0 - iris_m), 0.0);
    }
    let N_diffuse = normalize(TBN * vec3(n_ts.x, -n_ts.y, max(n_ts.z, 0.05)));

    // baked eye AO (the cornea's occlusion map): soften it, CC5 shades the corners itself
    let ao = mix(1.0, pbr_input.diffuse_occlusion.x, 0.5);

    // --- lighting: diffuse eye under the cornea, then the cornea's reflection ---
    var diffuse_input = pbr_input;
    diffuse_input.N = N_diffuse;
    diffuse_input.material.base_color = vec4(color, 1.0);
    diffuse_input.material.metallic = 0.0;
    diffuse_input.material.reflectance = 0.0;
    diffuse_input.material.perceptual_roughness = 1.0;
    diffuse_input.material.diffuse_transmission = 0.0;
    diffuse_input.diffuse_occlusion = vec3(ao);
    diffuse_input.specular_occlusion = 1.0;
    let lit = apply_pbr_lighting(diffuse_input);

    // the cornea's dome over the iris
    let dome = (1.0 - smoothstep(R, R + 3.0 * w, r)) * CORNEA_BULGE;
    let N_spec = normalize(TBN * vec3(d * dome, 1.0));

    var spec_input = pbr_input;
    spec_input.N = N_spec;
    spec_input.material.base_color = vec4(0.0, 0.0, 0.0, 1.0);
    spec_input.material.metallic = 0.0;
    spec_input.material.diffuse_transmission = 0.0;
    spec_input.material.emissive = vec4(0.0);
    // Fresnel F0 of the tear film / cornea from the IoR (Bevy: F0 = 0.16 reflectance^2)
    let f0 = pow((1.0 - eye.iris.w) / (1.0 + eye.iris.w), 2.0);
    spec_input.material.reflectance = sqrt(f0 / 0.16);
    spec_input.material.perceptual_roughness = clamp(mix(eye.spec.z, eye.spec.y, iris_m) + 0.06, 0.089, 1.0);
    spec_input.diffuse_occlusion = vec3(ao);
    spec_input.specular_occlusion = ao;
    let refl = apply_pbr_lighting(spec_input);

    // softbox catch-lights on the smooth cornea
    let Rv = reflect(-V, N_spec);
    var catch_light = vec3(0.0);
    var brightest = vec3(0.0);
    for (var i = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let c = dot(Rv, lights.directional_lights[i].direction_to_light);
        let radiance = directional_radiance(i);
        catch_light += radiance * smoothstep(CATCH_COS - CATCH_EDGE, CATCH_COS, c);
        if dot(radiance, vec3(1.0)) > dot(brightest, vec3(1.0)) { brightest = radiance; }
    }
    let view_dir = normalize((view.world_from_view * vec4(normalize(VIEW_CATCH_DIR), 0.0)).xyz);
    catch_light += brightest * smoothstep(VIEW_CATCH_COS - CATCH_EDGE, VIEW_CATCH_COS, dot(Rv, view_dir));
    catch_light *= f0 * CATCH_GAIN * ao;

    var out: FragmentOutput;
    out.color = vec4(lit.rgb + (refl.rgb + catch_light) * eye.spec.x / 0.8, 1.0);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
