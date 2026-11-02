//! `CcRenderer` — owns the dedicated Bevy thread + the inbound pose
//! / outbound frame channels. The streaming-node side
//! ([`super::node::CcRenderNode`]) drives this purely via channels;
//! it never touches Bevy types directly.
//!
//! Mirrors the role of
//! [`crate::nodes::live2d_render::render_worker::RenderWorker`]: a
//! single OS thread owns the !Send GPU/scene state, async callers
//! talk to it via channels.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use tokio::sync::watch;

/// One ARKit-52 weight set (channel name → [0,1] activation).
#[derive(Debug, Clone, Default)]
pub struct ArkitPose {
    pub weights: std::collections::HashMap<String, f32>,
    /// Source-side pts in milliseconds, propagated back onto the
    /// emitted `RuntimeData::Video` so downstream WebRTC can stamp
    /// frames against the audio clock the same way `live2d_render`
    /// does.
    pub pts_ms: u64,
}

/// One frame of SMPL-22 skeletal pose, ordered per `SMPL_22_NAMES` in
/// `scripts/avatars/kimodo_gen.py`: Pelvis, L_Hip, R_Hip, Spine1,
/// L_Knee, R_Knee, Spine2, L_Ankle, R_Ankle, Spine3, L_Foot, R_Foot,
/// Neck, L_Collar, R_Collar, Head, L_Shoulder, R_Shoulder, L_Elbow,
/// R_Elbow, L_Wrist, R_Wrist.
///
/// Each quaternion is parent-local (xyzw → bevy::Quat::from_xyzw),
/// matching the convention written by Kimodo and consumed by our
/// retargeter. Root translation is intentionally NOT plumbed in v1 —
/// rotations alone produce visually correct wave/sit/point/dance.
#[derive(Debug, Clone)]
pub struct SkeletalPose {
    pub stream: Option<super::office::timeline::StreamIdentity>,
    /// 22 parent-local quaternions in SMPL_22_NAMES order.
    pub joint_quats: [[f32; 4]; 22],
    /// World-space pelvis position (Y-up, meters), as emitted by Kimodo.
    /// The Bevy side anchors against the first pose seen of each
    /// streaming session and applies `root_pos - anchor` as a
    /// parent-local translation on the pelvis bone — same logic as the
    /// offline retargeter.
    pub root_pos: [f32; 3],
    /// Source-side pts in ms; same role as `ArkitPose::pts_ms`.
    pub pts_ms: u64,
}

impl SkeletalPose {
    /// Validate and normalize an external motion envelope before retargeting.
    pub(crate) fn normalized(mut self) -> Option<Self> {
        if self.root_pos.iter().any(|x| !x.is_finite()) {
            return None;
        }
        for q in &mut self.joint_quats {
            let norm = q.iter().map(|x| x * x).sum::<f32>().sqrt();
            if !norm.is_finite() || norm < 1e-4 {
                return None;
            }
            for value in q {
                *value /= norm;
            }
        }
        Some(self)
    }

    /// Identity-rotation pose (T-pose). Used as the watch channel's
    /// initial value before any real pose arrives.
    pub fn rest() -> Self {
        Self {
            joint_quats: [[0.0, 0.0, 0.0, 1.0]; 22],
            stream: None,
            root_pos: [0.0, 0.0, 0.0],
            pts_ms: 0,
        }
    }
}

impl Default for SkeletalPose {
    fn default() -> Self {
        Self::rest()
    }
}

/// One rendered RGBA frame (tightly packed, no row padding).
pub struct RenderedFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Echoes the `ArkitPose::pts_ms` of the pose this frame was
    /// rendered against. The Bevy app stores the most-recent applied
    /// pts and stamps each frame with it.
    pub pts_ms: u64,
}

