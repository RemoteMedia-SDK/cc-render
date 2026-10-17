#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
#import "scene://sky_common.wgsl"::scene_sky

struct Sky { conditions: vec4<f32> }
@group(2) @binding(0) var<uniform> sky: Sky;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(scene_sky(in.world_position.xyz-view.world_position.xyz, sky.conditions), 1.0);
}
