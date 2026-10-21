//! CC5's camera and lights in the editor, for comparing renders with CC5's
//! (`remotemedia-sdk/scripts/avatars/cc5_editor/skin/cc5_skin_capture.py` writes
//! `scene.json`: camera per view, lights, ambient colour).
//!
//! `--cc5-scene scene.json` makes snapshot views `cc5:<view>` (e.g. `cc5:face`)
//! put the camera where CC5's was: CC5's angle of view is horizontal over a
//! 36x24 mm film gate, which its render stretches to the viewport's size; render
//! ours at width x width/1.5 (e.g. `--size 1651x1101`) and resize to compare.
//! `--cc5-lights` also replaces the studio lights with CC5's, and the
//! environment map with CC5's IBL (`--cc5-ibl file.hdr`, default CC5's
//! `Image/Default_IBL.hdr`). Calibration knobs (env vars): CC5_LUMENS (lm per
//! light multiplier), CC5_AMBIENT, CC5_IBL_INTENSITY, CC5_IBL_ROT (degrees).
//! CC5 is Z up in cm, facing −Y; glTF is Y up in m, facing +Z.
use bevy::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// CC5 light multiplier → Bevy spot light intensity (lm); calibrated against
/// CC5's renders of the same scene.
pub const LUMENS_PER_MULTIPLIER: f32 = 4000.0;
/// CC5 ambient colour → Bevy ambient brightness.
pub const AMBIENT_BRIGHTNESS: f32 = 400.0;
/// CC5's IBL → Bevy environment map intensity (cd/m² per unit radiance).
pub const IBL_INTENSITY: f32 = 400.0;
pub const DEFAULT_IBL: &str = r"C:\Program Files\Reallusion\Character Creator 5\Image\Default_IBL.hdr";