#[derive(Clone, Debug)]
pub struct RendererConfig {
    pub glb_path: PathBuf,
    pub arkit_map_path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Target render rate. The Bevy app uses this to pace its
    /// `ScheduleRunnerPlugin` — the proto runs at 60Hz but we'd
    /// typically want 30Hz here to match the WebRTC video stream.
    pub fps: u32,
    /// Optional environment scene GLB loaded alongside the avatar.
    /// When set, the Bevy app spawns a second `SceneRoot` for it via
    /// the `scene://` asset source so the scene file can live in any
    /// directory (no need to colocate it with the avatar GLB).
    ///
    /// **Runtime tunables (env vars, all optional):**
    /// - `CC_SCENE_POS=x,y,z` — translation applied to the scene's
    ///   `SceneRoot` (default `0,0,0`). Avatar transform is unaffected.
    /// - `CC_SCENE_ROT_DEG_Y=<deg>` — rotation around Y (default `0`).
    /// - `CC_SCENE_SCALE=<f>` — uniform scale (default `1.0`).
    /// - `CC_SCENE_KEEP_LIGHTS=0` — strip authored
    ///   `DirectionalLight` / `PointLight` / `SpotLight` components
    ///   from the scene as Bevy spawns it, so the renderer's 3-light
    ///   portrait rig is the only illumination source. Defaults to
    ///   keeping scene lights.
    pub scene_glb_path: Option<PathBuf>,
    /// Show the environment map (`env://pisa_specular…`, the map that
    /// already lights the avatar) as the background instead of the flat
    /// `CC_AVATAR_CLEAR_COLOR` (or the studio backdrop).
    pub env_background: bool,
    /// Its brightness in cd/m². `None`: the map's own values at the
    /// camera's exposure ([`env_background_brightness`]).
    pub env_background_brightness: Option<f32>,
    /// Render materials that carry CC5 shader data (`extras.cc5` + the maps
    /// in `<name>.cc5maps/`, see `cc5_materials`) with cc-render's CC5
    /// skin / hair / eye / mouth shaders instead of glTF PBR, as the avatar
    /// editor does. The camera then anti-aliases with TAA instead of MSAA:
    /// the CC5 hair shader's dithered strand coverage needs it. GLBs
    /// without `extras.cc5` render as before.
    pub cc5_materials: bool,
    /// Light and grade like the avatar editor (`cc_render::studio`, calibrated
    /// against CC5's viewport): its portrait light rig, exposure, IBL level,
    /// SSAO, TAA, sharpening and colour grading, and its grey-blue studio
    /// backdrop (unless `env_background`). The camera keeps its framing and
    /// field of view (`CC_AVATAR_CAMERA_FOV_DEG=17.06` matches the editor's
    /// lens). Calibrated with `cc5_materials` (the CC5 shaders). On by
    /// default; `false` restores the renderer's earlier 3-light rig, AgX
    /// camera and flat `CC_AVATAR_CLEAR_COLOR` background.
    pub studio_lighting: bool,

    pub physics: Option<PhysicsConfig>,
    /// Where frames go: back to the node (`Capture`, the default) or onto
    /// the desktop (`Overlay`, see [`super::overlay`]).
    pub present: Present,
    /// A cost preset for the studio look; `None` keeps the full look
    /// (and `CC_AVATAR_MAX_TEXTURE` alone caps textures).
    pub quality: Option<RenderQuality>,
}

/// Where the rendered frames go.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Present {
    /// Read back and returned through [`CcRenderer::drain_frames`] (the
    /// node emits them as `RuntimeData::Video`).
    #[default]
    Capture,
    /// Drawn on top of the desktop; nothing is returned to the node.
    Overlay(super::overlay::OverlayConfig),
    /// Off-line, for [`super::offline::OfflineRenderer`]: a transparent
    /// background, a fixed `1/fps` time step, and one frame per pose,
    /// rendered as fast as the GPU goes.
    Offline,
}

/// How much of the studio look to render: most of the renderer's GPU time
/// and memory is here, not in getting pixels to the screen.
///
/// | | low | medium | high |
/// |---|---|---|---|
/// | shadows | off | 1024² | 4096² |
/// | SSAO, bloom, sharpening | off | off | on |
/// | anti-aliasing | MSAA 4× | MSAA 4× | TAA |
/// | textures (px per side) | 512 | 1024 | full |
/// | hair / body physics | off | off | on |
///
/// The CC5 shaders (`cc5_materials`) are independent of the preset; their
/// hair's dithered strands need TAA, so with them every preset uses TAA.
/// `CC_AVATAR_MAX_TEXTURE` still overrides the texture cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderQuality {
    Low,
    Medium,
    High,
}

