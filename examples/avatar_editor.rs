//! CC5-style avatar shape editor: the sliders captured from Character Creator 5,
//! applied live to the character outside CC5.
//!
//! Usage: avatar_editor character.glb pack_dir [shaped_out.glb] [--recipe recipe.json]
//!                      [--style other_character.glb] [--expressions expressions_dir]
//!
//! `character.glb` is the CC5 reference export converted with `fbx_to_glb.py`
//! (it carries the `_CCIDX` vertex attribute); `pack_dir` holds `pack.json` and
//! `pack.bin` from `build_editor_pack.py`. Every slider change re-evaluates the
//! mix and rewrites the bind mesh, joint translations and inverse bind matrices
//! the same way `shape_glb` does; "Save GLB" writes that as a GLB. A recipe (the
//! changed sliders as JSON, e.g. from `cc5_shape.fit`) can be opened with
//! `--recipe` or loaded/saved from the panel.
//!
//! `--style` loads another CC character (a GLB, e.g. the one a recipe was fitted
//! to) for two optional view layers: "Surface" puts its skin / eye / teeth
//! materials on our CC_Base_* meshes (same CC UV layout, matched by material
//! name); "Assets" shows its hair, clothes and accessories, bound to *our*
//! skeleton by joint name so they follow our shape, in place of ours. Its body is
//! never shown.
//!
//! `--expressions` loads the character's facial expression pack
//! (`build_expression_pack.py`: CC5's ~400 morph targets, more than Bevy's 64 per
//! mesh) for an "Expressions" section: 0..1 sliders applied on the CPU on top of
//! the shape (positions = bind + shape delta + sum of w_i * expression_i).
//! Expressions are a preview layer: "Save GLB" and recipes hold the shape only.
//!
//! `--hd map_dir` edits CC5's HD (subdivided) character instead: `character.glb`
//! is then the HD export (`build_cc5_editor_pack.ps1 -HdLevel 1`: `hd/character_hd.glb`)
//! and `map_dir` its `hd_map.json` + `.bin`, which spreads the pack's (level 0)
//! slider and expression moves over the HD mesh. "Save GLB" writes the HD GLB.
//!
//! `--hair-lib dir [--hair-style name]` adds a Hair section: card-hair parts from
//! a hair library (`remotemedia-sdk/scripts/avatars/cc5_editor/hair/`), one per
//! slot (Base, Top, Rear, Bangs, Beard, Brows, ...) or a named style, conformed
//! to the shaped body (see `avatar_editor/hair.rs`).
//!
//! Controls: right-drag orbits, middle-drag pans, wheel zooms towards the cursor
//! (down to 2 cm); Head / Body / Feet buttons frame those parts.
//!
//! `--snapshot out.png [--view head|body|feet] [--yaw deg] [--pitch deg] [--zoom k]
//! [--pan right,up (m)] [--size WxH] [--no-cc5]` renders one frame without the panel once the character
//! and its CC5 materials are in, writes it to `out.png` and exits (for comparing
//! the look against CC5's viewport). `--ev100` sets the exposure as usual.
use bevy::gltf::{Gltf, GltfPlugin};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::core_pipeline::{bloom::Bloom, tonemapping::Tonemapping, Skybox};
use bevy::core_pipeline::experimental::taa::{TemporalAntiAliasPlugin, TemporalAntiAliasing};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ShadowFilteringMethod};
use bevy::math::{Affine3A, Mat3};
use bevy::pbr::{light_consts::lux, CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::prelude::*;
use bevy::render::camera::Exposure;
use bevy::render::view::{ColorGrading, ColorGradingGlobal};
use bevy::render::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::render::mesh::{MeshVertexAttribute, VertexAttributeValues};
use bevy::render::render_resource::VertexFormat;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, RayCastSettings};
use bevy::render::view::NoFrustumCulling;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use cc_render_plugin::cc_render::shape::{ShapeDelta, ShapePack};
use cc5_shape::{shape_glb_hd, ExpressionPack, SubdivMap};
use cc_render_plugin::cc_render::shape_glb::shape_glb;
use cc_render_plugin::cc_render::cc5_materials::{Cc5Material, Cc5Materials, Cc5MaterialsPlugin, Cc5MaterialsSet};
use std::collections::HashMap;
use std::path::PathBuf;

#[path = "avatar_editor/hair.rs"]
mod hair;
use hair::{hair_bind, hair_shape, hair_sync, hair_ui, HairLib};

const ATTRIBUTE_CCIDX: MeshVertexAttribute = MeshVertexAttribute::new("CcIdx", 0x0CC5_1D_E7, VertexFormat::Float32);

#[derive(Resource)]
struct Editor {
    /// Camera exposure (EV100); the portrait rig is lit for studio levels, not daylight.
    ev100: f32,
    pack: ShapePack,
    categories: Vec<(String, Vec<usize>)>,
    weights: Vec<f32>,
    delta: ShapeDelta,
    dirty: bool,
    /// `--expressions`: the pack and one weight per expression (0 = neutral).
    expr: Option<ExpressionPack>,
    expr_weights: Vec<f32>,
    /// Only expression weights changed: positions are rewritten, joints are not.
    expr_dirty: bool,
    /// `--hd`: the character is CC5's HD mesh; vertex deltas go through this map.
    hd: Option<SubdivMap>,
    search: String,
    glb_path: PathBuf,
    out_path: PathBuf,
    recipe_path: String,
    status: String,
    /// Framing requested by the UI: (joints whose midpoint to aim at, camera distance);
    /// no joints = whole body.
    focus_request: Option<(&'static [&'static str], f32)>,
}

/// Base state of the spawned character, captured once it has settled.
#[derive(Resource, Default)]
struct Rig {
    ready: bool,
    settle_frames: u32,
    meshes: Vec<MeshRec>,
    joints: Vec<JointRec>,
    skins: Vec<SkinRec>,
}

