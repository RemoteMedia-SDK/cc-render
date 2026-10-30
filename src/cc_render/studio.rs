//! The avatar editor's studio look: backdrop, camera (exposure, grading, TAA,
//! SSAO, bloom, sharpening, CC5's lens), the pisa IBL and the portrait light rig,
//! calibrated against CC5's viewport. Used by the avatar editor, by
//! `studio_snapshot` (any GLB) and by the runtime renderer with
//! `RendererConfig::studio_lighting`.

use bevy::core_pipeline::experimental::taa::TemporalAntiAliasing;
use bevy::core_pipeline::{bloom::Bloom, tonemapping::Tonemapping, Skybox};
use bevy::pbr::{light_consts::lux, CascadeShadowConfigBuilder, ScreenSpaceAmbientOcclusion, ShadowFilteringMethod};
use bevy::prelude::*;
use bevy::render::camera::Exposure;
use bevy::render::view::{ColorGrading, ColorGradingGlobal};
use std::path::Path;

/// Default camera exposure: Bevy's default (EV100 9.7, daylight) leaves the
/// portrait rig (a few hundred lux) far too dark.
pub const DEFAULT_EV100: f32 = 7.25;

/// Size of the key light for its soft (PCSS) shadows.
const KEY_SOFT_SHADOW: f32 = 6.0;

/// Vertical field of view: CC5's Preview Camera, an 80 mm lens on its 36 x 24 mm
/// film gate (2·atan(12 / 80); 25.4 degrees across the gate's width), so faces
/// have CC5's perspective. Bevy's default 45 degrees distorts close-ups of the
/// face like a wide angle.
pub const FOV_DEG: f32 = 17.06;

/// Camera distances below are framed for 45 degrees; scale them to FOV_DEG.
pub fn fov_scale() -> f32 {
    (22.5f32.to_radians()).tan() / (FOV_DEG.to_radians() * 0.5).tan()
}

/// Studio backdrop like CC5's viewport: soft grey-blue, darkening above eye level
/// (at the editor's 25 degree lens the top of a level close-up is only ~12 degrees
/// up). A small sRGB cubemap for `Skybox`: it stays out of the lighting and of
/// mesh picking.
pub fn studio_backdrop(images: &mut Assets<Image>) -> Handle<Image> {
    use bevy::render::render_asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
    const N: usize = 64;
    let base = Vec3::new(84.0, 90.0, 104.0);
    let mut data = Vec::with_capacity(N * N * 6 * 4);
    for face in 0..6 {
        for y in 0..N {
            for x in 0..N {
                let s = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
                let t = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
                // cube face order +X -X +Y -Y +Z -Z, t pointing down the face
                let dir = match face {
                    0 => Vec3::new(1.0, -t, -s),
                    1 => Vec3::new(-1.0, -t, s),
                    2 => Vec3::new(s, 1.0, t),
                    3 => Vec3::new(s, -1.0, -t),
                    4 => Vec3::new(s, -t, 1.0),
                    _ => Vec3::new(-s, -t, -1.0),
                }
                .normalize();
                let top = ((dir.y + 0.02) / 0.26).clamp(0.0, 1.0);
                let c = base * (1.0 - 0.5 * top * top * (3.0 - 2.0 * top));
                data.extend_from_slice(&[c.x as u8, c.y as u8, c.z as u8, 255]);
            }
        }
    }
    let mut image = Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 6 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    images.add(image)
}

/// Backdrop luminance (cd/m²) for a texel of 1.0, such that the backdrop shows
/// the same at any exposure (it is a backdrop, not part of the lit scene).
pub fn backdrop_brightness(ev100: f32) -> f32 {
    // Bevy's exposure: 1 / (1.2 * 2^ev100); 0.55 = calibrated against CC5's viewport.
    0.4 * 1.2 * 2f32.powf(ev100)
}

/// The studio camera's components, without its transform.
// Like cc-render's runtime camera (bevy_app/camera.rs): HDR + AgX + bloom + 4x
// MSAA and the pisa environment map; the backdrop, studio lights, exposure and
// grading are calibrated against CC5's viewport (its "Beautiful" close-up).
pub fn studio_camera(images: &mut Assets<Image>, ev100: f32) -> impl Bundle {
    (
        Camera3d::default(),
        Camera { hdr: true, clear_color: ClearColorConfig::Custom(Color::srgb(0.20, 0.22, 0.26)), ..default() },
        Skybox { image: studio_backdrop(images), brightness: backdrop_brightness(ev100), ..default() },
        studio_look(ev100),
        // 5 mm near plane: close-ups of the face/eyes must not clip away.
        Projection::Perspective(PerspectiveProjection { fov: FOV_DEG.to_radians(), near: 0.005, ..default() }),
    )
}