impl RenderQuality {
    /// Shadow map side, or `None` for no shadows.
    pub fn shadow_map(self) -> Option<usize> {
        match self {
            Self::Low => None,
            Self::Medium => Some(1024),
            Self::High => Some(4096),
        }
    }
    /// TAA, SSAO, bloom and sharpening (else MSAA 4×).
    pub fn post_effects(self) -> bool {
        self == Self::High
    }
    pub fn max_texture(self) -> Option<u32> {
        match self {
            Self::Low => Some(512),
            Self::Medium => Some(1024),
            Self::High => None,
        }
    }
    pub fn physics(self) -> bool {
        self == Self::High
    }
}

impl std::str::FromStr for RenderQuality {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            _ => Err(format!("{s}: expected low, medium or high")),
        }
    }
}

/// Default background brightness for [`RendererConfig::env_background`]: the
/// map's texels shown at their own value under exposure `ev100` (Bevy's
/// exposure is 1 / (1.2 * 2^ev100)).
pub fn env_background_brightness(ev100: f32) -> f32 {
    1.2 * 2f32.powf(ev100)
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            glb_path: PathBuf::new(),
            arkit_map_path: PathBuf::new(),
            width: 1280,
            height: 1280,
            fps: 30,
            scene_glb_path: None,
            env_background: false,
            env_background_brightness: None,
            cc5_materials: false,
            studio_lighting: true,

            physics: None,
            present: Present::Capture,
            quality: None,
        }
    }
}

/// Abstraction over the renderer so tests can inject a mock without
/// spawning the heavy Bevy thread + GPU device. Production code uses
/// [`CcRenderer`]; unit tests construct a fake that records pushed
/// poses and emits scripted frames.
pub trait Renderer: Send + Sync {
    fn push_pose(&self, pose: ArkitPose);
    fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }
    /// Push a full-body skeletal pose. `None` clears any active stream
    /// and lets the avatar's baked AnimationPlayer take over again.
    fn push_skeletal_pose(&self, pose: Option<SkeletalPose>);
    fn drain_frames(&self, sink: &mut Vec<RenderedFrame>);
    /// Has the underlying scene finished loading? `true` means GLB +
    /// bind capture + a settle margin of frames have all completed,
    /// so subsequent `drain_frames` calls return frames containing
    /// the actual avatar (vs. clear-color pre-roll).
    ///
    /// Default impl returns `true` so mocks (which emit scripted
    /// frames synchronously) and any future renderer that doesn't
    /// have an async warmup don't have to implement this.
    fn is_ready(&self) -> bool {
        true
    }
    /// Skeletal-pose queue depth — how many enqueued poses Bevy hasn't
    /// drained yet. Used by `CcRenderNode::tick` to keep emitting V+A
    /// pairs (with silence-padded audio when speech is done) until the
    /// motion has fully played through, regardless of when the last
    /// `push_skeletal_pose` happened. Default 0 for mocks that don't
    /// queue.
    fn skeletal_queue_depth(&self) -> usize {
        0
    }
    fn office_command(&self, _cmd: super::office::controller::OfficeCommand) {}
    /// Display-only overlay state; `None` hides the panel.
    fn home_hud(&self, _hud: Option<super::office::home_hud::HomeHud>) {}
    /// Wear (`true`, the default) or take off the avatar's face mask: its
    /// primitives whose material carries `extras.face_mask`. Display only.
    fn face_mask(&self, _worn: bool) {}
    fn spawn_prop(&self, _cmd: PropCmd) {}
    fn despawn_prop(&self, _id: PropId) {}
    fn grab(&self, _cmd: GrabCmd) {}
    fn release(&self, _hand: Hand) {}
    fn cancel_grab_pursuit(&self, _hand: Hand) {}
    fn grab_config(&self) -> Option<GrabConfig> { None }
    fn scene_state(&self) -> SceneState {
        SceneState::default()
    }
}

