// Scenic occupied windows aligned to the facade's existing 3 m window cells.
// Emission does not introduce point lights or modify avatar lighting/motion.
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

@group(2) @binding(100) var<uniform> panes: vec4<f32>;

fn window_random(key: vec4<i32>) -> f32 {
    let bits = bitcast<vec4<u32>>(key);
    var h = bits.x*747796405u ^ bits.y*2891336453u ^ bits.z*277803737u ^ bits.w*1597334677u;
    h = (h ^ (h >> 16u))*2246822519u;
    return f32((h ^ (h >> 13u)) & 16777215u)/16777215.0;
}

fn pane_radiance(cell: vec2<i32>, origin: vec2<i32>, height: f32) -> vec3<f32> {
    let key = vec4<i32>(cell,origin+vec2<i32>(i32(panes.w)*17,731));
    let occupied = step(0.62,window_random(key));
    let variation = window_random(key+vec4<i32>(19,37,5,11));
    let tint = mix(vec3<f32>(1.0,0.62,0.28),vec3<f32>(0.66,0.79,1.0),step(0.76,variation));
    let shade = 0.65+0.35*smoothstep(0.05,0.85,height);
    return tint*(0.45+variation)*occupied*shade;
}

fn shade_integral(height: f32) -> f32 {
    let t = clamp((height-0.05)/0.8,0.0,1.0);
    return 0.8*(t*t*t-0.5*t*t*t*t)+max(height-0.85,0.0);
}

fn filtered_windows(cells: vec2<f32>, footprint: vec2<f32>, origin: vec2<i32>) -> vec3<f32> {
    // Integrate the actual neighbouring panes over the pixel footprint, rather
    // than antialiasing the pane edges while point-sampling random occupancy.
    let width = clamp(footprint,vec2<f32>(0.002),vec2<f32>(2.0));
    let low = cells-width*0.5;
    let high = cells+width*0.5;
    let base = vec2<i32>(floor(cells));
    let inset = panes.xy+vec2<f32>(0.025);
    var filtered = vec3<f32>(0.0);
    for (var y=-1; y<=1; y=y+1) {
        for (var x=-1; x<=1; x=x+1) {
            let cell = base+vec2<i32>(x,y);
            let start = max(low,vec2<f32>(cell)+inset);
            let end = min(high,vec2<f32>(cell)+vec2<f32>(0.97));
            let overlap = max(end-start,vec2<f32>(0.0));
            let weight = overlap.x*overlap.y/(width.x*width.y);
            if weight>0.0 {
                filtered += pane_radiance(cell,origin,(start.y+end.y)*0.5-f32(cell.y))*weight;
            }
        }
    }
    // Once individual cells are subpixel, converge to their expected radiance.
    // The tint coefficients integrate (0.45+variation) over each colour range.
    let opening = vec2<f32>(0.97)-inset;
    let mean_shade = 0.65+0.35*(shade_integral(0.97)-shade_integral(inset.y))/opening.y;
    let mean_tint = vec3<f32>(1.0,0.62,0.28)*0.6308+vec3<f32>(0.66,0.79,1.0)*0.3192;
    let average = mean_tint*0.38*opening.x*opening.y*mean_shade;
    let unresolved = smoothstep(1.25,2.0,max(footprint.x,footprint.y));
    return mix(filtered,average,unresolved);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var input = pbr_input_from_standard_material(in,is_front);
    let cells = in.uv*4.0;
    // Reconstruct over two pixel widths, matching the former pane-edge
    // smoothing while also filtering the discontinuous occupancy pattern.
    let footprint = fwidth(cells)*2.0;
    // Recover a horizontal face origin, constant across each wall. Metre
    // quantization keeps small interpolation errors out of the integer seed
    // without making adjacent facets of curved towers share broad light bands.
    let horizontal = normalize(cross(vec3<f32>(0.0,1.0,0.0),in.world_normal)).xz;
    let origin = vec2<i32>(floor(in.world_position.xz-horizontal*in.uv.x*12.0));
    let distance = length(in.world_position.xyz-view.world_position);
    let fade = 1.0-smoothstep(1800.0,4500.0,distance);
    input.material.emissive = vec4<f32>(filtered_windows(cells,footprint,origin)*panes.z*fade,1.0);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(input,apply_pbr_lighting(input));
    return out;
}