struct MeshRec {
    /// glTF mesh name
    name: String,
    handle: Handle<Mesh>,
    base: Vec<[f32; 3]>,
    global: Vec<u32>,
    /// mesh (bind) space -> world at bind
    bind_space: Mat4,
    to_mesh: Mat3,
}

struct JointRec {
    entity: Entity,
    name: String,
    pack: Option<usize>,
    parent: Option<usize>,
    base_local: Vec3,
    parent_lin_inv: Mat3,
    base_world: Affine3A,
}

struct SkinRec {
    handle: Handle<SkinnedMeshInverseBindposes>,
    joints: Vec<usize>,
    base: Vec<Mat4>,
}

/// Root of the character being edited (the pack's character).
#[derive(Component)]
struct OurCharacter;

/// Root of the `--style` character.
#[derive(Component)]
struct StyleCharacter;

/// `--style` view layers.
#[derive(Resource, Default)]
struct Style {
    path: Option<PathBuf>,
    our_gltf: Handle<Gltf>,
    style_gltf: Handle<Gltf>,
    ready: bool,
    surface: bool,
    assets: bool,
    applied: Option<(bool, bool)>,
    /// Our CC_Base_* primitives: entity, original material, its glTF material name.
    our_base: Vec<(Entity, Handle<StandardMaterial>, Option<String>)>,
    /// Our hair / clothes / accessories.
    our_assets: Vec<Entity>,
    style_assets: Vec<Entity>,
    style_materials: HashMap<String, Handle<StandardMaterial>>,
    status: String,
}

/// `--snapshot`: render a camera preset to a file, then exit.
#[derive(Resource)]
struct Snapshot {
    path: PathBuf,
    view: String,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    /// Focus offset along the camera's right / up (m).
    pan: Vec2,
    /// Seconds since the rig was ready and the CC5 material count last changed.
    stable: f32,
    cc5_count: usize,
    /// Frames since the materials settled.
    step: u32,
    /// Seconds since the camera preset was applied.
    framed: f32,
}

#[derive(Component)]
struct Orbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
}

/// Default camera exposure: Bevy's default (EV100 9.7, daylight) leaves the
/// portrait rig (a few hundred lux) far too dark.
const DEFAULT_EV100: f32 = 7.25;

/// `--snapshot`: seconds to render the framed view before the shot (shader and
/// pipeline compilation for new views and materials is asynchronous).
const SNAPSHOT_SETTLE_SECS: f32 = 6.0;

/// Size of the key light for its soft (PCSS) shadows.
const KEY_SOFT_SHADOW: f32 = 6.0;

/// Vertical field of view: a portrait lens. Bevy's default 45 degrees distorts
/// close-ups of the face like a wide angle.
const FOV_DEG: f32 = 25.0;

/// Camera distances below are framed for 45 degrees; scale them to FOV_DEG.
fn fov_scale() -> f32 {
    (22.5f32.to_radians()).tan() / (FOV_DEG.to_radians() * 0.5).tan()
}

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    let mut opt = |flag: &str| -> anyhow::Result<Option<String>> {
        match args.iter().position(|a| a == flag) {
            Some(i) if i + 1 < args.len() => Ok(Some(args.drain(i..i + 2).nth(1).unwrap())),
            Some(_) => anyhow::bail!("{flag} needs a file"),
            None => Ok(None),
        }
    };
    let recipe_arg = opt("--recipe")?;
    let ev100: f32 = opt("--ev100")?.map(|v| v.parse()).transpose()?.unwrap_or(DEFAULT_EV100);
    let style_arg = opt("--style")?.map(std::fs::canonicalize).transpose()?;
    let expr = opt("--expressions")?.map(|d| ExpressionPack::load(std::path::Path::new(&d))).transpose()?;
    let hd = opt("--hd")?.map(|d| SubdivMap::load(std::path::Path::new(&d))).transpose()?;
    let mut hair_lib = opt("--hair-lib")?.map(|d| HairLib::load(std::path::Path::new(&d))).transpose()?;
    if let Some(name) = opt("--hair-style")? {
        hair_lib.as_mut().ok_or_else(|| anyhow::anyhow!("--hair-style needs --hair-lib"))?.apply_style(&name)?;
    }
    let snapshot_arg = opt("--snapshot")?;
    let view = opt("--view")?.unwrap_or_else(|| "head".into());
    let deg = |v: Option<String>| -> anyhow::Result<f32> { Ok(v.map(|v| v.parse::<f32>()).transpose()?.unwrap_or(0.0).to_radians()) };
    let (yaw, pitch) = (deg(opt("--yaw")?)?, deg(opt("--pitch")?)?);
    let zoom: f32 = opt("--zoom")?.map(|v| v.parse()).transpose()?.unwrap_or(1.0);
    let pan = match opt("--pan")? {
        Some(p) => {
            let (x, y) = p.split_once(',').ok_or_else(|| anyhow::anyhow!("--pan right,up"))?;
            Vec2::new(x.parse()?, y.parse()?)
        }
        None => Vec2::ZERO,
    };
    let size = opt("--size")?.unwrap_or_else(|| "1139x978".into());
    let (w, h) = size.split_once('x').ok_or_else(|| anyhow::anyhow!("--size WxH"))?;
    let size = (w.parse::<f32>()?, h.parse::<f32>()?);
    let no_cc5 = match args.iter().position(|a| a == "--no-cc5") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    anyhow::ensure!(args.len() == 3 || args.len() == 4, "character.glb pack_dir [shaped_out.glb] [--recipe recipe.json] required");
    let glb_path = std::fs::canonicalize(&args[1])?;
    let pack = ShapePack::load(std::path::Path::new(&args[2]))?;
    let out_path = args.get(3).map(PathBuf::from).unwrap_or_else(|| glb_path.with_extension("shaped.glb"));
    let weights = match &recipe_arg {
        Some(r) => pack.weights_from_recipe(&serde_json::from_slice(&std::fs::read(r)?)?)?,
        None => pack.base_weights(),
    };
    let delta = pack.evaluate(&weights);
    if let Some(x) = &expr {
        anyhow::ensure!(
            x.vertex_count == pack.vertex_count,
            "expression pack has {} vertices, slider pack {}: built for another character",
            x.vertex_count,
            pack.vertex_count
        );
    }
    if let Some(h) = &hd {
        h.check(&pack).map_err(|e| anyhow::anyhow!("--hd map does not belong to this pack: {e:#}"))?;
    }
    let editor = Editor {
        categories: pack.categories(),
        status: format!("{} sliders in {} categories", pack.sliders.len(), pack.categories().len()),
        pack,
        weights,
        delta,
        dirty: false,
        expr_weights: vec![0.0; expr.as_ref().map_or(0, |x| x.len())],
        expr,
        expr_dirty: false,
        hd,
        search: String::new(),
        glb_path: glb_path.clone(),
        focus_request: None,
        ev100,
        recipe_path: recipe_arg.unwrap_or_else(|| out_path.with_extension("recipe.json").to_string_lossy().into_owned()),
        out_path,
    };
    let asset_dir = glb_path.parent().unwrap().to_string_lossy().to_string();
    let mut window = Window { title: "CC5 avatar editor".into(), ..default() };
    if snapshot_arg.is_some() {
        window.resolution = bevy::window::WindowResolution::new(size.0, size.1).with_scale_factor_override(1.0);
    }
    let snap = snapshot_arg.map(|p| Snapshot {
        path: PathBuf::from(p),
        view,
        yaw,
        pitch,
        zoom,
        pan,
        stable: 0.0,
        cc5_count: 0,
        step: 0,
        framed: 0.0,
    });
    let mut app = App::new();
    app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: Some(window), ..default() })
                .set(AssetPlugin { file_path: asset_dir, ..default() })
                .set(
                    GltfPlugin::default()
                        .add_custom_vertex_attribute("CCIDX", ATTRIBUTE_CCIDX)
                        .add_custom_vertex_attribute("_CCIDX", ATTRIBUTE_CCIDX),
                ),
        )
        .add_plugins(EguiPlugin)
        .add_plugins(TemporalAntiAliasPlugin)
        .add_plugins(Cc5MaterialsPlugin)
        .insert_resource(DirectionalLightShadowMap { size: 4096 })
        .insert_resource(editor)
        .init_resource::<Rig>()
        .insert_resource(Style { path: style_arg, ..default() })
        .insert_resource(Cc5Materials { enabled: !no_cc5, ..default() })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                no_culling,
                capture_rig,
                style_setup,
                ui.run_if(not(resource_exists::<Snapshot>)),
                style_apply,
                (hair_sync, hair_bind).chain().run_if(resource_exists::<HairLib>),
                apply_shape,
                snapshot.run_if(resource_exists::<Snapshot>),
                orbit_camera,
            )
                .chain()
                .after(Cc5MaterialsSet),
        );
    if let Some(s) = snap {
        app.insert_resource(s);
    }
    if let Some(h) = hair_lib {
        app.insert_resource(h);
    }
    app.run();
    Ok(())
}

