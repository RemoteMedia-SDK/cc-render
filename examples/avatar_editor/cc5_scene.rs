//! CC5's camera and lights in the editor, for comparing renders with CC5's
//! (`remotemedia-sdk/scripts/avatars/cc5_editor/skin/cc5_skin_capture.py` writes
//! `scene.json`: camera per view, lights, ambient colour).
//!
//! `--cc5-scene scene.json` makes snapshot views `cc5:<view>` (e.g. `cc5:face`)
//! put the camera where CC5's was: CC5's angle of view is horizontal over a
//! 36x24 mm film gate, and its render fits the gate's height to the image with
//! square pixels; render ours at CC5's size (e.g. `--size 1651x961`) to compare
//! pixel for pixel.
//! `--cc5-lights` also replaces the studio lights with CC5's, and the
//! environment map with CC5's IBL (`--cc5-ibl file.hdr`, default CC5's
//! `Image/Default_IBL.hdr`). Calibration knobs (env vars): CC5_LUMENS (lm per
//! light multiplier), CC5_AMBIENT, CC5_IBL_INTENSITY, CC5_IBL_ROT (degrees).
//! CC5 is Z up in cm, facing −Y; glTF is Y up in m, facing +Z.
use bevy::prelude::*;
use cc_render_plugin::cc_render::cc5_materials::{Cc5Materials, Cc5SkinMaterial, SkinTune};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// CC5 light multiplier → Bevy spot light intensity (lm); calibrated against
/// CC5's renders of the same scene. Lights without inverse-square falloff get
/// `LUMENS_FLAT` per multiplier and m² of distance to the head (the same light
/// there as CC5's, which does not fade with distance).
pub const LUMENS_PER_MULTIPLIER: f32 = 1500.0;
pub const LUMENS_FLAT: f32 = 1000.0;
/// Grading with CC5's lights (ACES tonemapping): post saturation, exposure
/// (stops) and sharpening. Against CC5's renders of 2 skins and 7 SkinGen layers:
/// dE 39.8 summed (AgX + saturation 1.6: 52.5), error in the layers' and skins'
/// change from the base 12.1 (21.6), detail 1.01.
pub const SATURATION: f32 = 1.0;
/// Scalp hair's diffuse / specular gains with CC5's lights: against CC5's render
/// of HD Neutral F with the character's hair, hair regions' mean dE 11.8 -> 7.9
/// (crown L 5.5 -> 11.8, CC5 16.7; bangs within ~3 L).
pub const HAIR_GAIN: [f32; 2] = [2.0, 1.0];
/// Scalp hair's colour multiplier with CC5's lights: the light tint's inverse
/// (its magenta reads on neutral hair; CC5's hair stays ash).
pub const HAIR_TINT: [f32; 3] = [1.0, 1.0 / 0.94, 1.0 / 1.10];
/// Hair's occlusion towards the roots with CC5's lights (studio: 0.55, fitted
/// on CC5's viewport): its renders' hair is lighter at the crown. Fitted on the
/// two hair faces (hair_metric.py): hair dE 10.0 -> 9.55, dL -6.6 -> -5.2.
pub const HAIR_ROOT_OCCLUSION: f32 = 0.3;
pub const EXPOSURE: f32 = 0.9;
pub const SHARPENING: f32 = 0.15;
/// Per-light corrections by CC5 light name: fitted against CC5's renders of the
/// bare skin and three skins at CC5's framing under ACES (`calib_gains.py`;
/// mean patch dE 4.09 -> 3.76; was Top 2.5 alone, from the flat grey skin).
const LIGHT_GAINS: [(&str, f32); 4] = [("Key Light - Up", 1.15), ("Left_Rim", 1.3), ("Top", 1.6), ("Key Light - Down", 1.3)];
/// Where CC5's lights aim: the head (m, editor space).
const HEAD: Vec3 = Vec3::new(0.0, 1.6, 0.0);
/// CC5 ambient colour → Bevy ambient brightness.
pub const AMBIENT_BRIGHTNESS: f32 = 110.0;
/// The skin shader's extra wrap with CC5's lights (SkinTune extra_wrap). With
/// AMBIENT_BRIGHTNESS fitted at CC5's framing over the bare skin and three skins
/// (`calib_fill.py`): mean patch dE 5.04 -> 4.77.
pub const SKIN_WRAP: f32 = 0.09;
/// The skin's subsurface falloff colour desaturation with CC5's lights; with
/// SKIN_WRAP and AMBIENT_BRIGHTNESS refitted on the whole face over five faces
/// (`calib_batch.py`): CC5's shadows are greyer than our warm wrap.
pub const SKIN_SSS_DESAT: f32 = 0.41;
/// Soft radius (m) for CC5's rect lights (area lights; scene.json has no size):
/// fitted against CC5's renders, patch dE bare skin 4.24 -> 4.12, HD Neutral F
/// 3.88 -> 3.80, HD Aaron 3.29 -> 3.18 (hard shadows before).
pub const RECT_RADIUS: f32 = 0.25;
/// The soft shadows' minimum penumbra (Bevy's PCSS blur units; Bevy: 0.5). Bevy
/// sizes the penumbra physically from the blocker / receiver gap: a nose or an
/// eyelid shadowing the skin a centimetre away stays hard edged whatever the
/// light's radius (a lighting step down the philtrum, crescents under the
/// eyes), where CC5's shadows are soft. Fitted on five faces (batch_variants.py):
/// face dE 7.27 -> 7.10 (0.5 / 4 / 10 / 25 / 40 / 70: 7.27 / 7.23 / 7.14 / 7.10 /
/// 7.23 / 7.45). Knob CC5_SHADOW_PENUMBRA.
pub const SHADOW_PENUMBRA: f32 = 25.0;