/// The studio camera's look without camera, backdrop or lens: anti-aliasing,
/// shadow filtering, SSAO, tonemapping, exposure, bloom, sharpening, grading
/// (for a camera of one's own, e.g. the runtime renderer's).
pub fn studio_look(ev100: f32) -> impl Bundle {
    (
        // TAA instead of MSAA: it resolves the CC5 hair's dithered strands into a soft
        // mass (hair_types.wgsl) and the soft shadows' noise (temporal filtering).
        Msaa::Off,
        TemporalAntiAliasing::default(),
        ShadowFilteringMethod::Temporal,
        // contact / inter-layer occlusion of ambient light (hair layers, creases)
        ScreenSpaceAmbientOcclusion { constant_object_thickness: 0.06, ..default() },
        Tonemapping::AgX,
        Exposure { ev100 },
        Bloom { intensity: 0.04, low_frequency_boost: 0.5, high_pass_frequency: 0.95, ..default() },
        // TAA softens skin pores below CC5's crisp viewport: sharpen back to its detail
        // (CC5_CAS overrides; calibrated on the face close-up)
        bevy::core_pipeline::contrast_adaptive_sharpening::ContrastAdaptiveSharpening {
            enabled: true,
            sharpening_strength: 0.5,
            denoise: false,
        },
        // AgX desaturates skin and hair against CC5's viewport: restore some saturation
        ColorGrading { global: ColorGradingGlobal { post_saturation: 1.3, ..default() }, ..default() },
    )
}

/// The pisa environment map from `env_dir`, if it is there.
pub fn studio_ibl(assets: &AssetServer, env_dir: &Path) -> Option<EnvironmentMapLight> {
    if !env_dir.join("pisa_diffuse_rgb9e5_zstd.ktx2").is_file() {
        warn!("no environment map in {} (set CC_AVATAR_ENVMAP_DIR); lighting will be flat", env_dir.display());
        return None;
    }
    let path = |f: &str| bevy::asset::AssetPath::from_path(&env_dir.join(f)).into_owned();
    Some(EnvironmentMapLight {
        diffuse_map: assets.load(path("pisa_diffuse_rgb9e5_zstd.ktx2")),
        specular_map: assets.load(path("pisa_specular_rgb9e5_zstd.ktx2")),
        intensity: 1.0,
        rotation: Quat::IDENTITY,
    })
}

/// The portrait light rig and the ambient fill.
pub fn spawn_studio_lights(commands: &mut Commands) {
    let aim = Vec3::new(0.0, 1.55, 0.0);
    // CC5-like studio light: a soft key from the front right (the character's left)
    // with soft (PCSS) shadows like a softbox, a weak fill, a back kicker and a top
    // light. Every light casts shadows: unshadowed ones light cavities (nostrils,
    // mouth corners) from inside.
    for (color, lux_frac, pos, soft) in [
        (Color::srgb(1.00, 0.95, 0.88), 0.042, Vec3::new(2.2, 2.5, 1.7), Some(KEY_SOFT_SHADOW)), // key
        (Color::srgb(0.95, 0.95, 1.00), 0.004, Vec3::new(-1.0, 2.2, 1.2), None), // fill, above eye level
        // back kicker from the character's right: rims the hair and shines through the ear
        (Color::srgb(1.00, 0.92, 0.82), 0.020, Vec3::new(-1.6, 2.2, -1.4), None),
        (Color::srgb(1.00, 0.95, 0.90), 0.006, Vec3::new(1.5, 3.0, 0.3), None), // top fill
    ] {
        commands.spawn((
            DirectionalLight {
                color,
                illuminance: lux::AMBIENT_DAYLIGHT * lux_frac,
                shadows_enabled: true,
                shadow_depth_bias: 0.02,
                shadow_normal_bias: 0.4,
                soft_shadow_size: soft,
                ..default()
            },
            Transform::from_translation(pos).looking_at(aim, Vec3::Y),
            CascadeShadowConfigBuilder { num_cascades: 3, maximum_distance: 8.0, first_cascade_far_bound: 0.8, ..default() }
                .build(),
        ));
    }
    // cool, soft studio fill (CC5's grey-blue backdrop): shadows stay neutral, not orange
    commands.insert_resource(AmbientLight { color: Color::srgb(0.86, 0.90, 1.00), brightness: 20.0 });
}