/// Studio backdrop like CC5's viewport: soft grey-blue, darkening above eye level
/// (at the editor's 25 degree lens the top of a level close-up is only ~12 degrees
/// up). A small sRGB cubemap for `Skybox`: it stays out of the lighting and of
/// mesh picking.
fn studio_backdrop(images: &mut Assets<Image>) -> Handle<Image> {
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
fn backdrop_brightness(ev100: f32) -> f32 {
    // Bevy's exposure: 1 / (1.2 * 2^ev100); 0.55 = calibrated against CC5's viewport.
    0.4 * 1.2 * 2f32.powf(ev100)
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    editor: Res<Editor>,
    mut style: ResMut<Style>,
    mut images: ResMut<Assets<Image>>,
) {
    let file = editor.glb_path.file_name().unwrap().to_string_lossy().to_string();
    commands.spawn((SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file.clone()))), OurCharacter));
    if let Some(path) = style.path.clone() {
        let ap = bevy::asset::AssetPath::from_path(&path).into_owned();
        style.our_gltf = assets.load(file);
        style.style_gltf = assets.load(ap.clone());
        commands.spawn((SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(ap))), StyleCharacter));
        style.status = "loading style character...".into();
    }

    // Like cc-render's runtime camera (bevy_app/camera.rs): HDR + AgX + bloom + 4x
    // MSAA and the pisa environment map; the backdrop, studio lights, exposure and
    // grading are calibrated against CC5's viewport (its "Beautiful" close-up).
    let env_dir = std::env::var("CC_AVATAR_ENVMAP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| editor.glb_path.parent().unwrap().join("env"));
    let has_env = env_dir.join("pisa_diffuse_rgb9e5_zstd.ktx2").is_file();
    let orbit = Orbit { focus: Vec3::new(0.0, 0.95, 0.0), yaw: 0.0, pitch: 0.05, dist: 3.2 * fov_scale() };
    let mut cam = commands.spawn((
        Camera3d::default(),
        Camera { hdr: true, clear_color: ClearColorConfig::Custom(Color::srgb(0.20, 0.22, 0.26)), ..default() },
        Skybox { image: studio_backdrop(&mut images), brightness: backdrop_brightness(editor.ev100), ..default() },
        // TAA instead of MSAA: it resolves the CC5 hair's dithered strands into a soft
        // mass (hair_types.wgsl) and the soft shadows' noise (temporal filtering).
        Msaa::Off,
        TemporalAntiAliasing::default(),
        ShadowFilteringMethod::Temporal,
        // contact / inter-layer occlusion of ambient light (hair layers, creases)
        ScreenSpaceAmbientOcclusion { constant_object_thickness: 0.06, ..default() },
        Tonemapping::AgX,
        Exposure { ev100: editor.ev100 },
        Bloom { intensity: 0.04, low_frequency_boost: 0.5, high_pass_frequency: 0.95, ..default() },
        // AgX desaturates skin and hair against CC5's viewport: restore some saturation
        ColorGrading { global: ColorGradingGlobal { post_saturation: 1.3, ..default() }, ..default() },
        // 5 mm near plane: close-ups of the face/eyes must not clip away.
        Projection::Perspective(PerspectiveProjection { fov: FOV_DEG.to_radians(), near: 0.005, ..default() }),
        orbit_transform(&orbit),
        orbit,
    ));
    if has_env {
        let path = |f: &str| bevy::asset::AssetPath::from_path(&env_dir.join(f)).into_owned();
        cam.insert(EnvironmentMapLight {
            diffuse_map: assets.load(path("pisa_diffuse_rgb9e5_zstd.ktx2")),
            specular_map: assets.load(path("pisa_specular_rgb9e5_zstd.ktx2")),
            intensity: 1.0,
            rotation: Quat::IDENTITY,
        });
    } else {
        warn!("no environment map in {} (set CC_AVATAR_ENVMAP_DIR); lighting will be flat", env_dir.display());
    }
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

/// Never frustum-cull the character: Bevy culls skinned meshes by their bind-pose
/// bounds, which go stale once the editor reshapes the mesh and moves the joints,
/// so parts vanish at close range. One character is cheap to draw unculled.
fn no_culling(mut commands: Commands, added: Query<Entity, (Added<Mesh3d>, Without<NoFrustumCulling>)>) {
    for e in &added {
        commands.entity(e).insert(NoFrustumCulling);
    }
}

/// Camera framing of the View buttons: (joints to aim at, distance at 45 degrees).
fn view_preset(name: &str) -> Option<(&'static [&'static str], f32)> {
    match name.to_lowercase().as_str() {
        "head" => Some((&["CC_Base_L_Eye", "CC_Base_R_Eye"], 0.4)),
        "body" => Some((&[], 3.2)),
        "feet" => Some((&["CC_Base_L_Foot", "CC_Base_R_Foot"], 0.9)),
        _ => None,
    }
}

