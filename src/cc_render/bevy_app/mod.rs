//! Bevy 0.15 app driving the CC5 avatar renderer.

pub(super) mod assets;
pub(super) mod camera;
pub(super) mod capture;
pub(super) mod gpu_select;
pub(super) mod joint_debug;
pub(super) mod overrides;
pub(super) mod pose;

pub(super) mod physics;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

use bevy::{
    app::{Animation as AnimationSet, AppExit, ScheduleRunnerPlugin},
    asset::{
        io::{AssetSource, AssetSourceId},
        AssetApp, AssetPlugin,
    },
    log::LogPlugin,
    pbr::DirectionalLightShadowMap,
    prelude::*,
    render::{
        extract_resource::ExtractResourcePlugin,
        settings::{InstanceFlags, WgpuSettings},
        Render, RenderApp, RenderPlugin, RenderSet,
    },
    transform::TransformSystem,
    window::ExitCondition,
    winit::WinitPlugin,
};

use super::renderer::{ArkitPose, RenderedFrame, RendererConfig, SkeletalPose};

use super::renderer::{GrabOrRelease, PropCmd, PropId, SceneState};

use assets::{load_arkit_map, parse_material_names_from_glb, ArkitMapping, MaterialNames};
use camera::{
    auto_orient_avatar, fit_camera_to_avatar, follow_avatar_camera, setup_scene,
    strip_scene_lights, AvatarRoot, GlbAsset,
};
use capture::{
    copy_render_target_to_cpu, CaptureChannelTx, CapturedFrame, LastAppliedPts,
    MonotonicCaptureClock,
};
use joint_debug::{
    configure_gizmo_overlay, draw_joint_skeleton, spawn_joint_labels, update_joint_labels,
    JointDebugConfig, JointLabelsSpawned,
};
use pose::{
    apply_arkit_face_bones, apply_arkit_pose, apply_skeletal_pose, capture_cc5_bind_rotations,
    capture_face_bones, inspect_morph_pipeline, start_pose_animation, CC5BindRotations, FaceBones,
    PoseAnimation, PoseWatchRx, SkeletalPoseWatchRx,
};

/// Holds the shutdown receiver so a Bevy system can ask the app to
/// exit when the parent `CcRenderer` is dropped.
#[derive(Resource)]
struct ShutdownRx(crossbeam_channel::Receiver<()>);

fn check_shutdown(rx: Res<ShutdownRx>, mut exit: EventWriter<AppExit>) {
    if rx.0.try_recv().is_ok() {
        exit.send(AppExit::Success);
    }
}

/// Cross-thread readiness flag flipped to `true` once the GLB scene
/// has settled enough to start writing the MP4: bind-pose capture
/// done + a small margin of frames rendered (= textures/materials are
/// uploaded + visible). Read from the smoke binary via
/// `CcRenderer::is_ready()`.
#[derive(Resource, Clone)]
struct ReadySignal(Arc<AtomicBool>);

/// Counts settled frames once ALL load conditions are simultaneously
/// true. Reset to 0 if any condition becomes false again (e.g. another
/// SceneSpawner pass adds entities).
#[derive(Resource, Default)]
struct ReadyMonitor {
    settled_frames: u32,
    last_mesh_count: usize,
    last_material_count: usize,
    stable_count_ticks: u32,
    fired: bool,
}

/// Frames to hold AFTER all gates are satisfied — gives wgpu time to
/// finish uploading textures + run a few full-material renders so the
/// first emitted MP4 frame has a visibly-shaded avatar instead of a
/// half-loaded shimmering one.
const READY_SETTLE_FRAMES: u32 = 60;
/// Mesh/material entity count must stay stable for this many ticks
/// before we trust SceneSpawner is fully done populating the scene.
const READY_STABLE_TICKS: u32 = 8;

