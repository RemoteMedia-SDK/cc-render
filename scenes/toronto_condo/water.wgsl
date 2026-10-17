// Standard Bevy PBR lighting with integrated wind-phase normal-map coordinates.
// Weather changes do not reset cloud/wave phase. No avatar inputs are involved.
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_bindings::{normal_map_texture, normal_map_sampler},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#import "scene://sky_common.wgsl"::scene_sky

@group(2) @binding(100) var<uniform> conditions: vec4<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let wind_time = conditions.x;
    // World metres keep waves isotropic on the rectangular lake mesh. Two
    // rotated, differently scaled layers avoid a single visible tile grid.
    let position = in.world_position.xz;
    let rotated = vec2<f32>(0.8*position.x-0.6*position.y,
                           0.6*position.x+0.8*position.y);
    let broad = textureSample(normal_map_texture, normal_map_sampler,
        position/160.0 + wind_time*vec2<f32>(0.0018,0.0007)).xyz*2.0-1.0;
    let fine = textureSample(normal_map_texture, normal_map_sampler,
        rotated/93.0 + wind_time*vec2<f32>(-0.0011,0.0023)).xyz*2.0-1.0;
    let broad_slope = broad.xy/max(broad.z,0.1);
    let fine_slope = fine.xy/max(fine.z,0.1);
    // Transform the rotated layer's slope back into world X/Z before adding.
    let fine_world = vec2<f32>(0.8*fine_slope.x+0.6*fine_slope.y,
                              -0.6*fine_slope.x+0.8*fine_slope.y);
    // Keep high-rise ripples below a foil-like normal contrast; stronger wind
    // increases both slope strength here and drift speed above.
    let wave_strength = 0.4*sqrt(max(conditions.w,0.1));
    let slope = (broad_slope*0.65 + fine_world*0.35)*wave_strength;
    var input = pbr_input_from_standard_material(in,is_front);
    input.N = normalize(vec3<f32>(slope.x,1.0,slope.y));
    // Match the visible moving sky at grazing angles. This artistic Fresnel
    // blend has no city/shoreline reflection or screen-space dependency.
    let reflection = scene_sky(reflect(-input.V, input.N), conditions);
    let facing = clamp(dot(input.N, input.V), 0.0, 1.0);
    let fresnel = 0.02 + 0.98*pow(1.0-facing, 5.0);
    var shaded = apply_pbr_lighting(input);
    shaded = vec4<f32>(mix(shaded.rgb, reflection, fresnel), shaded.a);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(input,shaded);
    return out;
}
