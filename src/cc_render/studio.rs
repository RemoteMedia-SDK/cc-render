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

/// The levels of the studio look and rig: exposure, each light's strength
/// (a fraction of `lux::AMBIENT_DAYLIGHT`), the key's position, the IBL and
/// ambient fill, and saturation. [`Self::STUDIO`] is the editor's look,
/// calibrated against CC5's viewport; [`Self::DAYLIGHT`] is bright, soft
/// outdoor light (a high sun, a broad sky fill, little shadow under the brows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StudioLevels {
    /// Camera exposure (lower = brighter).
    pub ev100: f32,
    pub key: f32,
    /// Key light position (it aims at the face); the character faces +Z.
    pub key_pos: Vec3,
    /// Key light colour (sRGB); warmer for sunlight.
    pub key_color: Vec3,
    pub fill: f32,
    pub back: f32,
    pub top: f32,
    /// Environment map (IBL) intensity.
    pub ibl: f32,
    /// `AmbientLight` brightness and colour (sRGB).
    pub ambient: f32,
    pub ambient_color: Vec3,
    /// Post-tonemap saturation.
    pub saturation: f32,
}

impl StudioLevels {
    pub const STUDIO: Self = Self {
        ev100: DEFAULT_EV100,
        key: 0.042,
        key_pos: Vec3::new(2.2, 2.5, 1.7),
        key_color: Vec3::new(1.00, 0.95, 0.88),
        fill: 0.004,
        back: 0.020,
        top: 0.006,
        ibl: 1.0,
        ambient: 20.0,
        ambient_color: Vec3::new(0.86, 0.90, 1.00),
        saturation: 1.3,
    };
    // Matched by eye to outdoor game screenshots of Valen (a golden tan): a
    // warm sun high on her right, a warm sky fill strong enough that the
    // brows cast little shadow, half a stop brighter than the studio, and
    // most of the warmth from the light's colour (saturation alone turns the
    // skin's reds orange-red rather than golden).
    pub const DAYLIGHT: Self = Self {
        ev100: 6.7,
        key: 0.08,
        key_pos: Vec3::new(-1.2, 5.0, 1.4),
        key_color: Vec3::new(1.00, 0.74, 0.50),
        fill: 0.018,
        back: 0.020,
        top: 0.03,
        ibl: 1.5,
        ambient: 55.0,
        ambient_color: Vec3::new(1.00, 0.86, 0.70),
        saturation: 1.25,
    };

    /// `CC_AVATAR_LIGHT`: a preset (`studio`, the default, or `daylight`)
    /// and/or `name=value` overrides, comma-separated, e.g.
    /// `daylight,fill=0.03,ev=7` (names: ev, key, keyx, keyy, keyz,
    /// keyr, keyg, keyb, fill, back, top, ibl, ambient, ambr, ambg, ambb, sat).
    pub fn from_env() -> Self {
        let Ok(spec) = std::env::var("CC_AVATAR_LIGHT") else { return Self::STUDIO };
        let mut l = Self::STUDIO;
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let Some((name, value)) = part.split_once('=') else {
                match part {
                    "studio" => l = Self::STUDIO,
                    "daylight" => l = Self::DAYLIGHT,
                    _ => warn!("CC_AVATAR_LIGHT: unknown preset {part:?}"),
                }
                continue;
            };
            let Ok(v) = value.trim().parse::<f32>() else {
                warn!("CC_AVATAR_LIGHT: {part:?} is not a number");
                continue;
            };
            match name.trim() {
                "ev" => l.ev100 = v,
                "key" => l.key = v,
                "keyx" => l.key_pos.x = v,
                "keyy" => l.key_pos.y = v,
                "keyz" => l.key_pos.z = v,
                "keyr" => l.key_color.x = v,
                "keyg" => l.key_color.y = v,
                "keyb" => l.key_color.z = v,
                "ambr" => l.ambient_color.x = v,
                "ambg" => l.ambient_color.y = v,
                "ambb" => l.ambient_color.z = v,
                "fill" => l.fill = v,
                "back" => l.back = v,
                "top" => l.top = v,
                "ibl" => l.ibl = v,
                "ambient" => l.ambient = v,
                "sat" => l.saturation = v,
                other => warn!("CC_AVATAR_LIGHT: unknown level {other:?}"),
            }
        }
        l
    }
}

/// The studio camera's look without camera, backdrop or lens: anti-aliasing,
/// shadow filtering, SSAO, tonemapping, exposure, bloom, sharpening, grading
/// (for a camera of one's own, e.g. the runtime renderer's).
pub fn studio_look(ev100: f32) -> impl Bundle {
    studio_look_with(&StudioLevels { ev100, ..StudioLevels::STUDIO })
}

/// [`studio_look`] at the given levels (exposure, saturation).
pub fn studio_look_with(levels: &StudioLevels) -> impl Bundle {
    let ev100 = levels.ev100;
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
        ColorGrading { global: ColorGradingGlobal { post_saturation: levels.saturation, ..default() }, ..default() },
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
    spawn_studio_lights_with(
        commands,
        &StudioLevels::STUDIO,
        CascadeShadowConfigBuilder { num_cascades: 3, maximum_distance: 8.0, first_cascade_far_bound: 0.8, ..default() },
    );
}

/// The rig for a fixed close-up (the runtime renderer's camera stays within
/// ~3 m of the face), at the given levels: one shadow cascade per light
/// instead of three. Each cascade re-renders the skinned, morphed body, so
/// this cuts the shadow passes from 12 to 4 at the same look.
pub fn spawn_studio_lights_closeup(commands: &mut Commands, levels: &StudioLevels) {
    spawn_studio_lights_with(
        commands,
        levels,
        CascadeShadowConfigBuilder { num_cascades: 1, maximum_distance: 3.0, ..default() },
    );
}

fn spawn_studio_lights_with(commands: &mut Commands, l: &StudioLevels, cascades: CascadeShadowConfigBuilder) {
    let aim = Vec3::new(0.0, 1.55, 0.0);
    // CC5-like studio light: a soft key from the front right (the character's left)
    // with soft (PCSS) shadows like a softbox, a weak fill, a back kicker and a top
    // light. Every light casts shadows: unshadowed ones light cavities (nostrils,
    // mouth corners) from inside.
    for (color, lux_frac, pos, soft) in [
        (Color::srgb(l.key_color.x, l.key_color.y, l.key_color.z), l.key, l.key_pos, Some(KEY_SOFT_SHADOW)), // key
        (Color::srgb(0.95, 0.95, 1.00), l.fill, Vec3::new(-1.0, 2.2, 1.2), None), // fill, above eye level
        // back kicker from the character's right: rims the hair and shines through the ear
        (Color::srgb(1.00, 0.92, 0.82), l.back, Vec3::new(-1.6, 2.2, -1.4), None),
        (Color::srgb(1.00, 0.95, 0.90), l.top, Vec3::new(1.5, 3.0, 0.3), None), // top fill
    ] {
        if lux_frac <= 0.0 {
            continue;
        }
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
            cascades.build(),
        ));
    }
    // cool, soft studio fill (CC5's grey-blue backdrop): shadows stay neutral, not orange
    let a = l.ambient_color;
    commands.insert_resource(AmbientLight { color: Color::srgb(a.x, a.y, a.z), brightness: l.ambient });
}