#[allow(clippy::too_many_arguments)]
fn mark_ready_when_settled(
    bind: Res<pose::CC5BindRotations>,
    asset_server: Res<AssetServer>,
    avatar_scene_q: Query<&SceneRoot, With<AvatarRoot>>,
    mesh_q: Query<&Mesh3d>,
    material_q: Query<&MeshMaterial3d<StandardMaterial>>,
    signal: Res<ReadySignal>,
    mut monitor: ResMut<ReadyMonitor>,
    mut log_throttle: Local<u32>,
) {
    if monitor.fired {
        return;
    }

    // Gate 1: bind capture done (bones found, scene at least
    // partially spawned).
    if !bind.captured {
        monitor.settled_frames = 0;
        return;
    }

    // Gate 2: the avatar's SceneRoot has fully loaded its transitive
    // dep tree (every Mesh, Image, Material, morph-target). We only
    // gate on the avatar — an optional environment scene can keep
    // streaming textures in the background without blocking
    // readiness, since the avatar is what the consumer cares about.
    let mut scene_seen = 0_usize;
    let mut deps_loaded = true;
    for scene_root in avatar_scene_q.iter() {
        scene_seen += 1;
        match asset_server.recursive_dependency_load_state(scene_root.0.id()) {
            bevy::asset::RecursiveDependencyLoadState::Loaded => {}
            _ => {
                deps_loaded = false;
                break;
            }
        }
    }
    if scene_seen == 0 || !deps_loaded {
        monitor.settled_frames = 0;
        // Periodic log so a stuck load is visible.
        *log_throttle += 1;
        if *log_throttle % 60 == 0 {
            tracing::info!(
                target: "cc_render",
                "ready: waiting on avatar scene deps (avatars={} deps_loaded={})",
                scene_seen, deps_loaded,
            );
        }
        return;
    }

    // Gate 3: SceneSpawner has stopped adding entities. We watch
    // mesh + material counts and only proceed once they've stayed
    // constant for `READY_STABLE_TICKS` ticks. Catches the case where
    // a GLB's children spawn over multiple frames.
    let mesh_count = mesh_q.iter().count();
    let material_count = material_q.iter().count();
    if mesh_count != monitor.last_mesh_count || material_count != monitor.last_material_count {
        monitor.last_mesh_count = mesh_count;
        monitor.last_material_count = material_count;
        monitor.stable_count_ticks = 0;
        monitor.settled_frames = 0;
        return;
    }
    monitor.stable_count_ticks = monitor.stable_count_ticks.saturating_add(1);
    if monitor.stable_count_ticks < READY_STABLE_TICKS {
        return;
    }

    // Gate 4: small final settle so wgpu finishes its texture upload
    // queue and we've rendered at least N fully-shaded frames.
    monitor.settled_frames = monitor.settled_frames.saturating_add(1);
    if monitor.settled_frames >= READY_SETTLE_FRAMES {
        signal.0.store(true, Ordering::Release);
        monitor.fired = true;
        tracing::info!(
            target: "cc_render",
            "ready: scene fully loaded — meshes={} materials={} settle_frames={} \
             — driver may start recording",
            mesh_count, material_count, monitor.settled_frames,
        );
    }
}

/// Bridges from the capture system's internal `CapturedFrame` channel
/// to the public `RenderedFrame` consumed by `CcRenderer`. We keep a
/// dedicated bridge channel because the RenderApp side speaks only
/// `crossbeam_channel`, and we don't want it to depend on the
/// public renderer types.
#[derive(Resource)]
struct FrameBridgeRx(crossbeam_channel::Receiver<CapturedFrame>);

