// Shared display-linear sky approximation for the dome and lake reflection.
// Continuous domain-warped noise avoids lattice corners in thin cloud edges.
#import bevy_pbr::{mesh_view_bindings::fog, mesh_view_types::FOG_MODE_LINEAR}

fn noise(p: vec2<f32>) -> f32 {
    let warped = p + vec2<f32>(sin(p.y*0.73),sin(p.x*0.61))*0.65;
    return 0.5 + 0.25*sin(warped.x*1.31+warped.y*0.79)
               + 0.25*sin(warped.y*1.57-warped.x*0.53);
}

fn cloud_noise(point: vec2<f32>) -> f32 {
    var p = point;
    var value = 0.0;
    var amplitude = 0.5;
    for (var octave=0; octave<5; octave=octave+1) {
        value += noise(p)*amplitude;
        p = p*2.03+vec2<f32>(7.1,3.9);
        amplitude *= 0.5;
    }
    return value;
}

fn scene_sky(direction_input: vec3<f32>, conditions: vec4<f32>) -> vec3<f32> {
    let direction = normalize(direction_input);
    let elevation = clamp(direction.y,0.0,1.0);
    let cover = conditions.y;
    let night = conditions.z;
    let clear = mix(vec3<f32>(0.62,0.75,0.85),vec3<f32>(0.07,0.23,0.53),pow(elevation,0.45));
    let overcast = smoothstep(0.55,0.95,cover);
    let day = mix(clear,vec3<f32>(0.31,0.36,0.40),overcast);
    var color = mix(day,vec3<f32>(0.005,0.009,0.025),night);
    let cloud_pos = direction.xz/max(direction.y+0.16,0.08)*2.6
                    + vec2<f32>(conditions.x*0.003,0.0);
    let field = cloud_noise(cloud_pos);
    let density = smoothstep(0.56-cover*0.20,0.85-cover*0.18,field);
    let horizon_fade = smoothstep(0.0,0.13,elevation);
    let lit_cloud = mix(mix(vec3<f32>(0.58,0.66,0.73),vec3<f32>(1.35,1.30,1.18),field),
        vec3<f32>(0.24+field*0.20),overcast);
    color = mix(color,mix(lit_cloud,vec3<f32>(0.018,0.025,0.045),night),density*horizon_fade);
    let sun_direction = normalize(vec3<f32>(-0.6,0.2,0.8));
    let sun = max(dot(direction,sun_direction),0.0);
    color += vec3<f32>(1.0,0.72,0.36)*pow(sun,32.0)*0.16*(1.0-night)*(1.0-density);
    // A shallow haze layer gives long sightlines at low elevations. Use the
    // camera's linear fog colour/range so the dome, distant geometry and lake
    // converge to one horizon colour, including during weather transitions.
    // This is an artistic height approximation, not volumetric scattering.
    if fog.mode == FOG_MODE_LINEAR {
        let haze_distance = 120.0 / max(direction.y, 0.002);
        let haze = clamp((haze_distance-fog.be.x)/max(fog.be.y-fog.be.x,1.0),0.0,1.0);
        color = mix(color, fog.base_color.rgb, haze*fog.base_color.a);
    }
    return color;
}
