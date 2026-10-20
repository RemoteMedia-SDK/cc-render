// Helpers shared by the CC5 material shaders (see cc5_materials/mod.rs).
#define_import_path cc5::common

#import bevy_pbr::{
    mesh_view_bindings::{view, lights},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    shadows::fetch_directional_shadow,
}

const PI: f32 = 3.141592653589793;

fn to_srgb(c: vec4<f32>) -> vec4<f32> {
    let lo = c.rgb * 12.92;
    let hi = 1.055 * pow(max(c.rgb, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return vec4(select(hi, lo, c.rgb <= vec3(0.0031308)), c.a);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3(0.0)) + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

// The stored texel values (what the map's author wrote), whatever the loader
// decoded: bit `i` of `srgb_bits` says texture `i` was decoded as sRGB.
fn data(srgb_bits: u32, i: u32, c: vec4<f32>) -> vec4<f32> {
    if (srgb_bits & (1u << i)) != 0u {
        return to_srgb(c);
    }
    return c;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

// Tangent frame from screen-space derivatives (for meshes exported without
// tangents): columns T (along +u), B (along +v), N. Compute dp/duv in uniform
// control flow (before any discard) and pass them in.
fn cotangent_frame(N: vec3<f32>, dp1: vec3<f32>, dp2: vec3<f32>, duv1: vec2<f32>, duv2: vec2<f32>) -> mat3x3<f32> {
    let dp2perp = cross(dp2, N);
    let dp1perp = cross(N, dp1);
    let T = dp2perp * duv1.x + dp1perp * duv2.x;
    let B = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = inverseSqrt(max(max(dot(T, T), dot(B, B)), 1e-20));
    return mat3x3(T * invmax, B * invmax, N);
}

// View-space depth of a world position, as Bevy's shadow cascades expect it.
fn view_z(world_position: vec4<f32>) -> f32 {
    return dot(vec4(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z
    ), world_position);
}

// Shadow factor of directional light `i` at a surface point (1 = lit).
fn directional_shadow(i: u32, world_position: vec4<f32>, world_normal: vec3<f32>, z: f32) -> f32 {
    if (lights.directional_lights[i].flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return 1.0;
    }
    return fetch_directional_shadow(i, world_position, world_normal, z);
}

// Radiance of directional light `i`, exposed as Bevy's lighting does it.
fn directional_radiance(i: u32) -> vec3<f32> {
    return lights.directional_lights[i].color.rgb * view.exposure;
}