fn forward_frames(rx: Res<FrameBridgeRx>, sink: Res<FrameSinkTx>, mut count: Local<u64>) {
    while let Ok(c) = rx.0.try_recv() {
        *count += 1;

        // Frame brightness sample: average of channel-0 byte across the
        // first 64 pixels. Helps distinguish "all-zero" (real black) from
        // "near-clear-color" (charcoal) from "actual content rendered".
        let sample_n = 64.min(c.pixels.len() / 4);
        let mut sum_r: u64 = 0;
        let mut sum_g: u64 = 0;
        let mut sum_b: u64 = 0;
        for i in 0..sample_n {
            sum_r += c.pixels[i * 4] as u64;
            sum_g += c.pixels[i * 4 + 1] as u64;
            sum_b += c.pixels[i * 4 + 2] as u64;
        }
        let avg_r = sum_r / sample_n.max(1) as u64;
        let avg_g = sum_g / sample_n.max(1) as u64;
        let avg_b = sum_b / sample_n.max(1) as u64;

        if *count == 1 || *count % 30 == 0 {
            tracing::debug!(
                target: "cc_render",
                "forward_frames: frame #{} ({}×{}, {} bytes, pts_ms={}) avg_rgb=({},{},{})",
                *count,
                c.width,
                c.height,
                c.pixels.len(),
                c.pts_ms,
                avg_r, avg_g, avg_b,
            );
        }

        // Dump frames at well-defined milestones so the user can inspect
        // what Bevy is actually rendering. Uses /tmp/cc_render_debug_*.png.
        // Spaced to cover both pre-asset-load (clear color) and
        // post-asset-load (real scene) — assistant.alphabaked.glb
        // is ~130MB and SceneSpawner usually settles by frame ~600 at 30fps.
        if matches!(
            *count,
            1 | 30 | 120 | 300 | 600 | 900 | 1200 | 1800 | 2400 | 3000
        ) {
            let path = format!("/tmp/cc_render_debug_frame_{:04}.png", *count);
            match image::RgbaImage::from_raw(c.width, c.height, c.pixels.clone()) {
                Some(img) => match img.save(&path) {
                    Ok(_) => tracing::debug!(
                        target: "cc_render",
                        "wrote debug frame snapshot: {}",
                        path
                    ),
                    Err(e) => tracing::warn!(
                        target: "cc_render",
                        "save debug frame {}: {e}",
                        path
                    ),
                },
                None => tracing::warn!(
                    target: "cc_render",
                    "frame #{} pixel buffer not RGBA-shaped ({} bytes for {}x{})",
                    *count,
                    c.pixels.len(),
                    c.width,
                    c.height
                ),
            }
        }

        let _ = sink.0.try_send(RenderedFrame {
            pixels: c.pixels,
            width: c.width,
            height: c.height,
            pts_ms: c.pts_ms,
        });
    }
}

#[derive(Resource)]
struct FrameSinkTx(crossbeam_channel::Sender<RenderedFrame>);