/// Bevy's shadow sampling with the minimum penumbra [`SHADOW_PENUMBRA`].
fn soft_shadows(shaders: &mut Assets<Shader>) {
    let floor = knob("CC5_SHADOW_PENUMBRA", SHADOW_PENUMBRA);
    let src = include_str!("shadow_sampling.wgsl").replace("const PENUMBRA_FLOOR: f32 = 0.5;", &format!("const PENUMBRA_FLOOR: f32 = {floor:?};"));
    shaders.insert(&bevy::pbr::SHADOW_SAMPLING_HANDLE, Shader::from_wgsl(src, "examples/avatar_editor/shadow_sampling.wgsl"));
}

/// CC5's IBL → Bevy environment map intensity (cd/m² per unit radiance).
pub const IBL_INTENSITY: f32 = 5.0;
pub const DEFAULT_IBL: &str = r"C:\Program Files\Reallusion\Character Creator 5\Image\Default_IBL.hdr";

fn knob(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// CC5's renders are cooler than its light colours say (a global tint on every
/// light, the ambient and the IBL). Refitted under the ACES grading at CC5's
/// framing (was 1, 0.92, 1.15 from the flat grey skin under AgX): patch dE bare
/// skin 4.58 -> 4.23, HD Neutral F 4.68 -> 3.88, HD Aaron 4.02 -> 3.29.
pub const TINT: [f32; 3] = [1.0, 0.94, 1.10];

fn tint() -> Vec3 {
    std::env::var("CC5_TINT")
        .ok()
        .and_then(|v| {
            let p: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (p.len() == 3).then(|| Vec3::new(p[0], p[1], p[2]))
        })
        .unwrap_or(Vec3::from(TINT))
}

fn tinted(c: Color, t: Vec3) -> Color {
    let l = c.to_linear();
    Color::linear_rgb(l.red * t.x, l.green * t.y, l.blue * t.z)
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
    /// CC5 "Inverse Square" falloff (rect lights have none: the same light at any distance)
    pub inverse_square: bool,
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
                name: name.clone(),
                transform: t,
                color: Color::srgb(c(0), c(1), c(2)),
                multiplier: l["GetMultiplier"].as_f64().unwrap_or(1.0) as f32,
                range: l["GetRange"].as_f64().unwrap_or(1000.0) as f32 / 100.0,
                shadows: l["IsCastShadow"].as_bool().unwrap_or(true)
                    && knob("CC5_SHADOWS", 1.0) > 0.5
                    && !std::env::var("CC5_SHADOW_OFF").is_ok_and(|v| v.split(',').any(|n| n == name)),
                // CC5's beam angle is the full cone (degrees), falloff a percentage of it
                angle: (b(0, 60.0) * 0.5).to_radians(),
                falloff: (b(1, 30.0) / 100.0).clamp(0.0, 1.0),
                // CC5's tube lights are soft (radius and soft radius, cm): Bevy's light
                // radius sets the soft shadows' penumbra
                radius: if l["IsRectangleShape"].as_bool() == Some(true) {
                    // CC5's rect lights are area lights, their size not in scene.json:
                    // a soft radius (m) fitted against CC5's renders
                    knob("CC5_RECT_RADIUS", RECT_RADIUS)
                } else {
                    l["GetTubeRadius"].as_f64().unwrap_or(0.0).max(l["GetTubeSoftRadius"].as_f64().unwrap_or(0.0)) as f32 / 100.0
                        * knob("CC5_SOFT", 1.0)
                },
                inverse_square: l["GetInverseSquare"].as_bool().unwrap_or(true),
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

/// The CC5 mode's calibration settings, changeable while running (snapshot jobs'
/// `"cc5"`: one editor launch renders a whole sweep).
#[derive(Clone, Debug, PartialEq)]
pub struct Cc5Settings {
    pub tint: Vec3,
    pub ambient: f32,
    pub ibl: f32,
    pub exposure: f32,
    pub sat: f32,
    /// per light of [`Cc5Scene::lights`]
    pub gains: Vec<f32>,
    pub skin_tune: SkinTune,
}

impl Cc5Settings {
    /// These settings with `v`'s overrides: {"tint": [r, g, b], "ambient", "ibl",
    /// "exposure", "sat", "gains": {"<light>": gain}, "skin_tune": [SkinTune fields]}.
    pub fn with(&self, v: &Value, scene: &Cc5Scene) -> Self {
        let mut s = self.clone();
        let f = |k: &str| v.get(k).and_then(Value::as_f64).map(|x| x as f32);
        if let Some(t) = v.get("tint").and_then(Value::as_array).filter(|a| a.len() == 3) {
            let c = |i: usize| t[i].as_f64().unwrap_or(1.0) as f32;
            s.tint = Vec3::new(c(0), c(1), c(2));
        }
        s.ambient = f("ambient").unwrap_or(s.ambient);
        s.ibl = f("ibl").unwrap_or(s.ibl);
        s.exposure = f("exposure").unwrap_or(s.exposure);
        s.sat = f("sat").unwrap_or(s.sat);
        if let Some(g) = v.get("gains").and_then(Value::as_object) {
            for (i, l) in scene.lights.iter().enumerate() {
                if let Some(x) = g.get(&l.name).and_then(Value::as_f64) {
                    s.gains[i] = x as f32;
                }
            }
        }
        if let Some(t) = v.get("skin_tune").and_then(Value::as_array) {
            s.skin_tune = SkinTune::parse(&t.iter().filter_map(Value::as_f64).map(|x| x.to_string()).collect::<Vec<_>>().join(","));
        }
        s
    }
}

/// [`Cc5Settings`] wanted and in force; `default` from the constants and the
/// environment (CC5_* knobs).
#[derive(Resource)]
pub struct Cc5Live {
    pub default: Cc5Settings,
    pub wanted: Cc5Settings,
    applied: Cc5Settings,
    lumens: f32,
    lumens_flat: f32,
}

impl Cc5Live {
    /// The wanted settings are on the scene.
    pub fn applied(&self) -> bool {
        self.applied == self.wanted
    }
}

/// A CC5 light's index in [`Cc5Scene::lights`].
#[derive(Component)]
pub struct Cc5LightIndex(pub usize);

fn light_intensity(l: &Cc5Light, gain: f32, lumens: f32, lumens_flat: f32) -> f32 {
    gain * if l.inverse_square { l.multiplier * lumens } else { l.multiplier * lumens_flat * l.transform.translation.distance_squared(HEAD) }
}

/// Puts changed [`Cc5Live`] settings on the scene: grading, IBL (re-prefiltered
/// when the tint changes), lights, ambient, and the skin materials' calibration.
#[allow(clippy::too_many_arguments)]
pub fn cc5_live(
    live: Option<ResMut<Cc5Live>>,
    scene: Option<Res<Cc5Scene>>,
    mut cams: Query<(&mut bevy::render::view::ColorGrading, Option<&mut EnvironmentMapLight>), With<Camera3d>>,
    mut lights: Query<(&Cc5LightIndex, &mut SpotLight)>,
    mut ambient: ResMut<AmbientLight>,
    mut images: ResMut<Assets<Image>>,
    mut skins: ResMut<Assets<Cc5SkinMaterial>>,
    mut materials: ResMut<Cc5Materials>,
) {
    let (Some(mut live), Some(scene)) = (live, scene) else { return };
    if live.applied() {
        return;
    }
    let (w, a) = (live.wanted.clone(), live.applied.clone());
    let maps = if w.tint != a.tint {
        crate::ibl::environment_maps(&scene.ibl, knob("CC5_IBL_ROT", 0.0).to_radians(), w.tint, &mut images).ok()
    } else {
        None
    };
    for (mut g, env) in &mut cams {
        g.global.post_saturation = w.sat;
        g.global.exposure = w.exposure;
        if let Some(mut env) = env {
            env.intensity = w.ibl;
            if let Some((d, s)) = &maps {
                env.diffuse_map = d.clone();
                env.specular_map = s.clone();
            }
        }
    }
    for (i, mut l) in &mut lights {
        let c = &scene.lights[i.0];
        l.intensity = light_intensity(c, w.gains[i.0], live.lumens, live.lumens_flat);
        l.color = tinted(c.color, w.tint);
    }
    ambient.color = tinted(scene.ambient, w.tint);
    ambient.brightness = w.ambient;
    if w.skin_tune != a.skin_tune {
        materials.skin_tune = w.skin_tune;
        for (_, m) in skins.iter_mut() {
            m.extension.source.tune = w.skin_tune;
            let (v, s) = (m.extension.source.vars.clone(), m.extension.source.sss.clone());
            m.extension.set_vars(v, s);
        }
    }
    live.applied = w;
}

/// Startup (after `setup`): CC5's lights instead of the studio rig.
pub fn cc5_lights(
    mut commands: Commands,
    scene: Res<Cc5Scene>,
    studio: Query<Entity, With<DirectionalLight>>,
    cams: Query<Entity, With<Camera3d>>,
    mut images: ResMut<Assets<Image>>,
    materials: Res<Cc5Materials>,
    mut shaders: ResMut<Assets<Shader>>,
) {
    if !scene.use_lights {
        return;
    }
    soft_shadows(&mut shaders);
    // grading (calibrated against CC5's renders of skins and SkinGen layers):
    // ACES, as CC5's response to colour changes (makeup, beards) matches it best;
    // knobs CC5_TONEMAP = aces | agx | tony | blender | reinhard | none,
    // CC5_EXPOSURE (stops), CC5_SAT (post saturation), CC5_CAS (sharpening)
    for c in &cams {
        commands.entity(c).insert(bevy::render::view::ColorGrading {
            global: bevy::render::view::ColorGradingGlobal {
                post_saturation: knob("CC5_SAT", SATURATION),
                exposure: knob("CC5_EXPOSURE", EXPOSURE),
                ..default()
            },
            ..default()
        });
        let k = knob("CC5_CAS", SHARPENING);
        commands.entity(c).insert(bevy::core_pipeline::contrast_adaptive_sharpening::ContrastAdaptiveSharpening {
            enabled: k > 0.0,
            sharpening_strength: k,
            denoise: false,
        });
        use bevy::core_pipeline::tonemapping::Tonemapping;
        let tm = match std::env::var("CC5_TONEMAP").as_deref().unwrap_or("aces") {
            "agx" => Tonemapping::AgX,
            "tony" => Tonemapping::TonyMcMapface,
            "none" => Tonemapping::None,
            "reinhard" => Tonemapping::ReinhardLuminance,
            "blender" => Tonemapping::BlenderFilmic,
            _ => Tonemapping::AcesFitted,
        };
        commands.entity(c).insert(tm);
    }
    let lumens = knob("CC5_LUMENS", LUMENS_PER_MULTIPLIER);
    let lumens_flat = knob("CC5_LUMENS_FLAT", LUMENS_FLAT);
    let beam = std::env::var("CC5_BEAM").ok().and_then(|v| v.parse::<f32>().ok());
    let t = tint();
    match crate::ibl::environment_maps(&scene.ibl, knob("CC5_IBL_ROT", 0.0).to_radians(), t, &mut images) {
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
    // per-light gains (calibration): CC5_GAINS="Top:1.5,Key:0.8"
    let gains: Vec<(String, f32)> = std::env::var("CC5_GAINS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|kv| kv.split_once(':').and_then(|(k, v)| Some((k.trim().to_string(), v.trim().parse().ok()?))))
        .collect();
    let gain_of = |l: &Cc5Light| gains.iter().find(|(n, _)| *n == l.name).map_or(LIGHT_GAINS.iter().find(|(n, _)| *n == l.name).map_or(1.0, |g| g.1), |g| g.1);
    for (i, l) in scene.lights.iter().enumerate() {
        let gain = gain_of(l);
        commands.spawn((
            Cc5LightIndex(i),
            SpotLight {
                color: tinted(l.color, t),
                intensity: light_intensity(l, gain, lumens, lumens_flat),
                range: l.range,
                radius: l.radius * knob("CC5_RADIUS_SCALE", 1.0),
                shadows_enabled: l.shadows,
                outer_angle: beam.map_or(l.angle, |b| (b * 0.5).to_radians()),
                inner_angle: beam.map_or(l.angle, |b| (b * 0.5).to_radians()) * (1.0 - l.falloff),
                soft_shadows_enabled: true,
                ..default()
            },
            if std::env::var("CC5_MIRROR").is_ok_and(|v| v == "1") {
                // diagnostic: the rig mirrored left / right
                let mut tr = l.transform;
                tr.translation.x = -tr.translation.x;
                let f = tr.forward();
                tr.look_to(Vec3::new(-f.x, f.y, f.z), Vec3::Y);
                tr
            } else {
                l.transform
            },
            Name::new(format!("CC5 {}", l.name)),
        ));
    }
    commands.insert_resource(AmbientLight { color: tinted(scene.ambient, t), brightness: knob("CC5_AMBIENT", AMBIENT_BRIGHTNESS) });
    let default = Cc5Settings {
        tint: t,
        ambient: knob("CC5_AMBIENT", AMBIENT_BRIGHTNESS),
        ibl: knob("CC5_IBL_INTENSITY", IBL_INTENSITY),
        exposure: knob("CC5_EXPOSURE", EXPOSURE),
        sat: knob("CC5_SAT", SATURATION),
        gains: scene.lights.iter().map(gain_of).collect(),
        skin_tune: materials.skin_tune,
    };
    commands.insert_resource(Cc5Live { wanted: default.clone(), applied: default.clone(), default, lumens, lumens_flat });
    info!("cc5 scene: {} lights", scene.lights.len());
}