/// `--snapshot`: once the rig is ready and the CC5 materials have stopped changing,
/// frame the preset, give the render pipelines time to compile (asynchronously:
/// meshes whose pipeline is not ready are skipped), save a screenshot and exit.
#[allow(clippy::too_many_arguments)]
fn snapshot(
    mut snap: ResMut<Snapshot>,
    time: Res<Time<Real>>,
    rig: Res<Rig>,
    mut editor: ResMut<Editor>,
    cc5: Query<(), With<Cc5Material>>,
    mut orbit: Query<&mut Orbit>,
    mut commands: Commands,
    mut exit: EventWriter<AppExit>,
    hair: Option<Res<HairLib>>,
) {
    use bevy::render::view::screenshot::{save_to_disk, Screenshot};
    if !rig.ready || hair.is_some_and(|h| !h.settled()) {
        return;
    }
    let n = cc5.iter().count();
    if n != snap.cc5_count {
        snap.cc5_count = n;
        snap.stable = 0.0;
    }
    snap.stable += time.delta_secs();
    if snap.stable < 1.5 {
        return; // materials still building / textures still loading
    }
    snap.step = snap.step.saturating_add(1);
    if snap.step > 3 {
        snap.framed += time.delta_secs();
    }
    match snap.step {
        1 => match view_preset(&snap.view) {
            Some(p) => {
                editor.focus_request = Some(p);
                let _ = std::fs::remove_file(&snap.path);
            }
            None => {
                error!("--view {}: expected head, body or feet", snap.view);
                exit.send(AppExit::error());
            }
        },
        3 => {
            if let Ok(mut o) = orbit.get_single_mut() {
                o.yaw += snap.yaw;
                o.pitch += snap.pitch;
                o.dist /= snap.zoom;
                let right = Quat::from_rotation_y(o.yaw) * Vec3::X;
                o.focus += right * snap.pan.x + Vec3::Y * snap.pan.y;
            }
        }
        _ if snap.framed >= SNAPSHOT_SETTLE_SECS && snap.step != u32::MAX => {
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(snap.path.clone()));
            snap.step = u32::MAX; // shot requested
        }
        u32::MAX if snap.path.is_file() || snap.framed > SNAPSHOT_SETTLE_SECS + 30.0 => {
            exit.send(AppExit::Success);
        }
        _ => {}
    }
}

fn orbit_transform(o: &Orbit) -> Transform {
    let rot = Quat::from_euler(EulerRot::YXZ, o.yaw, -o.pitch, 0.0);
    Transform::from_translation(o.focus + rot * Vec3::new(0.0, 0.0, o.dist)).looking_at(o.focus, Vec3::Y)
}