/// Build + run the Bevy app on the calling (dedicated) thread. Returns
/// when the app exits (either via shutdown signal or fatal error).
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    config: Arc<RendererConfig>,
    pose_rx: watch::Receiver<ArkitPose>,
    skeletal_pose_rx: crossbeam_channel::Receiver<Option<SkeletalPose>>,
    frame_tx: crossbeam_channel::Sender<RenderedFrame>,
    shutdown_rx: crossbeam_channel::Receiver<()>,
    ready_signal: Arc<AtomicBool>,
    prop_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<PropCmd>,
    despawn_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<PropId>,
    grab_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<GrabOrRelease>,
    scene_state_tx: tokio::sync::watch::Sender<SceneState>,
) {
    let glb_path = match config.glb_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(
                "cc_render: canonicalize glb {:?} failed: {e}",
                config.glb_path
            );
            return;
        }
    };
    let glb_dir = match glb_path.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            tracing::error!("cc_render: glb has no parent: {:?}", glb_path);
            return;
        }
    };
    let glb_filename = match glb_path.file_name() {
        Some(n) => n.to_string_lossy().to_string(),
        None => {
            tracing::error!("cc_render: glb has no filename: {:?}", glb_path);
            return;
        }
    };

    // Resolve the optional scene GLB. We canonicalize so a relative
    // path passed in via `RendererConfig::scene_glb_path` works from
    // any working directory, then split into (parent_dir, filename).
    // The parent_dir backs a separate `scene://` AssetSource so the
    // scene can live independently of the avatar's directory.
    let scene_dir_and_name: Option<(PathBuf, String)> =
        config.scene_glb_path.as_ref().and_then(|p| {
            let canonical = match p.canonicalize() {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(
                        target: "cc_render",
                        "canonicalize scene glb {:?} failed: {e} — scene will be skipped",
                        p,
                    );
                    return None;
                }
            };
            let dir = canonical.parent()?.to_path_buf();
            let name = canonical.file_name()?.to_string_lossy().to_string();
            Some((dir, name))
        });

    let map = match load_arkit_map(&config.arkit_map_path) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("cc_render: load arkit map: {e}");
            return;
        }
    };

    // RenderApp → main App readback channel.
    let (cap_tx, cap_rx) = crossbeam_channel::unbounded::<CapturedFrame>();

    // Tick rate from config.fps. Bevy uses ScheduleRunnerPlugin to
    // pace itself when there's no winit window driving the loop.
    //
    // `CC_RENDER_FAST=1` (offline batch render) drops the wait to
    // ZERO so the app ticks as fast as the GPU can render. Combined
    // with the lockstep streaming loop in the smoke binary, this
    // collapses a 5s-of-motion render from ~25s wall to ~3s wall
    // (1280×1280 on Mesa-WSL2). NEVER set this for realtime stream
    // pipelines — Bevy will spin a CPU core flat-out.
    let tick = if std::env::var("CC_RENDER_FAST").ok().as_deref() == Some("1") {
        tracing::info!(
            target: "cc_render",
            "fast-render: ScheduleRunnerPlugin tick=0 (uncapped fps)"
        );
        Duration::ZERO
    } else {
        Duration::from_secs_f64(1.0 / config.fps.max(1) as f64)
    };

    let mut app = App::new();

    // Register the `env://` asset source so KTX2 environment maps load
    // from a fixed location regardless of where the GLB sits. Resolution
    // order:
    //   1. CC_AVATAR_ENVMAP_DIR env var (absolute or repo-relative)
    //   2. <repo>/avatars/env (working-directory walk-up)
    //   3. <glb_dir>/env (legacy default; matches the old behavior)
    // Must run before `add_plugins(AssetPlugin)` — registered sources are
    // built when AssetPlugin initializes and cannot be added afterward.
    let env_dir = resolve_envmap_dir(&glb_dir);
    tracing::info!(
        target: "cc_render",
        "asset source 'env://' → {}",
        env_dir.display()
    );
    app.register_asset_source(
        AssetSourceId::from_static("env"),
        AssetSource::build().with_reader(AssetSource::get_default_reader(
            env_dir.to_string_lossy().into_owned(),
        )),
    );

    // Register `scene://` if a separate scene GLB has been configured.
    // Pointing at the scene's parent dir lets `setup_scene` load it via
    // `scene://{filename}#Scene0` regardless of where the avatar lives.
    if let Some((scene_dir, _)) = scene_dir_and_name.as_ref() {
        tracing::info!(
            target: "cc_render",
            "asset source 'scene://' → {}",
            scene_dir.display()
        );
        app.register_asset_source(
            AssetSourceId::from_static("scene"),
            AssetSource::build().with_reader(AssetSource::get_default_reader(
                scene_dir.to_string_lossy().into_owned(),
            )),
        );
    }

    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: glb_dir.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
            })
            .set(LogPlugin {
                // `bevy_animation=error` suppresses the per-track
                // "ComponentNotPresent" warnings: the FBX-exported pose
                // animation (`FBXExportClip_0`) has 172 channels — many
                // target room props (Chair, Sofa, …) or morph-weight
                // tracks on meshes whose target entity doesn't carry a
                // `MorphWeights` component. Bevy warns + skips them
                // (non-fatal); we just don't want the noise.
                filter: "warn,cc_render=info,bevy_render=warn,wgpu=warn,bevy_app=info,\
                     bevy_animation=error"
                    .into(),
                ..default()
            })
            .set(RenderPlugin {
                // `gpu_select::build_render_creation` returns
                // `RenderCreation::Automatic(default_settings)` unless
                // `AVATAR_BEVY_GPU_INDEX=N` is set, in which case it
                // hand-picks the Nth wgpu adapter so Bevy doesn't end
                // up on the same physical GPU as Windows-side encoders
                // (Snipping Tool / NVENC) on a multi-GPU host.
                render_creation: gpu_select::build_render_creation(WgpuSettings {
                    instance_flags: InstanceFlags::default()
                        | InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER,
                    ..default()
                }),
                ..default()
            })
            .disable::<WinitPlugin>(),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(tick))
    .add_plugins(ExtractResourcePlugin::<capture::CaptureTarget>::default())
    .add_plugins(ExtractResourcePlugin::<LastAppliedPts>::default())
    .insert_resource(DirectionalLightShadowMap { size: 4096 })
    .insert_resource(GlbAsset {
        filename: glb_filename,
        width: config.width,
        height: config.height,
        scene_filename: scene_dir_and_name.as_ref().map(|(_, n)| n.clone()),
    })
    .insert_resource(MaterialNames(parse_material_names_from_glb(&glb_path)))
    .insert_resource(ArkitMapping { map: Arc::new(map) })
    .insert_resource(PoseWatchRx(pose_rx))
    .insert_resource(SkeletalPoseWatchRx(skeletal_pose_rx))
    .insert_resource(LastAppliedPts(0))
    .insert_resource(FrameBridgeRx(cap_rx))
    .insert_resource(FrameSinkTx(frame_tx))
    .insert_resource(ShutdownRx(shutdown_rx))
    .insert_resource(ReadySignal(ready_signal))
    .init_resource::<ReadyMonitor>()
    .init_resource::<PoseAnimation>()
    .init_resource::<CC5BindRotations>()
    .init_resource::<FaceBones>()
    .insert_resource(JointDebugConfig::from_env())
    .init_resource::<JointLabelsSpawned>()
    .add_systems(Startup, (setup_scene, configure_gizmo_overlay))
    .add_systems(
        Update,
        (
            apply_arkit_pose,
            overrides::override_hair_alpha,
            start_pose_animation,
            capture_cc5_bind_rotations,
            // Face-bone capture: cache jaw + eye bone entities + their
            // bind-pose rotations once. Cheap, gated on captured flag.
            capture_face_bones,
            // Runs after capture so it can read pelvis_world / eye_axis_world.
            auto_orient_avatar.after(capture_cc5_bind_rotations),
            // Fit camera to avatar AABB (gated by CC_AVATAR_FIT_FRAME).
            // Runs AFTER auto_orient so the orbit doesn't fight the fit.
            fit_camera_to_avatar.after(auto_orient_avatar),
            // Optional: strip authored lights from the environment
            // scene so they don't layer on top of the portrait rig.
            // No-op unless `CC_SCENE_KEEP_LIGHTS=0`.
            strip_scene_lights,
            forward_frames,
            check_shutdown,
            // Joint-debug overlay: spawn labels lazily after bind capture.
            spawn_joint_labels.after(capture_cc5_bind_rotations),
            // Flip the cross-thread readiness flag once bind capture
            // has completed and the scene has had a settle margin.
            mark_ready_when_settled.after(capture_cc5_bind_rotations),
        ),
    )
    // Skeletal-pose override needs explicit ordering: AFTER the
    // bevy_animation `Animation` set (so we override the animation's
    // last-evaluated pose, even when paused) and BEFORE
    // `TransformSystem::TransformPropagate` (so our writes reach
    // GlobalTransform and thus the GPU this frame). This mirrors the
    // proven pattern in examples/avatar-render-proto.
    .add_systems(
        PostUpdate,
        apply_skeletal_pose
            .after(AnimationSet)
            .before(TransformSystem::TransformPropagate),
    )
    // Face bones (jaw + eyes) — same scheduling slot as
    // apply_skeletal_pose so the rotations reach GlobalTransform this
    // frame, but explicitly AFTER skeletal so the body-bone writes
    // don't overwrite the head/jaw/eye chain. Reads PoseWatchRx
    // (latest-wins watch) so it sees the freshest A2F weights even
    // when Bevy renders faster than the gate paces them.
    .add_systems(
        PostUpdate,
        apply_arkit_face_bones
            .after(apply_skeletal_pose)
            .before(TransformSystem::TransformPropagate),
    )
    // Diagnostic: read MeshMorphWeights AFTER Bevy's `inherit_weights`
    // (PostUpdate) propagates parent MorphWeights to child entities, so
    // we see exactly what's about to hit the GPU. Logs at ~1 Hz.
    .add_systems(PostUpdate, inspect_morph_pipeline)
    // Camera-follow runs in PostUpdate before TransformPropagate so
    // its Transform write reaches GlobalTransform this same frame.
    // Reads pelvis GlobalTransform from the PREVIOUS frame
    // (acceptable 1-frame lag); gated by CC_AVATAR_FOLLOW=1.
    .add_systems(
        PostUpdate,
        follow_avatar_camera
            .after(apply_skeletal_pose)
            .before(TransformSystem::TransformPropagate),
    )
    // Joint-debug skeleton + labels read GlobalTransforms — must run
    // AFTER `TransformPropagate` so the freshly-applied pose is
    // visible. No-op when `CC_AVATAR_DEBUG_JOINTS` != "1".
    .add_systems(
        PostUpdate,
        (draw_joint_skeleton, update_joint_labels).after(TransformSystem::TransformPropagate),
    );

    if let Some(phys) = config.physics.clone() {
        physics::install(&mut app, phys, config.glb_path.clone());
        // mpsc::UnboundedReceiver is !Sync; Bevy `Resource` requires
        // Sync, so wrap in Mutex. The polling system locks once per
        // tick, which is cheap.
        app.insert_resource(physics::props::PropCmdRx(std::sync::Mutex::new(
            prop_cmd_rx,
        )));
        app.insert_resource(physics::props::DespawnCmdRx(std::sync::Mutex::new(
            despawn_cmd_rx,
        )));
        app.insert_resource(physics::props::GrabCmdRx(std::sync::Mutex::new(
            grab_cmd_rx,
        )));
        app.insert_resource(physics::props::SceneStateTx(scene_state_tx));
    } else {
        // Physics disabled (e.g. CC_PHYSICS_DISABLE=1) — drop the
        // channels so the renderer's `Sender`s don't block expecting a
        // receiver. The cdylib still ships rapier in the binary; the
        // user just opted out at runtime.
        drop((prop_cmd_rx, despawn_cmd_rx, grab_cmd_rx, scene_state_tx));
    }

    // RenderApp side: install texture-to-buffer copy + sender.
    let render_app = app.sub_app_mut(RenderApp);
    render_app.insert_resource(CaptureChannelTx { tx: cap_tx });
    render_app.insert_resource(MonotonicCaptureClock {
        frame_count: 0,
        frame_interval_ms: 1000_u64 / config.fps.max(1) as u64,
    });
    render_app.add_systems(Render, copy_render_target_to_cpu.in_set(RenderSet::Cleanup));

    app.run();
}