fn knob(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub struct Cc5View {
    pub transform: Transform,
    /// horizontal angle of view (radians)
    pub hfov: f32,
}

pub struct Cc5Light {
    pub name: String,
    pub transform: Transform,
    pub color: Color,
    pub multiplier: f32,
    pub range: f32,
    pub shadows: bool,
    /// outer cone half-angle (radians) and inner/outer ratio
    pub angle: f32,
    pub falloff: f32,
    pub radius: f32,
}

#[derive(Resource)]
pub struct Cc5Scene {
    pub views: Vec<(String, Cc5View)>,
    pub lights: Vec<Cc5Light>,
    pub ambient: Color,
    /// `--cc5-lights`
    pub use_lights: bool,
    /// `--cc5-ibl`
    pub ibl: PathBuf,
}

/// CC5 (Z up, cm) → editor (Y up, m).
fn point(v: &Value) -> Option<Vec3> {
    let a = v.as_array()?;
    let f = |i: usize| a.get(i).and_then(Value::as_f64).map(|x| x as f32);
    Some(Vec3::new(f(0)?, f(2)?, -f(1)?) / 100.0)
}

fn rotation(v: &Value) -> Option<Quat> {
    let a = v.as_array()?;
    let f = |i: usize| a.get(i).and_then(Value::as_f64).map(|x| x as f32);
    let r = Quat::from_xyzw(f(0)?, f(1)?, f(2)?, f(3)?);
    // the basis change: world_ours = M world_cc, local axes alike (look −Z, up +Y)
    let m = Mat3::from_cols(Vec3::X, Vec3::NEG_Z, Vec3::Y);
    Some(Quat::from_mat3(&(m * Mat3::from_quat(r))).normalize())
}

fn transform(xf: &Value) -> Option<Transform> {
    Some(Transform { translation: point(&xf["t"])?, rotation: rotation(&xf["r"])?, scale: Vec3::ONE })
}

impl Cc5Scene {
    pub fn load(path: &Path, use_lights: bool, ibl: Option<PathBuf>) -> anyhow::Result<Self> {
        let s: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let mut views = Vec::new();
        if let Some(o) = s["views"].as_object() {
            for (name, v) in o {
                let (Some(t), Some(aov)) = (transform(&v["xf"]), v["angle_of_view"].as_f64()) else { continue };
                views.push((name.clone(), Cc5View { transform: t, hfov: (aov as f32).to_radians() }));
            }
        }
        let mut lights: Vec<Cc5Light> = Vec::new();
        for l in s["lights"].as_array().into_iter().flatten() {
            let name = l["name"].as_str().unwrap_or_default().to_string();
            // FindObjects lists spot lights under "Light" too
            if lights.iter().any(|x| x.name == name) || l["GetActive"].as_bool() == Some(false) {
                continue;
            }
            let Some(t) = transform(&l["xf"]) else { continue };
            let c = |i: usize| l["GetColor"][i].as_f64().unwrap_or(1.0) as f32;
            let beam = l["beam"].as_array();
            let b = |i: usize, d: f32| beam.and_then(|a| a.get(i)).and_then(Value::as_f64).map_or(d, |x| x as f32);
            lights.push(Cc5Light {
                name,
                transform: t,
                color: Color::srgb(c(0), c(1), c(2)),
                multiplier: l["GetMultiplier"].as_f64().unwrap_or(1.0) as f32,
                range: l["GetRange"].as_f64().unwrap_or(1000.0) as f32 / 100.0,
                shadows: l["IsCastShadow"].as_bool().unwrap_or(true),
                // CC5's beam angle is the full cone (degrees), falloff a percentage of it
                angle: (b(0, 60.0) * 0.5).to_radians(),
                falloff: (b(1, 30.0) / 100.0).clamp(0.0, 1.0),
                radius: l["GetTubeRadius"].as_f64().unwrap_or(5.0) as f32 / 100.0,
            });
        }
        let a = |i: usize| s["ambient"][i].as_f64().unwrap_or(0.0) as f32;
        let ibl = ibl.unwrap_or_else(|| PathBuf::from(DEFAULT_IBL));
        Ok(Self { views, lights, ambient: Color::linear_rgb(a(0), a(1), a(2)), use_lights, ibl })
    }

    pub fn view(&self, name: &str) -> Option<&Cc5View> {
        self.views.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

/// Startup (after `setup`): CC5's lights instead of the studio rig.
pub fn cc5_lights(
    mut commands: Commands,
    scene: Res<Cc5Scene>,
    studio: Query<Entity, With<DirectionalLight>>,
    cams: Query<Entity, With<Camera3d>>,
    mut images: ResMut<Assets<Image>>,
) {
    if !scene.use_lights {
        return;
    }
    let lumens = knob("CC5_LUMENS", LUMENS_PER_MULTIPLIER);
    match crate::ibl::environment_maps(&scene.ibl, knob("CC5_IBL_ROT", 0.0).to_radians(), &mut images) {
        Ok((diffuse, specular)) => {
            for c in &cams {
                commands.entity(c).insert(EnvironmentMapLight {
                    diffuse_map: diffuse.clone(),
                    specular_map: specular.clone(),
                    intensity: knob("CC5_IBL_INTENSITY", IBL_INTENSITY),
                    rotation: Quat::IDENTITY,
                });
            }
        }
        Err(e) => error!("cc5 ibl {}: {e:#}", scene.ibl.display()),
    }
    for e in &studio {
        commands.entity(e).despawn_recursive();
    }
    for l in &scene.lights {
        commands.spawn((
            SpotLight {
                color: l.color,
                intensity: l.multiplier * lumens,
                range: l.range,
                radius: l.radius,
                shadows_enabled: l.shadows,
                outer_angle: l.angle,
                inner_angle: l.angle * (1.0 - l.falloff),
                soft_shadows_enabled: true,
                ..default()
            },
            l.transform,
            Name::new(format!("CC5 {}", l.name)),
        ));
    }
    commands.insert_resource(AmbientLight { color: scene.ambient, brightness: knob("CC5_AMBIENT", AMBIENT_BRIGHTNESS) });
    info!("cc5 scene: {} lights", scene.lights.len());
}