#[allow(clippy::too_many_arguments)]
fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    windows: Query<&Window>,
    mut editor: ResMut<Editor>,
    rig: Res<Rig>,
    joints: Query<(&Name, &GlobalTransform)>,
    mut ray_cast: MeshRayCast,
    mut contexts: EguiContexts,
    mut q: Query<(&mut Orbit, &mut Transform, &Camera, &GlobalTransform)>,
) {
    let Ok((mut o, mut t, camera, cam_g)) = q.get_single_mut() else { return };
    if let Some((names, dist)) = editor.focus_request.take() {
        // our character's joints only: hair parts bring skeletons of their own
        // (re-bound to ours, their nodes stay at bind and would drag the focus)
        let found: Vec<Vec3> = if rig.joints.is_empty() {
            joints.iter().filter(|(n, _)| names.contains(&n.as_str())).map(|(_, g)| g.translation()).collect()
        } else {
            rig.joints.iter().filter(|j| names.contains(&j.name.as_str())).filter_map(|j| joints.get(j.entity).ok()).map(|(_, g)| g.translation()).collect()
        };
        if names.is_empty() {
            o.focus = Vec3::new(0.0, 0.95, 0.0);
        } else if !found.is_empty() {
            o.focus = found.iter().sum::<Vec3>() / found.len() as f32;
            o.pitch = 0.0;
        }
        o.dist = dist * fov_scale();
        *t = orbit_transform(&o);
        return;
    }
    if contexts.ctx_mut().wants_pointer_input() {
        return;
    }
    let d = motion.delta;
    if buttons.pressed(MouseButton::Right) {
        o.yaw -= d.x * 0.008;
        o.pitch = (o.pitch + d.y * 0.008).clamp(-1.4, 1.4);
    }
    if buttons.pressed(MouseButton::Middle) {
        let right = t.right();
        let up = t.up();
        let s = o.dist * 0.0015 / fov_scale();
        o.focus += -right * d.x * s + up * d.y * s;
    }
    if scroll.delta.y != 0.0 {
        let k = (1.0 - scroll.delta.y * 0.12).clamp(0.5, 1.5);
        let new_dist = (o.dist * k).clamp(0.02, 20.0);
        let k = new_dist / o.dist;
        // Zoom towards the surface point under the cursor: scale camera and focus
        // about it, so the camera closes in on it but never passes through the skin.
        // Over empty space, use the plane through the focus facing the camera.
        let cursor = windows.get_single().ok().and_then(|w| w.cursor_position());
        if let Some(ray) = cursor.and_then(|c| camera.viewport_to_world(cam_g, c).ok()) {
            let surface = ray_cast.cast_ray(ray, &RayCastSettings::default()).first().map(|(_, h)| h.point);
            let n = cam_g.forward();
            let denom = ray.direction.dot(*n);
            let anchor = surface.or_else(|| {
                (denom.abs() > 1e-6).then(|| ray.origin + ray.direction * ((o.focus - ray.origin).dot(*n) / denom))
            });
            if let Some(hit) = anchor {
                o.focus = hit + (o.focus - hit) * k;
            }
        }
        o.dist = new_dist;
    }
    *t = orbit_transform(&o);
}

/// Record base positions, joints and inverse bind matrices once the scene has settled.
#[allow(clippy::too_many_arguments)]
fn capture_rig(
    mut rig: ResMut<Rig>,
    mut editor: ResMut<Editor>,
    meshes: Res<Assets<Mesh>>,
    ibms: Res<Assets<SkinnedMeshInverseBindposes>>,
    skinned_all: Query<(Entity, &Name, &Mesh3d, &SkinnedMesh)>,
    joints_q: Query<(&Name, &Transform, &GlobalTransform, Option<&Parent>)>,
    parents: Query<&Parent>,
    ours: Query<(), With<OurCharacter>>,
) {
    if rig.ready {
        return;
    }
    // Only our character: a --style character is in the scene too.
    let skinned: Vec<(&Name, &Mesh3d, &SkinnedMesh)> = skinned_all
        .iter()
        .filter(|(e, ..)| ours.contains(root_of(*e, &parents)))
        .map(|(_, n, m, s)| (n, m, s))
        .collect();
    if skinned.is_empty() {
        return;
    }
    let loaded = skinned.iter().all(|(_, m, s)| meshes.get(&m.0).is_some() && ibms.get(&s.inverse_bindposes).is_some());
    rig.settle_frames = if loaded { rig.settle_frames + 1 } else { 0 };
    if rig.settle_frames < 3 {
        return; // let transform propagation run on the freshly spawned scene
    }

    let pack_joint: HashMap<&str, usize> = editor.pack.joints.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    let mut joint_index: HashMap<Entity, usize> = HashMap::new();
    let mut joints: Vec<JointRec> = Vec::new();
    for (_, _, s) in skinned.iter() {
        for &e in &s.joints {
            if joint_index.contains_key(&e) {
                continue;
            }
            let Ok((name, t, g, _)) = joints_q.get(e) else { continue };
            joint_index.insert(e, joints.len());
            joints.push(JointRec {
                entity: e,
                name: name.to_string(),
                pack: pack_joint.get(name.as_str()).copied(),
                parent: None,
                base_local: t.translation,
                parent_lin_inv: Mat3::IDENTITY,
                base_world: g.affine(),
            });
        }
    }
    for j in &mut joints {
        if let Ok((_, _, _, Some(p))) = joints_q.get(j.entity) {
            if let Ok((_, _, pg, _)) = joints_q.get(p.get()) {
                j.parent_lin_inv = Mat3::from(pg.affine().matrix3).inverse();
            }
            j.parent = joint_index.get(&p.get()).copied();
        }
    }

    let mut skins = Vec::new();
    let mut meshes_out = Vec::new();
    let mut missing = 0;
    for &(name, mesh3d, s) in skinned.iter() {
        let mesh = meshes.get(&mesh3d.0).unwrap();
        let base_ibm: Vec<Mat4> = ibms.get(&s.inverse_bindposes).unwrap().to_vec();
        let jidx: Vec<usize> = s.joints.iter().map(|e| joint_index[e]).collect();
        let bind_space = Mat4::from(joints[jidx[0]].base_world) * base_ibm[0];
        if !skins.iter().any(|k: &SkinRec| k.handle == s.inverse_bindposes) {
            skins.push(SkinRec { handle: s.inverse_bindposes.clone(), joints: jidx, base: base_ibm });
        }
        let mesh_name = name.as_str();
        // `_CCIDX` indexes the HD mesh with --hd, else the pack's level-0 mesh
        let mesh_offset = |n: &str| match &editor.hd {
            Some(h) => h.mesh_offset(n),
            None => editor.pack.mesh_offset(n),
        };
        let offset = mesh_offset(mesh_name).or_else(|| {
            let (stem, idx) = mesh_name.rsplit_once('.')?;
            idx.parse::<u32>().ok()?;
            mesh_offset(stem)
        });
        let (Some(offset), Some(VertexAttributeValues::Float32x3(pos)), Some(VertexAttributeValues::Float32(cc))) =
            (offset, mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.attribute(ATTRIBUTE_CCIDX))
        else {
            missing += 1;
            continue;
        };
        meshes_out.push(MeshRec {
            name: mesh_name.to_string(),
            handle: mesh3d.0.clone(),
            base: pos.clone(),
            global: cc.iter().map(|&c| offset + c as u32).collect(),
            bind_space,
            to_mesh: Mat3::from_mat4(bind_space.inverse()),
        });
    }
    let (nj, np) = (joints.iter().filter(|j| j.pack.is_some()).count(), meshes_out.len());
    rig.joints = joints;
    rig.skins = skins;
    rig.meshes = meshes_out;
    rig.ready = true;
    editor.status = format!(
        "{} sliders{}; {np} mesh primitives and {nj} joints linked{}",
        editor.pack.sliders.len(),
        editor.expr.as_ref().map_or(String::new(), |x| format!(", {} expressions", x.len())),
        if missing > 0 { format!(" ({missing} primitives without _CCIDX left unshaped)") } else { String::new() }
    );
    editor.dirty = true;
}