/// Resolve the directory holding KTX2 environment maps (`pisa_diffuse…`,
/// `pisa_specular…`). The renderer registers this as the `env://` asset
/// source so loads work no matter where the GLB sits on disk.
///
/// Order:
///   1. `CC_AVATAR_ENVMAP_DIR` env var
///   2. Walk up from the current working directory looking for
///      `avatars/env/`
///   3. Fall back to `<glb_dir>/env`
fn resolve_envmap_dir(glb_dir: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("CC_AVATAR_ENVMAP_DIR") {
        let pb = PathBuf::from(p);
        if pb.is_dir() {
            return pb;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut cur = cwd.as_path();
        loop {
            let candidate = cur.join("avatars").join("env");
            if candidate.is_dir() {
                return candidate;
            }
            match cur.parent() {
                Some(p) => cur = p,
                None => break,
            }
        }
    }
    glb_dir.join("env")
}

/// Re-export for the test/diagnostic path that wants to verify the
/// asset paths exist before kicking the renderer.
#[allow(dead_code)]
pub(super) fn validate_asset_paths(glb: &Path, map: &Path) -> anyhow::Result<()> {
    if !glb.exists() {
        anyhow::bail!("GLB not found: {:?}", glb);
    }
    if !map.exists() {
        anyhow::bail!("ARKit map not found: {:?}", map);
    }
    let _ = PathBuf::from(glb);
    Ok(())
}
