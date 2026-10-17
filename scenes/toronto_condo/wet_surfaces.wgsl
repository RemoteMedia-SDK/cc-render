// Exposed paving only. The coating never changes geometry, friction or motion.
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#import "scene://sky_common.wgsl"::{scene_sky, noise}

@group(2) @binding(100) var<uniform> sky: vec4<f32>;
@group(2) @binding(101) var<uniform> coating: vec4<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var input = pbr_input_from_standard_material(in, is_front);
    let p = in.world_position.xyz;
    var exposure = smoothstep(0.6, 0.95, input.world_normal.y);
    if coating.y > 0.5 {
        // Wind-driven rain reaches only the outer 0.8 m and side edges beneath
        // the overhang. The threshold and covered strip remain completely dry.
        let edge = max(smoothstep(6.6, 7.35, p.z), smoothstep(5.25, 5.95, abs(p.x)));
        exposure *= edge * smoothstep(5.35, 5.7, p.z);
    }
    let variation = 0.65 + 0.35 * noise(p.xz * 3.2);
    let film = clamp(coating.x * exposure * variation, 0.0, 1.0);
    input.material.base_color = vec4<f32>(input.material.base_color.rgb * mix(1.0, 0.58, film), input.material.base_color.a);
    // Asphalt remains granular beneath a thin film. Avoid turning distant
    // roads into broad, pale mirrors at the condo's grazing viewing angle.
    let wet_roughness = mix(0.34, 0.22, coating.y);
    input.material.perceptual_roughness = mix(input.material.perceptual_roughness, wet_roughness, film);
    input.material.reflectance = mix(input.material.reflectance, 0.35, film);
    var color = apply_pbr_lighting(input);
    // Low-contrast sky sheen; this does not reflect buildings or the avatar.
    let facing = clamp(dot(input.N, input.V), 0.0, 1.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - facing, 5.0);
    let reflection = scene_sky(reflect(-input.V, input.N), sky);
    let sky_visibility = mix(0.18, 0.35, coating.y);
    color = vec4<f32>(mix(color.rgb, reflection, fresnel * film * sky_visibility), color.a);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(input, color);
    return out;
}
