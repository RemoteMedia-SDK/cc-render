// CC5 RLTeethGum / RLTongue approximation on top of Bevy's StandardMaterial.
// Forward only.
//
//   - colour: the base colour map, split by the gums mask (tongue: no mask) into
//     teeth and gums, each with its Brightness and Desaturation (negative = more
//     saturated);
//   - the gradient AO map (bright at the front of the mouth, dark at the back)
//     blends Front / Back roughness, specular and AO strength: the inside of the
//     mouth falls dark and matte;
//   - tiled micro normal (separate tiling / strength for teeth and gums);
//   - a little wrapped-diffuse scattering (Teeth / Gums / _Scatter).
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing, calculate_tbn_mikktspace},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::lights,
}
#import cc5::common::{PI, data, luminance, directional_radiance}

struct Cc5Mouth {
    // x: brightness, y: desaturation, z: scatter, w: micro normal strength
    teeth: vec4<f32>,
    gums: vec4<f32>,
    // x: roughness, y: specular, z: AO strength
    front: vec4<f32>,
    back: vec4<f32>,
    // x: teeth tiling, y: gums tiling, z: flip y
    micro: vec4<f32>,
    // rgb: teeth edge colour (unused), w: unused
    edge_color: vec4<f32>,
    present: u32,
    srgb: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(2) @binding(100) var<uniform> mouth: Cc5Mouth;
@group(2) @binding(101) var micro_tex: texture_2d<f32>;
@group(2) @binding(102) var micro_smp: sampler;
@group(2) @binding(103) var grad_tex: texture_2d<f32>;
@group(2) @binding(104) var grad_smp: sampler;
@group(2) @binding(105) var gums_tex: texture_2d<f32>;
@group(2) @binding(106) var gums_smp: sampler;

const MICRO: u32 = 0u;
const GRAD: u32 = 1u;
const GUMS: u32 = 2u;

fn has(i: u32) -> bool {
    return (mouth.present & (1u << i)) != 0u;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
#ifdef VERTEX_UVS_A
    let uv = in.uv;
#else
    let uv = vec2(0.5);
#endif
    var gums = 0.0;
    if has(GUMS) { gums = data(mouth.srgb, GUMS, textureSample(gums_tex, gums_smp, uv)).r; }
    var front = 1.0;
    if has(GRAD) { front = data(mouth.srgb, GRAD, textureSample(grad_tex, grad_smp, uv)).r; }
    let part = mix(mouth.teeth, mouth.gums, gums);

    var col = pbr_input.material.base_color.rgb;
    col = mix(col, vec3(luminance(col)), part.y) * part.x;
    pbr_input.material.base_color = vec4(max(col, vec3(0.0)), pbr_input.material.base_color.a);

    let side = mix(mouth.back, mouth.front, front);
    pbr_input.material.perceptual_roughness = clamp(side.x, 0.089, 1.0);
    // CC5's specular 0..1 as Bevy reflectance (0.5 = F0 4%)
    pbr_input.material.reflectance = side.y;
    let ao = clamp(pow(max(front, 0.0), side.z), 0.0, 1.0);
    pbr_input.diffuse_occlusion *= ao;
    pbr_input.specular_occlusion *= ao;

#ifdef VERTEX_TANGENTS
    if has(MICRO) {
        let tiling = mix(mouth.micro.x, mouth.micro.y, gums);
        var mn = data(mouth.srgb, MICRO, textureSample(micro_tex, micro_smp, uv * tiling)).xyz * 2.0 - 1.0;
        if mouth.micro.z > 0.5 { mn.y = -mn.y; }
        let TBN = calculate_tbn_mikktspace(in.world_normal, in.world_tangent);
        if !is_front { mn = -mn; }
        pbr_input.N = normalize(pbr_input.N + (mn.x * TBN[0] + mn.y * TBN[1]) * part.w);
    }
#endif

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    // light wrapping past the terminator, reddened (gums / tongue more than teeth)
    var sss = vec3(0.0);
    for (var i = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let ndl = dot(pbr_input.N, lights.directional_lights[i].direction_to_light);
        let wrap = clamp((ndl + 0.5) / 1.5, 0.0, 1.0) - clamp(ndl, 0.0, 1.0);
        sss += directional_radiance(i) * max(wrap, 0.0);
    }
    out.color = vec4(out.color.rgb + sss * col * vec3(1.0, 0.45, 0.35) * part.z * 0.3 * ao / PI, out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
