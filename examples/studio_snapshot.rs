//! Render any character GLB with the avatar editor's studio look (its camera,
//! light rig, IBL and backdrop: `avatar_editor/studio.rs`) and save a screenshot.
//! The editor itself needs the character's CC5 slider pack; this needs only the GLB.
//!
//! Usage: studio_snapshot avatar.glb out.png [--view body|head|feet] [--size WxH]
//!        [--ev100 <ev>] [--yaw <deg>] [--zoom <factor>] [--standard]
//!
//! Materials with `extras.cc5` render with cc-render's CC5 shaders, as in the
//! editor; `--standard` renders the GLB's plain StandardMaterials instead.
//! The views aim at the CC5 joint names cc-render uses (`CC_Base_L_Eye`, ...).
//! `CC_AVATAR_ENVMAP_DIR` names the pisa environment map's folder (default
//! `<glb dir>/env`).
use bevy::asset::RecursiveDependencyLoadState;
use bevy::core_pipeline::experimental::taa::TemporalAntiAliasPlugin;
use bevy::pbr::DirectionalLightShadowMap;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::render::view::NoFrustumCulling;
use bevy::window::{ExitCondition, WindowResolution};
use cc_render_plugin::cc_render::cc5_materials::{generate_mips, Cc5Material, Cc5Materials, Cc5MaterialsPlugin};
use std::collections::HashSet;
use std::path::PathBuf;

#[path = "avatar_editor/studio.rs"]
mod studio;

/// Seconds to render the framed view before the shot: pipeline compilation is
/// asynchronous and TAA needs frames to converge (the editor's snapshot wait).
const SETTLE_SECS: f32 = 6.0;
/// ... and at least this many frames (a long frame, e.g. building mipmaps,
/// must not count as settling).
const SETTLE_FRAMES: u32 = 120;

#[derive(Resource)]
struct Job {
    glb: PathBuf,
    out: PathBuf,
    view: String,
    ev100: f32,
    yaw: f32,
    zoom: f32,
    scene: Handle<Scene>,
    framed: Option<(f32, u32)>,
    shot: bool,
}

#[derive(Component)]
struct Character;

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    let mut opt = |flag: &str| -> anyhow::Result<Option<String>> {
        match args.iter().position(|a| a == flag) {
            Some(i) if i + 1 < args.len() => Ok(Some(args.drain(i..i + 2).nth(1).unwrap())),
            Some(_) => anyhow::bail!("{flag} needs a value"),
            None => Ok(None),
        }
    };
    let view = opt("--view")?.unwrap_or_else(|| "body".into());
    let size = opt("--size")?.unwrap_or_else(|| "1200x1500".into());
    let ev100 = opt("--ev100")?.map(|v| v.parse()).transpose()?.unwrap_or(studio::DEFAULT_EV100);
    let yaw: f32 = opt("--yaw")?.map(|v| v.parse()).transpose()?.unwrap_or(0.0);
    let zoom: f32 = opt("--zoom")?.map(|v| v.parse()).transpose()?.unwrap_or(1.0);
    let standard = args.iter().position(|a| a == "--standard").map(|i| args.remove(i)).is_some();
    anyhow::ensure!(
        args.len() == 3,
        "avatar.glb out.png [--view body|head|feet] [--size WxH] [--ev100 ev] [--yaw deg] [--zoom f] [--standard]"
    );
    anyhow::ensure!(view_preset(&view).is_some(), "--view {view}: expected body, head or feet");
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .ok_or_else(|| anyhow::anyhow!("--size {size}: expected WxH"))?;
    let glb = std::fs::canonicalize(&args[1]).map_err(|e| anyhow::anyhow!("{}: {e}", args[1]))?;
    let out = std::path::absolute(&args[2])?;
    let _ = std::fs::remove_file(&out);

    let window = Window {
        title: "studio snapshot".into(),
        resolution: WindowResolution::new(w as f32, h as f32).with_scale_factor_override(1.0),
        ..default()
    };
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: Some(window), exit_condition: ExitCondition::OnPrimaryClosed, ..default() })
                .set(AssetPlugin { file_path: glb.parent().unwrap().to_string_lossy().into_owned(), ..default() }),
        )
        .add_plugins((TemporalAntiAliasPlugin, Cc5MaterialsPlugin))
        .insert_resource(Cc5Materials { enabled: !standard, ..default() })
        .insert_resource(DirectionalLightShadowMap { size: 4096 })
        .insert_resource(Job {
            glb,
            out,
            view,
            ev100,
            yaw: yaw.to_radians(),
            zoom,
            scene: Handle::default(),
            framed: None,
            shot: false,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (no_culling, mipmaps, snapshot).chain())
        .run();
    Ok(())
}

