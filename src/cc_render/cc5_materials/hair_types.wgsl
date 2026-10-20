// CC5 RLHair: the material uniform and textures, and the strand coverage test
// shared by the main pass (hair.wgsl) and the prepass (hair_prepass.wgsl). Both
// passes must keep exactly the same fragments: with a depth prepass, a fragment
// the prepass writes but the main pass drops leaves a hole.
#define_import_path cc5::hair_types

#import bevy_pbr::{
    pbr_bindings,
    pbr_types,
    mesh_view_bindings::view,
}

struct Cc5Hair {
    // rgb: linear colour, w: strength (0 when the colour change is off)
    root_color: vec4<f32>,
    tip_color: vec4<f32>,
    // rgb: linear colour, w: strength
    first_dye: vec4<f32>,
    // xyz: ID-map levels where the dye starts / peaks / ends, w: 0 everywhere,
    // 1 masked by the root map (tips), 2 by its inverse (roots)
    first_dye_range: vec4<f32>,
    second_dye: vec4<f32>,
    second_dye_range: vec4<f32>,
    // x: root/tip blend mode, y: invert root/tip, z: base colour map strength,
    // w: dye blend modes (1st + 8 * 2nd)
    color: vec4<f32>,
    // x: diffuse, y: specular (R), z: secondary specular (TRT), w: transmission (TT) strength
    light: vec4<f32>,
    // x: specular map strength, y: roughness map strength, z: flow map green sign,
    // w: AO occludes all lighting
    maps: vec4<f32>,
    // xyz: strand direction without a flow map (tangent space), w: highlight scale
    tangent: vec4<f32>,
    // x: primary shift, y: secondary shift, z: per-strand shift jitter,
    // w: per-strand brightness jitter
    lobes: vec4<f32>,
    // x: alpha cutoff, y: soft coverage ramp around it, z: opacity map mip bias,
    // w: 1 = dithered coverage (soft strands, resolved by TAA), 0 = alpha test
    alpha: vec4<f32>,
    present: u32,
    srgb: u32,
    pad_a: u32,
    pad_b: u32,
}

@group(2) @binding(100) var<uniform> hair: Cc5Hair;
@group(2) @binding(101) var root_tex: texture_2d<f32>;
@group(2) @binding(102) var root_smp: sampler;
@group(2) @binding(103) var id_tex: texture_2d<f32>;
@group(2) @binding(104) var id_smp: sampler;
@group(2) @binding(105) var flow_tex: texture_2d<f32>;
@group(2) @binding(106) var flow_smp: sampler;
@group(2) @binding(107) var spec_tex: texture_2d<f32>;
@group(2) @binding(108) var spec_smp: sampler;

// The strand opacity at `uv` (call in uniform control flow: implicit derivatives).
fn hair_alpha(uv: vec2<f32>) -> f32 {
    var a = pbr_bindings::material.base_color.a;
    let tuv = (pbr_bindings::material.uv_transform * vec3(uv, 1.0)).xy;
    let t = textureSampleBias(pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, tuv, view.mip_bias + hair.alpha.z);
    if (pbr_bindings::material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u {
        a *= t.a;
    }
    return a;
}

// Soft coverage of a strand texel (0..1) around the cutoff.
fn hair_coverage(a: f32) -> f32 {
    return clamp((a - hair.alpha.x) / max(hair.alpha.y, 1e-3) + 0.5, 0.0, 1.0);
}

// Per-frame seed from the camera's TAA jitter (in the projection): the dither
// changes every frame and TAA averages it into partial coverage.
fn frame_seed() -> f32 {
    let j = view.clip_from_view[2].xy;
    return fract(sin(dot(j * 4096.0, vec2(12.9898, 78.233))) * 43758.5453);
}

// Interleaved gradient noise at a pixel, offset per frame.
fn dither(pixel: vec2<f32>) -> f32 {
    let p = floor(pixel) + frame_seed() * vec2(113.0, 71.0);
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
}

// Whether the fragment at `pixel` with opacity `a` is drawn (main pass and view prepass).
fn hair_keep(a: f32, pixel: vec2<f32>) -> bool {
    if hair.alpha.w > 0.5 {
        return dither(pixel) < hair_coverage(a);
    }
    return a >= hair.alpha.x;
}