pub struct CcRenderer {
    pose_tx: watch::Sender<ArkitPose>,
    /// Bounded FIFO queue of skeletal poses. Replaces the previous
    /// `watch` channel so paced motion frames are delivered IN ORDER
    /// to Bevy's `apply_skeletal_pose` system — preserving history
    /// even if the consumer is briefly behind (e.g. while Bevy's
    /// `bind.captured` flag flips). Drained one-per-frame on the Bevy
    /// side; overflow is rare in practice (4096 poses ≈ 2 minutes at
    /// 30 fps content rate). Sending `None` enqueues a "stream
    /// cleared" sentinel that resets `apply_skeletal_pose` state.
    skeletal_pose_tx: crossbeam_channel::Sender<Option<SkeletalPose>>,
    frame_rx: crossbeam_channel::Receiver<RenderedFrame>,
    config: Arc<RendererConfig>,
    /// Flipped to `true` by a Bevy system once the GLB has spawned,
    /// bind capture has succeeded, and a settle margin of frames have
    /// rendered (= textures + materials are visibly applied). Lets the
    /// driving binary gate "start writing the MP4" on actual scene
    /// readiness instead of a wall-clock warmup timer.
    ready: Arc<AtomicBool>,
    /// Explicit shutdown joins before a standalone process unloads GPU libraries.
    join: parking_lot::Mutex<Option<JoinHandle<()>>>,
    shutdown_tx: crossbeam_channel::Sender<()>,
    prop_cmd_tx: tokio::sync::mpsc::UnboundedSender<PropCmd>,
    despawn_cmd_tx: tokio::sync::mpsc::UnboundedSender<PropId>,
    grab_cmd_tx: tokio::sync::mpsc::UnboundedSender<GrabOrRelease>,
    scene_state_rx: tokio::sync::watch::Receiver<SceneState>,
    hud_tx: watch::Sender<Option<super::office::home_hud::HomeHud>>,
    face_mask_tx: watch::Sender<bool>,
    /// `Present::Offline`: lets one frame render per step; frames come
    /// straight from the render world.
    pub(super) offline: Option<OfflineChannels>,
}

pub(super) struct OfflineChannels {
    /// Dropped at shutdown so a waiting frame step gives up.
    pub step_tx: parking_lot::Mutex<Option<crossbeam_channel::Sender<()>>>,
    pub frames: crossbeam_channel::Receiver<super::bevy_app::OfflineFrame>,
}

impl Renderer for CcRenderer {
    fn grab_config(&self) -> Option<GrabConfig> {
        self.config.physics.as_ref().map(|physics| physics.grab.clone())
    }
    fn shutdown(&self) -> anyhow::Result<()> {
        CcRenderer::shutdown(self)
    }
    fn push_pose(&self, pose: ArkitPose) {
        Self::push_pose(self, pose);
    }
    fn push_skeletal_pose(&self, pose: Option<SkeletalPose>) {
        Self::push_skeletal_pose(self, pose);
    }
    fn drain_frames(&self, sink: &mut Vec<RenderedFrame>) {
        Self::drain_frames(self, sink);
    }
    fn is_ready(&self) -> bool {
        Self::is_ready(self)
    }
    fn skeletal_queue_depth(&self) -> usize {
        // crossbeam_channel::Sender::len = number of enqueued items
        // Bevy hasn't pulled yet via try_recv.
        self.skeletal_pose_tx.len()
    }
    fn office_command(&self, cmd: super::office::controller::OfficeCommand) {
        let _ = self.grab_cmd_tx.send(GrabOrRelease::Office(cmd));
    }
    fn spawn_prop(&self, cmd: PropCmd) {
        let _ = self.prop_cmd_tx.send(cmd);
    }
    fn despawn_prop(&self, id: PropId) {
        let _ = self.despawn_cmd_tx.send(id);
    }
    fn home_hud(&self, hud: Option<super::office::home_hud::HomeHud>) {
        Self::home_hud(self, hud);
    }
    fn face_mask(&self, worn: bool) {
        Self::set_face_mask(self, worn);
    }
    fn grab(&self, cmd: GrabCmd) {
        let _ = self.grab_cmd_tx.send(GrabOrRelease::Grab(cmd));
    }
    fn release(&self, hand: Hand) {
        let _ = self.grab_cmd_tx.send(GrabOrRelease::Release(hand));
    }
    fn cancel_grab_pursuit(&self, hand: Hand) {
        let _ = self.grab_cmd_tx.send(GrabOrRelease::CancelPursuit(hand));
    }
    fn scene_state(&self) -> SceneState {
        self.scene_state_rx.borrow().clone()
    }
}