fn root_of(mut e: Entity, parents: &Query<&Parent>) -> Entity {
    while let Ok(p) = parents.get(e) {
        e = p.get();
    }
    e
}

/// Once both characters are in: classify primitives, hide the style character's
/// body, and bind its hair/clothes/accessories to our skeleton by joint name.
#[allow(clippy::too_many_arguments)]
fn style_setup(
    mut style: ResMut<Style>,
    rig: Res<Rig>,
    gltfs: Res<Assets<Gltf>>,
    mut commands: Commands,
    mut prims: Query<
        (Entity, &Name, Option<&MeshMaterial3d<StandardMaterial>>, Option<&Cc5Material>, Option<&mut SkinnedMesh>),
        With<Mesh3d>,
    >,
    names: Query<(&Name, &GlobalTransform), Without<Mesh3d>>,
    mut ibms: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    parents: Query<&Parent>,
    ours: Query<(), With<OurCharacter>>,
    theirs: Query<(), With<StyleCharacter>>,
    mut frames: Local<u32>,
) {
    if style.path.is_none() || style.ready || !rig.ready {
        return;
    }
    let (Some(our_g), Some(style_g)) = (gltfs.get(&style.our_gltf), gltfs.get(&style.style_gltf)) else { return };
    let style_prims = prims.iter().filter(|(e, ..)| theirs.contains(root_of(*e, &parents))).count();
    *frames = if style_prims > 0 { *frames + 1 } else { 0 };
    if *frames < 3 {
        return; // let the style scene finish spawning
    }
    let our_names: HashMap<AssetId<StandardMaterial>, String> =
        our_g.named_materials.iter().map(|(n, h)| (h.id(), n.to_string())).collect();
    // our joints by name: entity and base (unshaped) world transform
    let our_joint: HashMap<&str, (Entity, Mat4)> =
        rig.joints.iter().map(|j| (j.name.as_str(), (j.entity, Mat4::from(j.base_world)))).collect();
    let (mut bound, mut unbound) = (0usize, 0usize);
    for (e, name, mat, cc5, skin) in prims.iter_mut() {
        // CC5-shaded primitives keep their StandardMaterial in Cc5Material
        let Some(mat) = cc5.map(|c| c.standard.clone()).or_else(|| mat.map(|m| m.0.clone())) else { continue };
        let root = root_of(e, &parents);
        let base = name.as_str().starts_with("CC_Base_");
        if ours.contains(root) {
            if base {
                style.our_base.push((e, mat.clone(), our_names.get(&mat.id()).cloned()));
            } else {
                style.our_assets.push(e);
            }
        } else if theirs.contains(root) {
            commands.entity(e).insert(Visibility::Hidden);
            if base {
                continue; // the style character's own body is never shown
            }
            style.style_assets.push(e);
            if let Some(mut skin) = skin {
                // Re-bind to our joints by name. Their IBMs expect *their* joint frames
                // (bone axes differ between exports), so rebuild them:
                //   IBM' = W_ours^-1 * T(p_ours - p_theirs) * W_theirs * IBM_theirs
                // At bind the asset sits exactly as in its own file, shifted per joint
                // by how far our joint is from theirs; it then follows our joints.
                let Some(old) = ibms.get(&skin.inverse_bindposes).map(|a| a.to_vec()) else { continue };
                let mut new = old.clone();
                for (k, j) in skin.joints.iter_mut().enumerate() {
                    let Ok((jn, jg)) = names.get(*j) else {
                        unbound += 1;
                        continue;
                    };
                    match our_joint.get(jn.as_str()) {
                        Some(&(ours_e, ours_w)) => {
                            let theirs_w = jg.compute_matrix();
                            let shift = Mat4::from_translation(ours_w.w_axis.truncate() - theirs_w.w_axis.truncate());
                            new[k] = ours_w.inverse() * shift * theirs_w * old[k];
                            *j = ours_e;
                            bound += 1;
                        }
                        None => unbound += 1, // their own extra bone: stays on their skeleton
                    }
                }
                skin.inverse_bindposes = ibms.add(SkinnedMeshInverseBindposes::from(new));
            }
        }
    }
    style.style_materials = style_g.named_materials.iter().map(|(n, h)| (n.to_string(), h.clone())).collect();
    let matched = style
        .our_base
        .iter()
        .filter(|(_, _, n)| n.as_ref().is_some_and(|n| style.style_materials.contains_key(n)))
        .count();
    style.status = format!(
        "{}: {matched}/{} base materials match; {} assets ({bound} joint refs bound to ours, {unbound} kept)",
        style.path.as_ref().unwrap().file_name().unwrap().to_string_lossy(),
        style.our_base.len(),
        style.style_assets.len()
    );
    style.ready = true;
}