fn setup(mut commands: Commands, assets: Res<AssetServer>, mut images: ResMut<Assets<Image>>, mut job: ResMut<Job>) {
    let file = job.glb.file_name().unwrap().to_string_lossy().into_owned();
    job.scene = assets.load(GltfAssetLabel::Scene(0).from_asset(file));
    commands.spawn((SceneRoot(job.scene.clone()), Character));
    let env_dir = std::env::var("CC_AVATAR_ENVMAP_DIR").map(PathBuf::from).unwrap_or_else(|_| job.glb.parent().unwrap().join("env"));
    let mut cam = commands.spawn((studio::studio_camera(&mut images, job.ev100), Transform::default()));
    if let Some(ibl) = studio::studio_ibl(&assets, &env_dir) {
        cam.insert(ibl);
    }
    studio::spawn_studio_lights(&mut commands);
}

/// Skinned meshes are culled by their bind-pose bounds; one character is cheap
/// to draw unculled (as in the editor).
fn no_culling(mut commands: Commands, added: Query<Entity, (Added<Mesh3d>, Without<NoFrustumCulling>)>) {
    for e in &added {
        commands.entity(e).insert(NoFrustumCulling);
    }
}

/// Mip chains for every glTF texture, once (Bevy loads PNG / JPEG with one
/// level, so 2K-4K maps seen small alias into noise and moire). Base colours
/// with alpha are averaged weighted by alpha, as for CC5's hair cards.
fn mipmaps(materials: Res<Assets<StandardMaterial>>, mut images: ResMut<Assets<Image>>, mut done: Local<HashSet<AssetId<Image>>>) {
    for (_, m) in materials.iter() {
        let textures = [
            (m.base_color_texture.as_ref(), !matches!(m.alpha_mode, AlphaMode::Opaque)),
            (m.normal_map_texture.as_ref(), false),
            (m.metallic_roughness_texture.as_ref(), false),
            (m.occlusion_texture.as_ref(), false),
            (m.emissive_texture.as_ref(), false),
        ];
        for (t, alpha_weighted) in textures {
            let Some(t) = t else { continue };
            if done.contains(&t.id()) || images.get(t).is_none_or(|i| i.data.is_empty()) {
                continue; // done, or not loaded yet
            }
            done.insert(t.id());
            if let Some(img) = images.get_mut(t) {
                generate_mips(img, alpha_weighted);
            }
        }
    }
}

/// The editor's view buttons: (joints to aim at, distance at 45 degrees).
fn view_preset(name: &str) -> Option<(&'static [&'static str], f32)> {
    match name {
        "head" => Some((&["CC_Base_L_Eye", "CC_Base_R_Eye"], 0.4)),
        "body" => Some((&[], 3.2)),
        "feet" => Some((&["CC_Base_L_Foot", "CC_Base_R_Foot"], 0.9)),
        _ => None,
    }
}

fn snapshot(
    mut job: ResMut<Job>,
    time: Res<Time<Real>>,
    assets: Res<AssetServer>,
    joints: Query<(&Name, &GlobalTransform)>,
    meshes: Query<(), With<Mesh3d>>,
    cc5_added: Query<(), Added<Cc5Material>>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut commands: Commands,
    mut exit: EventWriter<AppExit>,
    mut exiting: Local<bool>,
) {
    if job.shot {
        if !*exiting && job.out.is_file() {
            *exiting = true;
            info!("snapshot {} saved", job.out.display());
            exit.send(AppExit::Success);
        }
        return;
    }
    let Some(framed) = job.framed else {
        if !matches!(assets.recursive_dependency_load_state(&job.scene), RecursiveDependencyLoadState::Loaded) || meshes.is_empty() {
            return;
        }
        let (names, dist) = view_preset(&job.view).unwrap();
        let found: Vec<Vec3> = joints.iter().filter(|(n, _)| names.contains(&n.as_str())).map(|(_, g)| g.translation()).collect();
        if found.len() < names.len() {
            error!("--view {}: the GLB lacks {:?}", job.view, names);
            exit.send(AppExit::error());
            return;
        }
        // the editor's orbit: body view slightly from above, the others level
        let (focus, pitch) = if names.is_empty() {
            (Vec3::new(0.0, 0.95, 0.0), 0.05)
        } else {
            (found.iter().sum::<Vec3>() / found.len() as f32, 0.0)
        };
        let rot = Quat::from_euler(EulerRot::YXZ, job.yaw, -pitch, 0.0);
        let d = dist * studio::fov_scale() / job.zoom;
        if let Ok(mut t) = camera.get_single_mut() {
            *t = Transform::from_translation(focus + rot * Vec3::new(0.0, 0.0, d)).looking_at(focus, Vec3::Y);
        }
        job.framed = Some((0.0, 0));
        return;
    };
    // CC5 shader materials swap in once their maps load: settle from there
    let framed = if cc5_added.is_empty() { (framed.0 + time.delta_secs(), framed.1 + 1) } else { (0.0, 0) };
    job.framed = Some(framed);
    if framed.0 >= SETTLE_SECS && framed.1 >= SETTLE_FRAMES {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(job.out.clone()));
        job.shot = true;
    }
}