impl CcRenderer {
    /// Spawn the Bevy app on a dedicated OS thread. Returns once the
    /// thread is up; the GLB still loads asynchronously inside Bevy
    /// so the first few frames may be empty (the consumer should
    /// drop frames whose `pts_ms == 0` if it cares about that).
    pub fn spawn(config: RendererConfig) -> anyhow::Result<Self> {
        tracing::info!(
            target: "cc_render",
            "CcRenderer::spawn: glb={:?} map={:?} {}x{}@{}fps",
            config.glb_path,
            config.arkit_map_path,
            config.width,
            config.height,
            config.fps,
        );
        let config = Arc::new(config);

        let (pose_tx, pose_rx) = watch::channel(ArkitPose::default());
        // Skeletal poses are FIFO so paced kimodo / motion_player frames
        // play through in order even if Bevy's `apply_skeletal_pose`
        // system is briefly stalled (typically during the bind-capture
        // settle). Cap = 4096 ≈ 2 min at 30 fps content rate; overflow
        // drops the NEW push (a noisy `try_send` failure is logged).
        let (skeletal_pose_tx, skeletal_pose_rx) =
            crossbeam_channel::bounded::<Option<SkeletalPose>>(4096);
        // Frame ring sized for ~2s of buffered video at 30 fps.
        // Audio2Face emits blendshapes in bursts (30 frames-worth of
        // poses arrive in <1 ms after each TTS chunk), so the
        // streaming-node side may drain fewer frames than Bevy is
        // producing for a given burst window. A bigger ring keeps the
        // burst lossless; the encoder + WebRTC jitter buffer rate-pace
        // delivery to the wire from the timestamps we stamp on.
        let (frame_tx, frame_rx) = crossbeam_channel::bounded::<RenderedFrame>(64);
        let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded::<()>(1);

        let ready = Arc::new(AtomicBool::new(false));
        let ready_tx = Arc::clone(&ready);
        let (prop_cmd_tx, prop_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<PropCmd>();
        let (despawn_cmd_tx, despawn_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<PropId>();
        let (grab_cmd_tx, grab_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<GrabOrRelease>();
        let (scene_state_tx, scene_state_rx) =
            tokio::sync::watch::channel::<SceneState>(SceneState::default());
        let (hud_tx, hud_rx) = watch::channel(None);
        let (face_mask_tx, face_mask_rx) = watch::channel(true);

        // The overlay window comes up first: it picks the render size
        // (side x monitor scale), and a failure fails the spawn.
        let overlay = match &config.present {
            Present::Overlay(oc) => {
                let target_px = Arc::new(std::sync::atomic::AtomicU32::new(oc.size));
                let (tx, _detached) = super::overlay::spawn_presenter(oc.clone(), target_px.clone())?;
                Some(super::overlay::OverlaySink { tx, target_px })
            }
            Present::Capture | Present::Offline => None,
        };
        let (offline, offline_gate) = if config.present == Present::Offline {
            let (step_tx, step_rx) = crossbeam_channel::bounded::<()>(1);
            let (frames_tx, frames) = crossbeam_channel::unbounded();
            (
                Some(OfflineChannels { step_tx: parking_lot::Mutex::new(Some(step_tx)), frames }),
                Some(super::bevy_app::OfflineGate { step_rx, frames_tx }),
            )
        } else {
            (None, None)
        };

        let cfg = Arc::clone(&config);
        let join = thread::Builder::new()
            .name("cc-render-bevy".into())
            .spawn(move || {
                tracing::info!(target: "cc_render", "cc-render-bevy thread starting");
                run_bevy_app(
                    cfg,
                    pose_rx,
                    skeletal_pose_rx,
                    frame_tx,
                    shutdown_rx,
                    ready_tx,
                    prop_cmd_rx,
                    despawn_cmd_rx,
                    grab_cmd_rx,
                    scene_state_tx,
                    hud_rx,
                    face_mask_rx,
                    overlay,
                    offline_gate,
                );
                tracing::warn!(target: "cc_render", "cc-render-bevy thread exited");
            })?;

        LIVE_RENDERERS.fetch_add(1, Ordering::AcqRel);
        Ok(Self {
            pose_tx,
            skeletal_pose_tx,
            frame_rx,
            config,
            ready,
            join: parking_lot::Mutex::new(Some(join)),
            shutdown_tx,
            prop_cmd_tx,
            despawn_cmd_tx,
            grab_cmd_tx,
            scene_state_rx,
            hud_tx,
            face_mask_tx,
            offline,
        })
    }

    /// Replace the display-only home overlay (latest wins); `None` hides it.
    pub fn home_hud(&self, hud: Option<super::office::home_hud::HomeHud>) {
        let _ = self.hud_tx.send(hud);
    }

    /// Wear (`true`, the default) or take off the avatar's face mask: the
    /// primitives whose glTF material carries `extras.face_mask` (the Saluki
    /// operators' balaclavas and masks, see `scripts/saluki/glb_materials.py`).
    /// Takes effect on the next frame, also before the avatar has loaded.
    pub fn set_face_mask(&self, worn: bool) {
        let _ = self.face_mask_tx.send(worn);
    }

    /// Has the Bevy app finished spawning the GLB scene, captured
    /// bind-pose data, and rendered a few frames after that? Drives
    /// the smoke binary's "start writing the MP4 now" decision —
    /// replaces a fixed wall-clock warmup with a precise readiness
    /// signal.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Has the render thread exited (failed to start, or shut down)?
    pub(super) fn thread_finished(&self) -> bool {
        self.join.lock().as_ref().map_or(true, |j| j.is_finished())
    }

    /// Finish GPU teardown before the host exits. Ordinary Drop remains
    /// nonblocking for pipeline owners that are not terminating the process.
    pub fn shutdown(&self) -> anyhow::Result<()> {
        let _ = self.shutdown_tx.try_send(());
        if let Some(o) = &self.offline {
            o.step_tx.lock().take();
        }
        if let Some(thread) = self.join.lock().take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("renderer thread panicked during shutdown"))?;
        }
        Ok(())
    }

    /// Push the latest ARKit pose. Older un-rendered poses are
    /// silently dropped (watch channel = single-slot, latest-wins).
    pub fn push_pose(&self, pose: ArkitPose) {
        // `send` only fails if the receiver is dropped, which only
        // happens during shutdown — fine to ignore.
        let _ = self.pose_tx.send(pose);
    }

    /// Push the latest skeletal pose, or `None` to release the avatar
    /// back to its baked AnimationPlayer track. Same single-slot
    /// latest-wins semantics as `push_pose`.
    pub fn push_skeletal_pose(&self, pose: Option<SkeletalPose>) {
        let pts = pose.as_ref().map(|p| p.pts_ms);
        match self.skeletal_pose_tx.try_send(pose) {
            Ok(_) => tracing::debug!(target: "cc_render",
                "push_skeletal_pose: queued pts_ms={:?}", pts),
            Err(crossbeam_channel::TrySendError::Full(_)) => tracing::warn!(
                target: "cc_render",
                "push_skeletal_pose: queue full (4096) — dropping pts_ms={:?}",
                pts
            ),
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => tracing::warn!(
                target: "cc_render",
                "push_skeletal_pose: receiver dropped (renderer shutting down)"
            ),
        }
    }

    /// Drain any rendered frames that are ready. Non-blocking.
    pub fn drain_frames(&self, sink: &mut Vec<RenderedFrame>) {
        while let Ok(f) = self.frame_rx.try_recv() {
            sink.push(f);
        }
    }

    pub fn config(&self) -> &RendererConfig {
        &self.config
    }

    /// Send a prop spawn command to the Bevy thread. Returns immediately;
    /// the prop is created on the next physics tick.
    pub fn spawn_prop(&self, cmd: PropCmd) {
        let _ = self.prop_cmd_tx.send(cmd);
    }

    /// Send a prop despawn command. No-op if the prop id is unknown.
    pub fn despawn_prop(&self, id: PropId) {
        let _ = self.despawn_cmd_tx.send(id);
    }

    /// Issue a grab command (pursue → attach) for the given hand.
    pub fn grab(&self, cmd: GrabCmd) {
        let _ = self.grab_cmd_tx.send(GrabOrRelease::Grab(cmd));
    }

    /// Release any prop currently grabbed by the given hand.
    pub fn release(&self, hand: Hand) {
        let _ = self.grab_cmd_tx.send(GrabOrRelease::Release(hand));
    }

    /// Latest published `SceneState` snapshot (latest-wins, never blocks).
    pub fn scene_state(&self) -> SceneState {
        self.scene_state_rx.borrow().clone()
    }
}

// ============================================================
// Physics types (avatar-render-cc-physics feature)
// ============================================================
pub use physics_types::*;
mod physics_types {
    use std::path::PathBuf;