/// Apply the Surface / Assets toggles: material *handle* swaps and visibility only
/// (mutating material assets corrupts GPU bindings in Bevy 0.15).
fn style_apply(mut style: ResMut<Style>, mut cc5: ResMut<Cc5Materials>, mut commands: Commands) {
    if !style.ready || style.applied == Some((style.surface, style.assets)) {
        return;
    }
    for (e, orig, name) in &style.our_base {
        let m = match (style.surface, name) {
            (true, Some(n)) => style.style_materials.get(n).cloned().unwrap_or_else(|| orig.clone()),
            _ => orig.clone(),
        };
        commands.entity(*e).insert(MeshMaterial3d(m));
    }
    let (ours, theirs) =
        if style.assets { (Visibility::Hidden, Visibility::Inherited) } else { (Visibility::Inherited, Visibility::Hidden) };
    for e in &style.our_assets {
        commands.entity(*e).insert(ours);
    }
    for e in &style.style_assets {
        commands.entity(*e).insert(theirs);
    }
    // the style character's surface is plain StandardMaterials: compare like with like
    if style.surface && cc5.enabled {
        cc5.enabled = false;
    }
    style.applied = Some((style.surface, style.assets));
}

fn ui(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut style: ResMut<Style>,
    mut cc5: ResMut<Cc5Materials>,
    mut exposure: Query<(&mut Exposure, Option<&mut Skybox>)>,
    mut hair: Option<ResMut<HairLib>>,
) {
    let ed = &mut *editor;
    egui::SidePanel::left("sliders").default_width(420.0).show(contexts.ctx_mut(), |ui| {
        ui.heading("CC5 avatar editor");
        ui.label(&ed.status);
        let active = ed.weights.iter().zip(&ed.pack.sliders).filter(|(w, s)| (**w - s.base).abs() > 1e-6).count();
        ui.horizontal(|ui| {
            ui.label(format!("{active} slider(s) changed"));
            if ui.button("Reset all").clicked() {
                ed.weights = ed.pack.base_weights();
                ed.dirty = true;
            }
            if ui.button("Save GLB").clicked() {
                let result = std::fs::read(&ed.glb_path)
                    .map_err(anyhow::Error::from)
                    .and_then(|src| match &ed.hd {
                        Some(h) => shape_glb_hd(&src, &ed.pack, h, &h.expand(&ed.delta)),
                        None => shape_glb(&src, &ed.pack, &ed.delta),
                    })
                    .and_then(|out| std::fs::write(&ed.out_path, out).map_err(anyhow::Error::from));
                ed.status = match result {
                    Ok(()) => format!("saved {}", ed.out_path.display()),
                    Err(e) => format!("save failed: {e:#}"),
                };
            }
        });
        ui.horizontal(|ui| {
            ui.label("Recipe");
            ui.text_edit_singleline(&mut ed.recipe_path);
            if ui.button("Load").clicked() {
                let loaded = std::fs::read(&ed.recipe_path)
                    .map_err(anyhow::Error::from)
                    .and_then(|b| Ok(serde_json::from_slice::<serde_json::Value>(&b)?))
                    .and_then(|v| ed.pack.weights_from_recipe(&v));
                ed.status = match loaded {
                    Ok(w) => {
                        ed.weights = w;
                        ed.dirty = true;
                        format!("loaded {}", ed.recipe_path)
                    }
                    Err(e) => format!("load failed: {e:#}"),
                };
            }
            if ui.button("Save").clicked() {
                let json = serde_json::to_vec_pretty(&ed.pack.recipe(&ed.weights)).unwrap();
                ed.status = match std::fs::write(&ed.recipe_path, json) {
                    Ok(()) => format!("saved {}", ed.recipe_path),
                    Err(e) => format!("save failed: {e}"),
                };
            }
        });
        if style.path.is_some() {
            ui.horizontal(|ui| {
                ui.label("Style");
                ui.add_enabled_ui(style.ready, |ui| {
                    ui.checkbox(&mut style.surface, "Surface");
                    ui.checkbox(&mut style.assets, "Assets");
                });
            });
            ui.small(&style.status);
        }
        ui.horizontal(|ui| {
            ui.label("Shading");
            let mut on = cc5.enabled;
            ui.add_enabled_ui(!style.surface, |ui| {
                ui.checkbox(&mut on, "CC5 shaders").on_hover_text(
                    "CC5's skin / hair / eye / teeth shaders from their maps and settings vs plain glTF PBR",
                );
            });
            if on != cc5.enabled {
                cc5.enabled = on;
            }
            ui.label("Exposure");
            if ui
                .add(egui::DragValue::new(&mut ed.ev100).speed(0.05).range(0.0..=16.0).prefix("EV100 "))
                .on_hover_text("lower = brighter")
                .changed()
            {
                for (mut e, sky) in &mut exposure {
                    e.ev100 = ed.ev100;
                    if let Some(mut sky) = sky {
                        sky.brightness = backdrop_brightness(ed.ev100);
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("View");
            for name in ["Head", "Body", "Feet"] {
                if ui.button(name).clicked() {
                    ed.focus_request = view_preset(name);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.text_edit_singleline(&mut ed.search);
        });
        ui.separator();
        let needle = ed.search.to_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(h) = hair.as_deref_mut() {
                hair_ui(ui, h);
            }
            if let Some(x) = &ed.expr {
                // Expressions that move nothing (CC5 exports some empty correctives) are hidden.
                let shown: Vec<usize> = (0..x.len())
                    .filter(|&i| x.expressions[i].verts[1] > 0)
                    .filter(|&i| {
                        needle.is_empty()
                            || x.expressions[i].name.to_lowercase().contains(&needle)
                            || "expressions".contains(&needle)
                    })
                    .collect();
                if !shown.is_empty() {
                    let active = ed.expr_weights.iter().filter(|w| **w != 0.0).count();
                    egui::CollapsingHeader::new(format!("Expressions ({}, {active} active)", shown.len()))
                        .default_open(!needle.is_empty())
                        .show(ui, |ui| {
                            if active > 0 && ui.button("Neutral").clicked() {
                                ed.expr_weights.iter_mut().for_each(|w| *w = 0.0);
                                ed.expr_dirty = true;
                            }
                            for i in shown {
                                ui.horizontal(|ui| {
                                    let changed = ui
                                        .add(egui::Slider::new(&mut ed.expr_weights[i], 0.0..=1.0).text(&x.expressions[i].name))
                                        .changed();
                                    if ed.expr_weights[i] != 0.0 && ui.small_button("↺").clicked() {
                                        ed.expr_weights[i] = 0.0;
                                        ed.expr_dirty = true;
                                    }
                                    ed.expr_dirty |= changed;
                                });
                            }
                        });
                }
            }
            for (cat, items) in &ed.categories {
                let shown: Vec<usize> = items
                    .iter()
                    .copied()
                    .filter(|&i| needle.is_empty() || ed.pack.sliders[i].name.to_lowercase().contains(&needle) || cat.to_lowercase().contains(&needle))
                    .collect();
                if shown.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(format!("{cat} ({})", shown.len()))
                    .default_open(!needle.is_empty())
                    .show(ui, |ui| {
                        for i in shown {
                            let s = &ed.pack.sliders[i];
                            ui.horizontal(|ui| {
                                let changed = ui
                                    .add(egui::Slider::new(&mut ed.weights[i], s.min..=s.max).text(&s.name))
                                    .changed();
                                if (ed.weights[i] - s.base).abs() > 1e-6 && ui.small_button("↺").clicked() {
                                    ed.weights[i] = s.base;
                                    ed.dirty = true;
                                }
                                ed.dirty |= changed;
                            });
                        }
                    });
            }
        });
    });
}

fn apply_shape(
    mut editor: ResMut<Editor>,
    rig: Res<Rig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut ibms: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut transforms: Query<&mut Transform>,
    hair: Option<Res<HairLib>>,
) {
    if !rig.ready || !(editor.dirty || editor.expr_dirty) {
        return;
    }
    let ed = &mut *editor;
    ed.expr_dirty = false;
    let mut dworld = None;
    if std::mem::take(&mut ed.dirty) {
        let delta = ed.pack.evaluate(&ed.weights);
        dworld = Some(apply_joints(&rig, &delta, &mut ibms, &mut transforms));
        ed.delta = delta;
    }

    // Vertex delta = shape + expressions, layered here so both stay consistent
    // (the expression pack shares the slider pack's global vertex space and frame).
    let expr_w: Vec<(usize, f32)> =
        ed.expr_weights.iter().enumerate().filter(|(_, w)| **w != 0.0).map(|(i, w)| (i, *w)).collect();
    let layered;
    let verts = match &ed.expr {
        Some(x) if !expr_w.is_empty() => {
            let mut v = ed.delta.verts.clone();
            x.add_to(&expr_w, &mut v);
            layered = v;
            &layered
        }
        _ => &ed.delta.verts,
    };
    if let Some(h) = &hair {
        hair_shape(h, ed, verts, dworld.as_deref(), &rig, &mut meshes, &mut ibms);
    }
    let hd_verts;
    let verts = match &ed.hd {
        Some(h) => {
            hd_verts = h.expand_verts(verts);
            &hd_verts
        }
        None => verts,
    };
    for m in &rig.meshes {
        let Some(mesh) = meshes.get_mut(&m.handle) else { continue };
        let pos: Vec<[f32; 3]> = m
            .base
            .iter()
            .zip(&m.global)
            .map(|(b, &g)| (Vec3::from(*b) + m.to_mesh * Vec3::from(verts[g as usize])).into())
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    }
}

/// Joint translations and inverse bind matrices for a shape delta; returns each
/// rig joint's world translation delta.
fn apply_joints(
    rig: &Rig,
    delta: &ShapeDelta,
    ibms: &mut Assets<SkinnedMeshInverseBindposes>,
    transforms: &mut Query<&mut Transform>,
) -> Vec<Vec3> {
    // World delta per joint: its own, else its parent's (helpers ride along).
    let mut dworld = vec![Vec3::ZERO; rig.joints.len()];
    let mut done = vec![false; rig.joints.len()];
    fn resolve(i: usize, rig: &Rig, delta: &ShapeDelta, dworld: &mut [Vec3], done: &mut [bool]) -> Vec3 {
        if !done[i] {
            dworld[i] = match (rig.joints[i].pack, rig.joints[i].parent) {
                (Some(p), _) => Vec3::from(delta.joints[p]),
                (None, Some(par)) => resolve(par, rig, delta, dworld, done),
                (None, None) => Vec3::ZERO,
            };
            done[i] = true;
        }
        dworld[i]
    }
    for i in 0..rig.joints.len() {
        resolve(i, rig, delta, &mut dworld, &mut done);
    }
    for (i, j) in rig.joints.iter().enumerate() {
        let dp = j.parent.map_or(Vec3::ZERO, |p| dworld[p]);
        if let Ok(mut t) = transforms.get_mut(j.entity) {
            t.translation = j.base_local + j.parent_lin_inv * (dworld[i] - dp);
        }
    }
    update_skins(rig, &rig.skins, &dworld, ibms);
    dworld
}

/// Inverse bind matrices of skins on our joints for the joints' world deltas, so
/// rewritten bind meshes render as written: `IBM' = (T(Δ)·W)⁻¹ · W · IBM`.
fn update_skins(rig: &Rig, skins: &[SkinRec], dworld: &[Vec3], ibms: &mut Assets<SkinnedMeshInverseBindposes>) {
    for skin in skins {
        let new: Vec<Mat4> = skin
            .joints
            .iter()
            .zip(&skin.base)
            .map(|(&ji, ibm)| {
                let w = Mat4::from(rig.joints[ji].base_world);
                (Mat4::from_translation(dworld[ji]) * w).inverse() * w * *ibm
            })
            .collect();
        if let Some(asset) = ibms.get_mut(&skin.handle) {
            *asset = SkinnedMeshInverseBindposes::from(new);
        }
    }
}
