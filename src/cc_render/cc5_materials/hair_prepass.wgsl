// Prepass (shadow maps, depth / normal / motion-vector prepasses) for CC5 hair,
// like Bevy's pbr_prepass.wgsl but with the strand coverage of hair_types.wgsl:
//   - camera prepasses keep exactly the fragments the main pass keeps (dithered
//     coverage for scalp hair), so the depth prepass has no holes;
//   - directional shadow maps write strands as a static screen door: a texel
//     survives with probability alpha * SHADOW_DENSITY, so after the shadow filter
//     hair casts a translucent shadow, as in CC5.
#import bevy_pbr::{
    prepass_io,
    pbr_bindings::material,
    pbr_types,
    pbr_functions,
    pbr_prepass_functions,
}
#import cc5::hair_types::{hair, hair_alpha, hair_keep}

// Fraction of fully opaque strand texels that cast.
const SHADOW_DENSITY: f32 = 0.6;

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hair_discard(in: prepass_io::VertexOutput) {
#ifdef VERTEX_UVS_A
    let a = hair_alpha(in.uv);
#ifdef DEPTH_CLAMP_ORTHO
    // directional shadow map
    var keep: bool;
    if hair.alpha.w > 0.5 {
        keep = hash12(floor(in.position.xy)) < a * SHADOW_DENSITY;
    } else {
        keep = a >= hair.alpha.x;
    }
#else
    let keep = hair_keep(a, in.position.xy);
#endif
    if !keep {
        discard;
    }
#endif
}

#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(in: prepass_io::VertexOutput, @builtin(front_facing) is_front: bool) -> prepass_io::FragmentOutput {
    hair_discard(in);
    var out: prepass_io::FragmentOutput;
#ifdef DEPTH_CLAMP_ORTHO
    out.frag_depth = in.clip_position_unclamped.z;
#endif
#ifdef NORMAL_PREPASS
    let double_sided = (material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    let normal = pbr_functions::prepare_world_normal(in.world_normal, double_sided, is_front);
    out.normal = vec4(normal * 0.5 + vec3(0.5), 1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.motion_vector = pbr_prepass_functions::calculate_motion_vector(in.world_position, in.previous_world_position);
#endif
    return out;
}
#else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    hair_discard(in);
}
#endif