    #[derive(Clone, Debug)]
    pub struct PhysicsConfig {
        pub gravity: [f32; 3],
        pub timestep_hz: u32,
        pub max_substeps_per_frame: u32,
        pub ground_y: Option<f32>,
        pub play_volume: Option<PlayVolume>,
        pub sidecar_path: Option<PathBuf>,
        pub grab: GrabConfig,
    }

    impl Default for PhysicsConfig {
        fn default() -> Self {
            Self {
                gravity: [0.0, -9.81, 0.0],
                // 120 Hz physics: 4 steps per render frame at 30 fps.
                // Smaller per-step jumps + interpolation between
                // steps gives visibly smoother jiggle. Cost is
                // negligible for the few-bones-per-avatar jiggle set.
                timestep_hz: 120,
                max_substeps_per_frame: 4,
                ground_y: None,
                play_volume: Some(PlayVolume::default()),
                sidecar_path: None,
                grab: GrabConfig::default(),
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct PlayVolume {
        pub half_extents_xz: f32,
        pub ceiling_y: f32,
    }

    impl Default for PlayVolume {
        fn default() -> Self {
            Self {
                half_extents_xz: 2.0,
                ceiling_y: 3.0,
            }
        }
    }

    /// Grab-FSM tunables. Defaults match the spec:
    /// - 5 cm wrist bound PLUS verified solid-shape contact for joint formation
    /// - 2 s pursuit timeout before `failed_grabs("timeout")`
    /// - historical palm offset retained for source compatibility only; contact
    ///   grips capture existing relative frames and reject forced overrides
    #[derive(Clone, Debug)]
    pub struct GrabConfig {
        pub proximity_m: f32,
        pub timeout_secs: f32,
        pub default_palm_offset: [f32; 3],
    }

    impl Default for GrabConfig {
        fn default() -> Self {
            Self {
                proximity_m: 0.05,
                timeout_secs: 2.0,
                default_palm_offset: [0.08, 0.0, 0.0],
            }
        }
    }

    pub type PropId = String;
    // Duplicate spawn_prop with same id REPLACES the existing prop
    // (despawn-then-spawn). Active grabs against the replaced prop
    // release first and emit a failed_grabs entry with
    // reason="prop_replaced".

    #[derive(Clone, Debug)]
    pub struct PropCmd {
        pub id: PropId,
        pub shape: PropShape,
        pub initial_transform: [[f32; 4]; 4],
        pub mass_kg: f32,
        pub grasp: Option<GraspAnchor>,
        pub friction: f32,
        pub restitution: f32,
    }

    #[derive(Clone, Debug)]
    pub enum PropShape {
        Box { half_extents: [f32; 3] },
        Sphere { radius: f32 },
        Capsule { radius: f32, half_height: f32 },
        MeshGlb { path: PathBuf, scale: f32 },
    }

    #[derive(Clone, Debug)]
    pub struct GraspAnchor {
        pub local_offset: [f32; 3],
        pub local_rotation: [f32; 4],
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Hand {
        Left,
        Right,
    }

    #[derive(Clone, Debug)]
    pub struct GrabCmd {
        pub hand: Hand,
        pub target: PropId,
        pub palm_offset_override: Option<[f32; 3]>,
    }

    #[derive(Clone, Debug, Default, serde::Serialize)]
    pub struct SceneState {
        pub office: Option<crate::cc_render::office::controller::OfficeSnapshot>,
        pub props: Vec<PropSnapshot>,
        pub grabs: Vec<GrabSnapshot>,
        pub failed_grabs: Vec<GrabFailure>,
        pub pts_ms: u64,
    }

    #[derive(Clone, Debug, serde::Serialize)]
    pub struct PropSnapshot {
        pub id: PropId,
        pub transform: [[f32; 4]; 4],
        pub aabb_min: [f32; 3],
        pub aabb_max: [f32; 3],
        pub grasp_world: Option<[[f32; 4]; 4]>,
        pub loading: bool,
    }

    #[derive(Clone, Debug, serde::Serialize)]
    pub struct GrabSnapshot {
        pub hand: Hand,
        pub prop: PropId,
    }

    #[derive(Clone, Debug, serde::Serialize)]
    pub struct GrabFailure {
        pub hand: Hand,
        pub target: PropId,
        pub reason: String,
    }

    /// Internal command on the grab channel — multiplexes both
    /// `Renderer::grab` and `Renderer::release` so a single
    /// `mpsc::UnboundedReceiver` can drive the FSM in `physics::grab`.
    #[derive(Debug, Clone)]
    pub enum GrabOrRelease {
        Office(crate::cc_render::office::controller::OfficeCommand),
        Grab(GrabCmd),
        Release(Hand),
        CancelPursuit(Hand),
    }
}

impl Drop for CcRenderer {
    fn drop(&mut self) {
        // Best-effort shutdown signal. The thread observes this and
        // breaks out of its Bevy loop; the JoinHandle is detached
        // (we don't `.join()` here because Bevy shutdown is
        // historically flaky around wgpu drops and we don't want
        // the calling thread to hang).
        let _ = self.shutdown_tx.try_send(());
        LIVE_RENDERERS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// `CcRenderer`s spawned and not yet dropped in this process.
static LIVE_RENDERERS: AtomicUsize = AtomicUsize::new(0);

/// How many `CcRenderer`s are alive in this process (each one a Bevy app
/// rendering on the GPU), e.g. for a host that keeps exactly one session
/// preloaded: it preloads again once the last viewer's renderer is gone.
pub fn live_renderers() -> usize {
    LIVE_RENDERERS.load(Ordering::Acquire)
}

/// Body of the dedicated render thread. Constructs the Bevy `App`
/// (using the systems in [`super::bevy_app`]) and runs it.
#[allow(clippy::too_many_arguments)]
fn run_bevy_app(
    config: Arc<RendererConfig>,
    pose_rx: watch::Receiver<ArkitPose>,
    skeletal_pose_rx: crossbeam_channel::Receiver<Option<SkeletalPose>>,
    frame_tx: crossbeam_channel::Sender<RenderedFrame>,
    shutdown_rx: crossbeam_channel::Receiver<()>,
    ready: Arc<AtomicBool>,
    prop_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<PropCmd>,
    despawn_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<PropId>,
    grab_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<GrabOrRelease>,
    scene_state_tx: tokio::sync::watch::Sender<SceneState>,
    hud_rx: watch::Receiver<Option<super::office::home_hud::HomeHud>>,
    face_mask_rx: watch::Receiver<bool>,
    overlay: Option<super::overlay::OverlaySink>,
    offline: Option<super::bevy_app::OfflineGate>,
) {
    super::bevy_app::run(
        config,
        pose_rx,
        skeletal_pose_rx,
        frame_tx,
        shutdown_rx,
        ready,
        prop_cmd_rx,
        despawn_cmd_rx,
        grab_cmd_rx,
        scene_state_tx,
        hud_rx,
        face_mask_rx,
        overlay,
        offline,
    );
}
