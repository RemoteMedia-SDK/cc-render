// Strand hair as camera-facing ribbons, expanded on the GPU (HAIR_RIBBONS).
//
// A strand part stores one point per strand vertex (glTF LINES); the editor turns
// each point into two vertices holding the same centre position, with
//   TANGENT.xyz = the strand direction, TANGENT.w = the signed half-width
// (one side -w, the other +w). This shader skins the centre like Bevy's mesh
// vertex shader, then moves it sideways by w along cross(direction, to viewer),
// so strands keep their width from every angle and the shape (conform) only
// moves the points. NORMAL is the strand's lighting normal (out from the head),
// unchanged. Forward and prepass (depth, shadows, motion vectors) in one file:
// PREPASS_PIPELINE selects the prepass interface. Morph targets: none on hair.
#import bevy_pbr::{
    mesh_functions,
    skinning,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::VertexOutput
#else
#import bevy_pbr::forward_io::VertexOutput
#endif

// The vertex layouts of Bevy's forward_io / prepass_io, with the tangent always
// present (cc5_materials' specialize adds it to the prepass layout).
struct Vertex {
    @builtin(instance_index) instance_index: u32,
#ifdef PREPASS_PIPELINE
    @location(0) position: vec3<f32>,
#ifdef VERTEX_UVS_A
    @location(1) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(2) uv_b: vec2<f32>,
#endif
    @location(3) normal: vec3<f32>,
    @location(4) tangent: vec4<f32>,
#ifdef SKINNED
    @location(5) joint_indices: vec4<u32>,
    @location(6) joint_weights: vec4<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(7) color: vec4<f32>,
#endif
#else // PREPASS_PIPELINE
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
#ifdef VERTEX_UVS_A
    @location(2) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(3) uv_b: vec2<f32>,
#endif
    @location(4) tangent: vec4<f32>,
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
#ifdef SKINNED
    @location(6) joint_indices: vec4<u32>,
    @location(7) joint_weights: vec4<f32>,
#endif
#endif // PREPASS_PIPELINE
}

// Shadow maps: strands face the light too, so each would cast its full width;
// hair_prepass.wgsl's shadow density is calibrated for fixed ribbons, which
// average 2/pi of it.
const SHADOW_WIDTH: f32 = 0.6366;

// How much the width follows the strand's facing (|normal . view|, as a fixed
// ribbon's projected width does): 0 = always full width.
const FACING: f32 = 0.5;

// The sideways offset of a ribbon vertex: across the strand, facing the view
// (for orthographic views, e.g. directional shadow maps, their view axis).
fn ribbon_offset(centre: vec3<f32>, tangent: vec4<f32>, normal: vec3<f32>) -> vec3<f32> {
    let t = tangent.xyz;
    var to_view = view.world_position - centre;
    var width = tangent.w;
    if view.clip_from_view[3][3] == 1.0 {
        to_view = view.world_from_view[2].xyz;
#ifdef DEPTH_CLAMP_ORTHO
        width *= mix(SHADOW_WIDTH, 1.0, FACING);
#endif
    }
    var side = cross(t, to_view);
    if dot(side, side) < 1e-14 {
        // looking along the strand: any direction across it
        side = cross(t, select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), abs(t.y) > 0.9));
    }
    width *= mix(1.0, abs(dot(normalize(normal), normalize(to_view))), FACING);
    return normalize(side) * width;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

#ifdef SKINNED
    let world_from_local = skinning::skin_model(vertex.joint_indices, vertex.joint_weights);
#else
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
#endif
    let tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex.instance_index);
    let centre = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
#ifdef SKINNED
    let world_normal = skinning::skin_normals(world_from_local, vertex.normal);
#else
    let world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
#endif
    let offset = ribbon_offset(centre.xyz, tangent, world_normal);
    out.world_position = vec4<f32>(centre.xyz + offset, 1.0);
    out.position = position_world_to_clip(out.world_position.xyz);

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif

#ifdef PREPASS_PIPELINE
#ifdef DEPTH_CLAMP_ORTHO
    out.clip_position_unclamped = out.position;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = world_normal;
#ifdef VERTEX_TANGENTS
    out.world_tangent = tangent;
#endif
#endif
#ifdef MOTION_VECTOR_PREPASS
#ifdef SKINNED
#ifdef HAS_PREVIOUS_SKIN
    let prev_model = skinning::skin_prev_model(vertex.joint_indices, vertex.joint_weights);
#else
    let prev_model = mesh_functions::get_previous_world_from_local(vertex.instance_index);
#endif
#else
    let prev_model = mesh_functions::get_previous_world_from_local(vertex.instance_index);
#endif
    // the same sideways offset: motion of the strand, not of the view-facing turn
    out.previous_world_position = mesh_functions::mesh_position_local_to_world(prev_model, vec4<f32>(vertex.position, 1.0))
        + vec4<f32>(offset, 0.0);
#endif

#else // PREPASS_PIPELINE
    out.world_normal = world_normal;
#ifdef VERTEX_TANGENTS
    out.world_tangent = tangent;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
#endif
#endif // PREPASS_PIPELINE

    return out;
}
