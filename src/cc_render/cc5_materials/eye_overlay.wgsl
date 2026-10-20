// CC5 RLEyeOcclusion(_Plus) / RLEyeTearline(_Plus) approximations: transparent
// meshes over the eye, drawn with premultiplied alpha (see mod.rs). Forward only.
//
// Eye occlusion: a mesh spanning the eye opening, UV u from the outer (0) to the
// inner (1) corner, v from the upper (0) to the lower (1) lid. CC5 shades it
// procedurally:
//   - shadow: bands fading in from the top / bottom / inner / outer edges
//     (each "Range" = where the band ends, "Contrast" its falloff, "Edge Fadeout
//     Contrast" how it fades towards the corners), darkening towards Shadow Color
//     by Shadow Strength: the lids' contact shadow on the eyeball;
//   - blur: CC5 blurs the eye under the same kind of bands; here a lit tint of
//     Blur Color (soft pink, wet flesh) by Blur Strength, strongest at the tear duct.
// Tearline: a thin strip along the lower lid: a nearly clear wet film (_Opacity)
// with a sharp water reflection (_Roughness), faded at both ends (Edge Fadeout).
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}
#import bevy_pbr::mesh_view_bindings::{lights, view}
#import cc5::common::{luminance, directional_radiance}

struct Cc5EyeOverlay {
    // x: 0 occlusion, 1 tearline; y: opacity for blink
    kind: vec4<f32>,
    // rgb: shadow colour, w: shadow strength
    shadow_color: vec4<f32>,
    // bands: x, y: range, z: contrast, w: edge fadeout contrast
    shadow_top: vec4<f32>,
    shadow_bottom: vec4<f32>,
    shadow_inner: vec4<f32>,
    shadow_outer: vec4<f32>,
    // rgb: blur colour, w: blur strength
    blur_color: vec4<f32>,
    blur_top: vec4<f32>,
    blur_bottom: vec4<f32>,
    blur_inner: vec4<f32>,
    blur_outer: vec4<f32>,
    // x: roughness, y: opacity, z: edge fadeout, w: detail amount
    tear: vec4<f32>,
}

@group(2) @binding(100) var<uniform> overlay: Cc5EyeOverlay;

// Brightness of the tearline's softbox glints (water F0 0.02 x softbox gain).
const TEAR_GLINT: f32 = 0.4;

// A band fading in from an edge: `x` is the distance from that edge (0..1),
// `along` the position along the edge (0..1).
fn band(b: vec4<f32>, x: f32, along: f32) -> f32 {
    var v = 1.0 - smoothstep(b.x, max(b.y, b.x + 1e-3), x);
    // contrast: steeper falloff
    v = pow(v, 1.0 + 2.0 * b.z);
    // edge fadeout: weaker towards the ends of the edge
    let ends = smoothstep(0.0, 0.5, min(along, 1.0 - along) * 2.0);
    return v * mix(1.0, ends, b.w);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef VERTEX_UVS_A
    let uv = in.uv;
#else
    let uv = vec2(0.5);
#endif
    let blink = overlay.kind.y;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.diffuse_transmission = 0.0;
    pbr_input.material.emissive = vec4(0.0);
    pbr_input.diffuse_occlusion = vec3(1.0);
    pbr_input.specular_occlusion = 1.0;

    var out: FragmentOutput;
    if overlay.kind.x < 0.5 {
        // --- eye occlusion ---
        let u = uv.x;
        let v = uv.y;
        let shadow = max(
            max(band(overlay.shadow_top, v, u), band(overlay.shadow_bottom, 1.0 - v, u)),
            max(band(overlay.shadow_inner, 1.0 - u, v), band(overlay.shadow_outer, u, v))
        );
        let blur = max(
            max(band(overlay.blur_top, v, u), band(overlay.blur_bottom, 1.0 - v, u)),
            max(band(overlay.blur_inner, 1.0 - u, v), band(overlay.blur_outer, u, v))
        );
        // darkening towards the shadow colour: premultiplied black at that opacity
        let a_shadow = clamp(shadow * overlay.shadow_color.w * (1.0 - luminance(overlay.shadow_color.rgb)) * 2.0, 0.0, 1.0);
        // wet flesh tint, lit like the surrounding skin
        let a_blur = clamp(blur * overlay.blur_color.w, 0.0, 1.0);
        pbr_input.material.base_color = vec4(overlay.blur_color.rgb * vec3(0.62, 0.36, 0.32), 1.0);
        pbr_input.material.reflectance = 0.5;
        pbr_input.material.perceptual_roughness = 0.35;
        let tint = apply_pbr_lighting(pbr_input).rgb;
        let a = 1.0 - (1.0 - a_shadow) * (1.0 - a_blur);
        out.color = vec4(tint * a_blur * (1.0 - a_shadow), a) * blink;
    } else {
        // --- tearline ---
        let fade = smoothstep(0.0, 0.12 * max(overlay.tear.z, 0.1), min(uv.x, 1.0 - uv.x));
        pbr_input.material.base_color = vec4(0.0, 0.0, 0.0, 1.0);
        // water: F0 0.02 (Bevy: F0 = 0.16 reflectance^2)
        pbr_input.material.reflectance = 0.35;
        pbr_input.material.perceptual_roughness = clamp(overlay.tear.x * 0.5, 0.089, 1.0);
        var spec = apply_pbr_lighting(pbr_input).rgb;
        // glints: the lights reflected as softboxes in the curved wet film
        let R = reflect(-pbr_input.V, pbr_input.N);
        var brightest = vec3(0.0);
        for (var i = 0u; i < lights.n_directional_lights; i = i + 1u) {
            let c = dot(R, lights.directional_lights[i].direction_to_light);
            let radiance = directional_radiance(i);
            spec += radiance * smoothstep(0.985, 0.995, c) * TEAR_GLINT;
            if dot(radiance, vec3(1.0)) > dot(brightest, vec3(1.0)) { brightest = radiance; }
        }
        // the eye's camera-relative highlight (eye.wgsl) glints in the wet line too
        let view_dir = normalize((view.world_from_view * vec4(normalize(vec3(0.9, 0.45, 1.0)), 0.0)).xyz);
        spec += brightest * smoothstep(0.96, 0.985, dot(R, view_dir)) * TEAR_GLINT;
        let a = overlay.tear.y * fade;
        out.color = vec4(spec * fade, a) * blink;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
