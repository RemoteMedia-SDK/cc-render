//! `CcRenderNode` — streaming-pipeline wrapper around [`super::CcRenderer`].
//!
//! Consumes JSON envelopes (`kind = "blendshapes" | "skeletal_pose" |
//! "barge_in" | "motion_goal" | "home_hud"`) on its reactive input path and emits
//! `RuntimeData::Video` frames from `tick()` —
//! `pacing_nature() = ClockedToOutboundMedia` with a wall fallback at
//! `config.fps`. The Bevy thread free-runs and produces frames; the
//! pacer drives `tick()` to drain + emit them on the wire's cadence.
//!
//! Output `PixelFormat` is `Rgba32` (Bevy renders 8-bit RGBA).
//! Downstream WebRTC video sender accepts it directly.

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use remotemedia_plugin_sdk::traits::streaming::{AsyncStreamingNode, PacingNature, Tick};
use remotemedia_plugin_sdk::types::{AudioSamples, Error, PixelFormat, RuntimeData};

use crate::arkit::{BlendshapeFrame, ARKIT_BLENDSHAPE_NAMES};
use crate::session_control::{aux_port_of, BARGE_IN_PORT};

use super::renderer::{ArkitPose, CcRenderer, Renderer, RendererConfig, SkeletalPose};

/// Local `Result` alias so the ported source keeps the bare-name
/// `Result<T>` style from the original module.
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct CcRenderConfig {
    pub glb_path: PathBuf,
    pub arkit_map_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Stream id stamped onto every emitted `RuntimeData::Video`.
    pub video_stream_id: String,
    /// Optional environment scene GLB loaded alongside the avatar.
    /// Forwarded to `RendererConfig::scene_glb_path`. See that field's
    /// doc-comment for the `CC_SCENE_*` env-var tunables (placement,
    /// scale, light stripping).
    pub scene_glb_path: Option<PathBuf>,
    /// When true, `tick()` emits `RuntimeData::Video` frames continuously
    /// once the renderer is ready, regardless of upstream audio /
    /// blendshape arrival. Buffered speech drains on that same presentation
    /// clock, including ticks where video capture misses its deadline.
    ///
    /// Default `false` preserves the smoke binary's strict A+V pairing
    /// for offline recording. Set to `true` for live WebRTC so the
    /// avatar / scene is visible immediately on connect, during model
    /// warmup, and during silence.
    pub realtime_mode: bool,
    /// Startup-owned signing key; never accepted through runtime control messages.
    pub scene_key_file: Option<PathBuf>,
    /// Fixed asset data, verified against the registered sampler's content hash.
    pub scene_geometry_file: Option<PathBuf>,
}

impl Default for CcRenderConfig {
    fn default() -> Self {
        Self {
            glb_path: PathBuf::new(),
            arkit_map_path: PathBuf::new(),
            width: 1280,
            height: 1280,
            fps: 30,
            video_stream_id: "avatar.video".to_string(),
            scene_glb_path: None,
            realtime_mode: false,
            scene_key_file: None,
            scene_geometry_file: None,
        }
    }
}

/// Internal audio buffer state — owned by `CcRenderNode` so it can
/// emit audio chunks paired with each rendered Video frame at content
/// time. Audio enters via `process_streaming` (RuntimeData::Audio
/// passed through `Audio2FaceLipSyncNode` and the gate); audio leaves
/// via `tick()` synchronized with Video.
struct AudioBuf {
    /// Mono f32 samples in arrival order. Currently we only handle
    /// mono — kokoro is mono and audio2face requires mono.
    samples: std::collections::VecDeque<f32>,
    sample_rate: u32,
    channels: u32,
    /// Total samples emitted so far (for content-time bookkeeping).
    samples_emitted: u64,
    faces: std::collections::VecDeque<ArkitPose>,
    speech_ends: std::collections::VecDeque<(u64, serde_json::Value)>,
    face_started: bool,
    speech_epoch: u64,
}

impl AudioBuf {
    fn new() -> Self {
        Self {
            samples: std::collections::VecDeque::with_capacity(48_000 * 4),
            sample_rate: 0,
            channels: 0,
            samples_emitted: 0,
            faces: Default::default(),
            speech_ends: Default::default(),
            face_started: false,
            speech_epoch: 0,
        }
    }
    fn cancel(&mut self) {
        let epoch = self.speech_epoch + 1;
        *self = Self::new();
        self.speech_epoch = epoch;
    }

    fn push_audio(
        &mut self,
        samples: &[f32],
        rate: u32,
        channels: u32,
        epoch: Option<u64>,
    ) -> Result<()> {
        if epoch.is_some_and(|e| e < self.speech_epoch) {
            return Ok(());
        }
        if rate == 0 || channels == 0 || samples.len() % channels as usize != 0 {
            return Err(Error::Execution("invalid speech audio format".into()));
        }
        if self.sample_rate != 0 && self.sample_rate != rate {
            return Err(Error::Execution(
                "speech sample rate changed mid-session".into(),
            ));
        }
        if self.samples.len() + samples.len() / channels as usize > rate as usize * 60 {
            return Err(Error::Execution(
                "renderer speech buffer exceeds 60 seconds".into(),
            ));
        }
        self.sample_rate = rate;
        self.channels = 1;
        self.samples.extend(
            samples
                .chunks_exact(channels as usize)
                .map(|c| c.iter().sum::<f32>() / channels as f32),
        );
        Ok(())
    }

    fn push_face(&mut self, pose: ArkitPose, epoch: Option<u64>) -> Result<()> {
        if epoch.is_some_and(|e| e < self.speech_epoch) {
            return Ok(());
        }
        if self.faces.len() >= 1800 {
            return Err(Error::Execution(
                "renderer facial queue exceeds 60 seconds".into(),
            ));
        }
        if self.faces.back().is_some_and(|p| p.pts_ms > pose.pts_ms) {
            return Err(Error::Execution("out of order facial frame".into()));
        }
        self.faces.push_back(pose);
        Ok(())
    }
}

fn audio_samples_per_tick(frame: u64, rate: u32, fps: u64) -> usize {
    (((frame + 1) as u128 * rate as u128 / fps as u128)
        - (frame as u128 * rate as u128 / fps as u128)) as usize
}

pub struct CcRenderNode {
    config: CcRenderConfig,
    renderer: Arc<dyn Renderer>,
    frame_counter: Arc<AtomicU64>,
    /// First pts seen (anchor); subsequent frame timestamps are
    /// reported relative to this so y4m playback aligns with audio
    /// captured by the same session anchor.
    anchor_pts_ms: Arc<tokio::sync::Mutex<Option<u64>>>,
    /// Audio buffer fed by `RuntimeData::Audio` inputs. `tick()` emits
    /// matching Audio chunks alongside each Video frame so the
    /// downstream consumer (smoke binary / WebRTC sender) receives
    /// content-time-aligned A/V pairs.
    audio_buf: Arc<parking_lot::Mutex<AudioBuf>>,
    /// One-shot guard: emits a `{kind:"renderer_ready"}` Json envelope
    /// to `output_rx` the very first tick that `renderer.is_ready()`
    /// returns true. Driver bins (smoke binary / WebRTC orchestrator)
    /// listen for it before opening the upstream gate so paced pose
    /// pushes are delivered to a Bevy app whose `bind.captured` and
    /// scene-load gates have all flipped — preventing the
    /// "pre-ready burst → watch latest-wins → animation stuck on last
    /// pose" failure mode that `check_sync.py` exposed.
    ready_signaled: Arc<AtomicBool>,
    /// Wallclock of the last input we received on each driving stream.
    /// `tick()` keeps emitting V+A pairs (with silence-padded A when
    /// the audio buffer is empty) until BOTH streams have been quiet
    /// for `TAIL_QUIET`. Lets a 4 s body motion play through to the
    /// end even when the speech audio finishes earlier.
    last_audio_recv_at: Arc<parking_lot::Mutex<Option<std::time::Instant>>>,
    last_skel_recv_at: Arc<parking_lot::Mutex<Option<std::time::Instant>>>,
    /// Wallclock of the first blendshape we received on the reactive
    /// input path. Used by `tick()` to delay the first V emit until
    /// after the first blendshape has had a chance to actually be
    /// applied by Bevy — otherwise V_0 captures the default rest pose
    /// while the audio drain is already content_time=0, and lipsync
    /// stays offset by the gate→bevy_apply latency for the entire
    /// stream.
    first_bs_recv_at: Arc<parking_lot::Mutex<Option<std::time::Instant>>>,
    /// Cumulative count of stale Bevy frames discarded in realtime mode.
    /// In `pacing_nature() = ClockedToOutboundMedia` we want exactly one
    /// outbound frame per tick; if Bevy renders faster than the wire
    /// cadence (typical for a small avatar on a fast GPU), the surplus
    /// frames are dropped at the source — cheaper than letting them
    /// flow downstream and getting dropped by the WebRTC dispatcher
    /// after a YUV conversion + memcpy. Logged at debug.
    stale_frames_dropped: Arc<AtomicU64>,
    motion_driver: parking_lot::Mutex<super::office::motion_intent::NativeMotionDriver>,
    scene_delivery: parking_lot::Mutex<super::office::scene_delivery::SceneDelivery>,
    scene_task: parking_lot::Mutex<Option<super::office::scene_lifecycle::SceneLifecycle>>,
    scene_context_revision: AtomicU64,
    scene_signer: parking_lot::Mutex<Option<(super::office::scene_signing::SceneSigner,
        super::office::scene_signing::SceneSamplerCapability)>>,
    motion_clock: std::time::Instant,
}

/// Minimum wallclock delay between receiving the first blendshape on
/// the reactive input path and emitting the first V frame. Gives Bevy
/// roughly one frame interval to read the watch and run
/// `apply_arkit_pose` so `LastAppliedPts` reflects the new pose by the
/// time the RenderApp's capture stamps the frame.
const FIRST_BS_APPLY_GRACE: std::time::Duration = std::time::Duration::from_millis(50);

/// How long we keep emitting V+A after the most recent driving input.
/// Once both audio AND skeletal_pose have been silent for this long,
/// `tick()` stops producing — letting the smoke binary's termination
/// logic close the streams. Tuned so a kokoro chunk gap (~250 ms) or
/// a brief gate-pacing pause doesn't cause a premature cut, while
/// still keeping wrap-up latency reasonable.
const TAIL_QUIET: std::time::Duration = std::time::Duration::from_millis(500);

impl std::fmt::Debug for CcRenderNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CcRenderNode")
            .field("config", &self.config)
            .field("frames_emitted", &self.frames_emitted())
            .finish()
    }
}

impl CcRenderNode {
    fn office_query(&self, data: &RuntimeData) -> Option<RuntimeData> {
        let RuntimeData::Json(value) = data else { return None };
        let reply = match value.get("kind")?.as_str()? {
            "get_scene_state" => serde_json::json!({"kind":"scene_state",
                "state":self.observed_scene(), "request_id":value.get("request_id")}),
            "generate_waypoint" => super::office::navigation::query_reply(
                &self.renderer.scene_state(), value,
            ),
            _ => return None,
        };
        Some(RuntimeData::Json(reply))
    }

    pub fn new(config: CcRenderConfig) -> Result<Self> {
        let renderer = CcRenderer::spawn(RendererConfig {
            glb_path: config.glb_path.clone(),
            arkit_map_path: config.arkit_map_path.clone(),
            width: config.width,
            height: config.height,
            fps: config.fps,
            scene_glb_path: config.scene_glb_path.clone(),
            // Physics on by default — needed for hair / breast /
            // skirt / tongue jiggle (see `secondary::auto_detect_chains`).
            // Set `CC_PHYSICS_DISABLE=1` to skip the install for an
            // A/B render without rebuilding.
            physics: if std::env::var("CC_PHYSICS_DISABLE").as_deref() == Ok("1") {
                None
            } else {
                Some(super::renderer::PhysicsConfig::default())
            },
        })
        .map_err(|e| Error::Execution(format!("spawn CcRenderer: {e}")))?;

        Ok(Self::with_renderer(config, Arc::new(renderer)))
    }

    /// Build a node around a custom [`Renderer`] implementation. Used
    /// in tests to inject a mock that records pushed poses + emits
    /// scripted frames without requiring Bevy or a real GPU.
    pub fn with_renderer(config: CcRenderConfig, renderer: Arc<dyn Renderer>) -> Self {
        Self {
            config,
            renderer,
            frame_counter: Arc::new(AtomicU64::new(0)),
            anchor_pts_ms: Arc::new(tokio::sync::Mutex::new(None)),
            audio_buf: Arc::new(parking_lot::Mutex::new(AudioBuf::new())),
            ready_signaled: Arc::new(AtomicBool::new(false)),
            last_audio_recv_at: Arc::new(parking_lot::Mutex::new(None)),
            last_skel_recv_at: Arc::new(parking_lot::Mutex::new(None)),
            first_bs_recv_at: Arc::new(parking_lot::Mutex::new(None)),
            stale_frames_dropped: Arc::new(AtomicU64::new(0)),
            motion_driver: parking_lot::Mutex::new(Default::default()),
            scene_signer: parking_lot::Mutex::new(None),
            scene_task: parking_lot::Mutex::new(None),
            scene_delivery: parking_lot::Mutex::new(Default::default()),
            scene_context_revision: AtomicU64::new(0),
            motion_clock: std::time::Instant::now(),
        }
    }

    /// Replace retired planner telemetry with the actual native request owner.
    fn observed_scene(&self) -> serde_json::Value {
        let mut scene = serde_json::to_value(self.renderer.scene_state()).unwrap();
        if let Some(office) = scene.get_mut("office").filter(|v| v.is_object()) {
            // A queued, accepted goal already owns request revision ordering;
            // its lifecycle/poses remain distinct until the native handoff.
            let delivery = self.scene_delivery.lock();
            let task = self.scene_task.lock();
            office["motion"] = if let Some((goal, _)) = delivery.pending.as_ref() {
                serde_json::to_value(goal).unwrap()
            } else {
                serde_json::to_value(self.motion_driver.lock().current_goal()).unwrap()
            };
            office["scene_lifecycle"] = serde_json::to_value(task.as_ref()).unwrap();
            let registered = self.scene_signer.lock();
            let mut operations = Vec::new();
            if let Some((_, capability)) = registered.as_ref() {
                operations.push("navigate");
                operations.push("interact");
                if capability.renderer_geometry_sha256.is_some()
                    && office["native_seat_profile"].as_str() == Some(capability.profile_identity.as_str()) {
                    operations.push("sit");
                }
                if capability.grip_capture.as_ref().zip(self.renderer.grab_config())
                    .is_some_and(|(contract, config)| contract.matches(&config)) {
                    operations.push("grasp");
                }
            }
            office["scene_capabilities"] = serde_json::json!({"operations":operations});
        }
        scene
    }

    fn start_pending_scene(&self, office: &super::office::controller::OfficeSnapshot) -> Result<()> {
        use super::office::{controller::OfficeCommand, scene_delivery::SceneDelivery, scene_lifecycle::SceneLifecycle};
        let mut delivery = self.scene_delivery.lock();
        let Some((goal, continuation)) = delivery.pending.clone() else { return Ok(()) };
        let mut task = self.scene_task.lock();
        let mut driver = self.motion_driver.lock();
        if driver.in_flight() || !office.avatar.ready || office.motion_error.is_some() { return Ok(()) }
        let Some(end) = driver.generated_until() else { return Ok(()) };
        let Some(anchor) = SceneDelivery::request_anchor(&goal, office.avatar.displayed_pts_ms, end)
            .filter(|anchor| office.avatar.buffered_until_pts_ms.is_some_and(|buffered| buffered >= *anchor))
            else { return Ok(()) };
        let next = if continuation {
            let mut next = task.as_ref().ok_or_else(|| Error::Execution("missing scene continuation owner".into()))?.clone();
            next.bind_generation(goal.clone()).map_err(Error::Execution)?;
            Some(next)
        } else if goal.scene_outcome().is_some() {
            Some(SceneLifecycle::new(goal.clone(), office).map_err(Error::Execution)?)
        } else { None };
        driver.submit(goal.clone()).map_err(Error::Execution)?;
        delivery.reserve(&goal, anchor);
        self.renderer.office_command(OfficeCommand::NativeSceneHandoff { generation_revision: goal.revision(), history_pts_ms: anchor });
        *task = next;
        delivery.pending = None;
        Ok(())
    }

    fn observe_scene_lifecycle(&self, office: &super::office::controller::OfficeSnapshot) -> Result<Option<serde_json::Value>> {
        use super::office::scene_lifecycle::Update;
        let mut delivery = self.scene_delivery.lock();
        let mut slot = self.scene_task.lock();
        let Some(task) = slot.as_mut() else { return Ok(None) };
        if let Some(target) = task.arm_pursuit(office) {
            self.renderer.grab(super::renderer::GrabCmd { hand: super::renderer::Hand::Right,
                target, palm_offset_override: None });
        }
        let forward = |task: &mut super::office::scene_lifecycle::SceneLifecycle| for command in task.take_couplings() {
            // A physically gripped object is released by the grip itself; it settles under physics.
            if let super::office::interaction::CouplingCommand::Release { object, .. } = &command {
                if let Some((hand, _)) = office.occupied_hands.iter().find(|(_, id)| id == object) {
                    self.renderer.release(*hand);
                    continue;
                }
            }
            self.renderer.office_command(super::office::controller::OfficeCommand::Couple(command));
        };
        forward(task);
        let update = task.observe(office);
        forward(task);
        let (status, detail) = match update {
            Update::None => return Ok(None),
            Update::PhaseChanged => {
                let next = task.goal.next_revision().map_err(Error::Execution)?;
                if delivery.pending.is_none() { delivery.pending = Some((next, true)); }
                ("running", None)
            }
            Update::Completed => ("completed", None),
            Update::Failed(error) => {
                self.motion_driver.lock().fail();
                self.renderer.office_command(super::office::controller::OfficeCommand::MotionError { error: error.clone() });
                ("failed", Some(error))
            }
        };
        let event = serde_json::json!({"kind":"motion_goal_status", "goal":task.goal,
            "phase":task.phase, "status":status,"detail":detail,"displayed_pts_ms":office.avatar.displayed_pts_ms,
            "interaction":task.declared});
        if status == "failed" { *slot = None; }
        Ok(Some(event))
    }

    fn accept_semantic_goal(&self, goal: super::office::motion_intent::MotionGoal,
        delivery: &mut super::office::scene_delivery::SceneDelivery,
        task: &mut Option<super::office::scene_lifecycle::SceneLifecycle>) -> Result<()> {
        let mut driver = self.motion_driver.lock();
        if driver.current_goal().is_some_and(|old| goal.revision() > old.revision()) {
            // Finish the in-flight source block, then hand off at an existing
            // future token. Keep the old sampler context until that handoff.
            if let Some(task) = task.as_mut() { task.superseded = true; }
            delivery.pending = Some((goal, false));
        } else {
            driver.submit(goal).map_err(Error::Execution)?;
            delivery.pending = None;
            *task = None;
        }
        Ok(())
    }

    fn accept_motion_control(&self, data: &RuntimeData) -> Result<bool> {
        let RuntimeData::Json(value) = data else { return Ok(false) };
        match value["kind"].as_str() {
            Some("motion_goal") => {
                let goal: super::office::motion_intent::MotionGoal = serde_json::from_value(value.clone())
                    .map_err(|e| Error::Execution(format!("invalid motion_goal: {e}")))?;
                let mut delivery = self.scene_delivery.lock();
                let mut slot = self.scene_task.lock();
                self.motion_driver.lock().validate_goal(&goal).map_err(Error::Execution)?;
                // Semantic replacements must not bypass an already accepted
                // scene revision while its old generation is still draining.
                if delivery.pending.as_ref().is_some_and(|(old, _)| old.revision() >= goal.revision()) {
                    return Err(Error::Execution("stale pending scene revision".into()));
                }
                if let Some(outcome) = goal.scene_outcome() {
                    if self.scene_signer.lock().is_none() {
                        return Err(Error::Execution("unsupported_capability: scene operation is not registered".into()));
                    }
                    if outcome.operation == super::office::motion_intent::SceneOperation::Grasp {
                        let registered = self.scene_signer.lock();
                        let compatible = registered.as_ref().and_then(|(_, capability)| capability.grip_capture.as_ref())
                            .zip(self.renderer.grab_config()).is_some_and(|(contract, config)| contract.matches(&config));
                        if !compatible {
                            return Err(Error::Execution("unsupported_capability: physical grip contract mismatch".into()));
                        }
                    }
                    let state = self.renderer.scene_state();
                    let office = state.office.as_ref().ok_or_else(|| Error::Execution("office context unavailable".into()))?;
                    if !office.avatar.ready || self.motion_driver.lock().current_goal().is_none()
                        || !office.avatar.native_stream.as_ref().is_some_and(|s| s.session_id == goal.session_id()) {
                        return Err(Error::Execution("scene operation requires active native history".into()));
                    }
                    if outcome.operation == super::office::motion_intent::SceneOperation::Sit {
                        let registered = self.scene_signer.lock();
                        if !registered.as_ref().is_some_and(|(_, cap)| cap.renderer_geometry_sha256.is_some()
                            && office.native_seat_profile.as_deref() == Some(cap.profile_identity.as_str())) {
                            return Err(Error::Execution("unsupported_capability: native seat geometry not installed".into()));
                        }
                    }
                    super::office::scene_lifecycle::SceneLifecycle::new(goal.clone(), office).map_err(Error::Execution)?;
                    self.renderer.cancel_grab_pursuit(super::renderer::Hand::Right);
                    if let Some(task) = slot.as_mut() { task.superseded = true; }
                    delivery.pending = Some((goal, false));
                } else {
                    self.accept_semantic_goal(goal, &mut delivery, &mut slot)?;
                    self.renderer.cancel_grab_pursuit(super::renderer::Hand::Right);
                }
                Ok(true)
            }
            Some("motion_capabilities") => {
                if let Some(key_file) = &self.config.scene_key_file {
                    let capability = serde_json::from_value(value["scene_sampler"].clone())
                        .map_err(|e| Error::Execution(format!("invalid scene capability: {e}")))?;
                    let mut slot = self.scene_signer.lock();
                    if let Some((_, current)) = slot.as_ref() {
                        if current == &capability { return Ok(true); }
                        return Err(Error::Execution("scene capability cannot be replaced mid-session".into()));
                    }
                    let signer = super::office::scene_signing::SceneSigner::load(
                        key_file, &self.config.glb_path, &capability).map_err(Error::Execution)?;
                    if let Some(path) = &self.config.scene_geometry_file {
                        let geometry = super::office::avatar_geometry::AvatarGeometry::load(path, &capability)
                            .map_err(Error::Execution)?;
                        self.renderer.office_command(super::office::controller::OfficeCommand::InstallNativeSeatGeometry {
                            geometry: std::sync::Arc::new(geometry),
                        });
                    }
                    *slot = Some((signer, capability));
                }
                Ok(true)
            }
            Some("home_hud") => {
                // Display-only producer state; never scheduling or motion input.
                let hud = super::office::home_hud::parse(value).map_err(Error::Execution)?;
                self.renderer.home_hud(hud);
                Ok(true)
            }
            Some("motion_tick") => {
                if let (Some(session), Some(revision), Some(pts)) = (
                    value["session_id"].as_str(), value["generation_revision"].as_u64(),
                    value["pts_ms"].as_u64(),
                ) {
                    self.motion_driver.lock().acknowledge(session, revision, pts);
                }
                Ok(true)
            }
            Some("motion_error") => {
                self.motion_driver.lock().fail();
                Ok(false) // Also stop renderer playback through the existing error path.
            }
            Some("motion_request" | "perform_action" | "cancel_action" | "edit_motion_timeline") => {
                Err(Error::Execution("legacy authored motion is retired; submit motion_goal v2".into()))
            }
            _ => Ok(false),
        }
    }

    fn accept_face(&self, pose: ArkitPose, data: &RuntimeData) -> Result<()> {
        if self.config.realtime_mode {
            let epoch = match data {
                RuntimeData::Json(v) => v["speech_epoch"].as_u64(),
                _ => None,
            };
            self.audio_buf.lock().push_face(pose, epoch)
        } else {
            self.renderer.push_pose(pose);
            Ok(())
        }
    }

    fn accept_speech_end(&self, data: &RuntimeData) -> Result<bool> {
        let RuntimeData::Json(value) = data else {
            return Ok(false);
        };
        if value["kind"] != "speech_end" {
            return Ok(false);
        }
        let mut audio = self.audio_buf.lock();
        if value["speech_epoch"]
            .as_u64()
            .is_some_and(|e| e < audio.speech_epoch)
        {
            return Ok(true);
        }
        audio.push_face(
            ArkitPose {
                pts_ms: value["pts_us"].as_u64().unwrap_or(0) / 1000,
                ..Default::default()
            },
            value["speech_epoch"].as_u64(),
        )?;
        if self.config.realtime_mode {
            let end = audio.samples_emitted + audio.samples.len() as u64;
            audio.speech_ends.push_back((end, value.clone()));
        }
        Ok(true)
    }

    pub fn frames_emitted(&self) -> u64 {
        self.frame_counter.load(Ordering::Relaxed)
    }

    /// Convert a `BlendshapeFrame` (52-element fixed-order array) into
    /// the `name → weight` map shape the Bevy `apply_arkit_pose`
    /// system expects.
    pub(crate) fn frame_to_pose(f: &BlendshapeFrame) -> ArkitPose {
        let mut weights = std::collections::HashMap::with_capacity(ARKIT_BLENDSHAPE_NAMES.len());
        for (i, name) in ARKIT_BLENDSHAPE_NAMES.iter().enumerate() {
            // Clamp to [0,1] — the proto's `apply_arkit_pose` does this
            // anyway, but doing it once at the boundary makes the Bevy
            // thread cheaper and avoids per-mesh-per-morph clamping.
            let w = f.arkit_52[i].clamp(0.0, 1.0);
            weights.insert((*name).to_string(), w);
        }
        ArkitPose {
            weights,
            pts_ms: f.pts_ms,
        }
    }

    /// Convert a timed ARKit-52 envelope using the same channel mapping as
    /// streaming node input. Useful for direct-renderer recording clients.
    pub fn arkit_pose_from_json(v: &serde_json::Value) -> Option<ArkitPose> {
        BlendshapeFrame::from_json(v)
            .ok()
            .map(|f| Self::frame_to_pose(&f))
    }

    /// Decode an envelope. Same shape as `Live2DRenderNode::decode_envelope`
    /// but we also handle skeletal-pose JSON shipped by `KimodoMotionNode`.
    fn decode_envelope(data: &RuntimeData) -> Option<RendererInput> {
        if matches!(aux_port_of(data), Some(BARGE_IN_PORT)) {
            return Some(RendererInput::BargeIn);
        }
        let RuntimeData::Json(v) = data else {
            return None;
        };
        let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        match kind {
            "blendshapes" => BlendshapeFrame::from_json(v)
                .ok()
                .map(RendererInput::Blendshape),
            "skeletal_pose" => {
                let parsed = Self::skeletal_pose_from_json(v);
                if parsed.is_none() {
                    tracing::warn!(
                        target: "cc_render",
                        "skeletal_pose envelope failed to parse: keys={:?}",
                        v.as_object().map(|o| o.keys().collect::<Vec<_>>()),
                    );
                } else {
                    tracing::debug!(target: "cc_render",
                        "skeletal_pose decoded (pts_ms={:?})",
                        v.get("pts_ms").and_then(|p| p.as_u64()));
                }
                parsed.map(RendererInput::Skeletal)
            }
            "motion_error" => {
                serde_json::from_value(v.clone())
                    .ok()
                    .map(RendererInput::Office)
            }
            "barge_in" => Some(RendererInput::BargeIn),
            // Audio clock + emotion silently ignored — see TODO above.
            _ => None,
        }
    }

    /// Parse a `kind="skeletal_pose"` JSON envelope (shape produced by
    /// `KimodoMotionNode`) into a `SkeletalPose`. Returns `None` on
    /// any structural problem so the streaming loop just drops the
    /// frame instead of stalling — matches `BlendshapeFrame::from_json`'s
    /// fail-safe behavior.
    pub fn skeletal_pose_from_json(v: &serde_json::Value) -> Option<SkeletalPose> {
        let stream = super::office::timeline::StreamIdentity::from_envelope(v)?;
        let root = v
            .get("root_pos")
            .and_then(|r| serde_json::from_value(r.clone()).ok());
        let root_pos = if stream.is_some() {
            root?
        } else {
            root.unwrap_or([0.; 3])
        };
        SkeletalPose {
            joint_quats: serde_json::from_value(v.get("joint_quats_xyzw")?.clone()).ok()?,
            root_pos,
            stream,
            pts_ms: v.get("pts_ms").and_then(|p| p.as_u64()).unwrap_or(0),
        }
        .normalized()
    }
}

#[derive(Debug)]
enum RendererInput {
    Blendshape(BlendshapeFrame),
    Skeletal(SkeletalPose),
    BargeIn,
    Office(super::office::controller::OfficeCommand),
}

#[async_trait]
impl AsyncStreamingNode for CcRenderNode {
    fn node_type(&self) -> &str {
        "CcRenderNode"
    }

    async fn process(&self, _data: RuntimeData) -> Result<RuntimeData> {
        Err(Error::Execution(
            "CcRenderNode requires streaming mode — use process_streaming()".into(),
        ))
    }

    /// Reactive input path: ingest blendshape envelopes / barge to
    /// update the renderer's pose state. Frame emission moved to
    /// `tick()` (pacing-domains migration) — this method never
    /// invokes the callback.
    async fn process_streaming<F>(
        &self,
        data: RuntimeData,
        _session_id: Option<String>,
        mut _callback: F,
    ) -> Result<usize>
    where
        F: FnMut(RuntimeData) -> Result<()> + Send,
    {
        // Python workers carry structured envelopes on the JSON text channel.
        // Decode that channel before dispatch, just as native Json inputs are.
        let data = match &data {
            RuntimeData::Text(text)
                if remotemedia_plugin_sdk::types::split_text_str(text).0 == "json" =>
            {
                RuntimeData::Json(
                    serde_json::from_str(remotemedia_plugin_sdk::types::split_text_str(text).1)
                        .map_err(|e| {
                            Error::Execution(format!("invalid renderer JSON channel: {e}"))
                        })?,
                )
            }
            _ => data,
        };
        if self.accept_motion_control(&data)? {
            return Ok(0);
        }
        if self.accept_speech_end(&data)? {
            return Ok(0);
        }
        if let Some(reply) = self.office_query(&data) {
            _callback(reply)?;
            return Ok(1);
        }
        // Audio inputs: buffer for `tick()` to emit paired with Video
        // frames at content time. This is what makes
        // [[TTS → Audio2Face], Kimodo Motion] → CC Render emit mouth
        // movements + audio together — both driven by the same content
        // clock instead of independent wallclock timelines.
        if let RuntimeData::Audio {
            samples,
            sample_rate,
            channels,
            timestamp_us,
            metadata,
            ..
        } = &data
        {
            tracing::debug!(
                target: "timing",
                stage = "ccrender_recv_audio",
                pts_ms = timestamp_us.map(|u| (u / 1000) as i64).unwrap_or(-1),
                samples = samples.len() as u64,
            );
            *self.last_audio_recv_at.lock() = Some(std::time::Instant::now());
            self.audio_buf.lock().push_audio(
                samples.as_slice(),
                *sample_rate,
                *channels,
                metadata
                    .as_ref()
                    .and_then(|m| m.get("speech_epoch"))
                    .and_then(|v| v.as_u64()),
            )?;
            return Ok(0);
        }

        match Self::decode_envelope(&data) {
            Some(RendererInput::Blendshape(f)) => {
                let max_w = f.arkit_52.iter().copied().fold(0.0_f32, f32::max);
                tracing::debug!(
                    target: "timing",
                    stage = "ccrender_apply_bs",
                    pts_ms = f.pts_ms as i64,
                    max_w = max_w as f64,
                );
                // Periodic per-channel diagnostic: log the 8 highest
                // blendshape weights so we can verify the model is
                // actually driving non-mouth channels (brows, eyes,
                // cheeks) and not just the lips.
                if f.pts_ms == 0 || (f.pts_ms / 33) % 30 == 0 {
                    let mut indexed: Vec<(usize, f32)> = f
                        .arkit_52
                        .iter()
                        .copied()
                        .enumerate()
                        .filter(|(_, w)| w.abs() > 0.001)
                        .collect();
                    indexed.sort_by(|a, b| {
                        b.1.abs()
                            .partial_cmp(&a.1.abs())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    let top: Vec<String> = indexed
                        .iter()
                        .take(8)
                        .map(|(i, w)| {
                            format!(
                                "{}={:.3}",
                                ARKIT_BLENDSHAPE_NAMES.get(*i).copied().unwrap_or("?"),
                                w
                            )
                        })
                        .collect();
                    tracing::debug!(
                        target: "timing",
                        stage = "bs_top_channels",
                        pts_ms = f.pts_ms as i64,
                        nonzero = indexed.len() as u64,
                        top = top.join(",").as_str(),
                    );
                }
                {
                    let mut slot = self.first_bs_recv_at.lock();
                    if slot.is_none() {
                        *slot = Some(std::time::Instant::now());
                    }
                }
                self.accept_face(Self::frame_to_pose(&f), &data)?;
            }
            Some(RendererInput::Skeletal(s)) => {
                if let Some(stream) = &s.stream {
                    let displayed = self.renderer.scene_state().office.as_ref()
                        .map_or(u64::MAX, |o| o.avatar.displayed_pts_ms);
                    if let Err(error) = self.scene_delivery.lock().first_frame(&stream.session_id,
                        stream.generation_revision, s.pts_ms, displayed) {
                        self.motion_driver.lock().fail();
                        self.renderer.office_command(super::office::controller::OfficeCommand::MotionError { error: error.clone() });
                        return Err(Error::Execution(error));
                    }
                    self.motion_driver.lock().observe_frame(&stream.session_id, stream.generation_revision, s.pts_ms);
                }
                tracing::debug!(
                    target: "timing",
                    stage = "ccrender_apply_skel",
                    pts_ms = s.pts_ms as i64,
                    root_y = s.root_pos[1] as f64,
                );
                let was_quiet = (*self.last_skel_recv_at.lock()).map_or(true, |t| {
                    std::time::Instant::now().duration_since(t).as_secs() > 1
                });
                if was_quiet {
                    // SMPL joint indices: 16=L_Shoulder, 17=R_Shoulder,
                    // 18=L_Elbow, 19=R_Elbow. A wave should put non-trivial
                    // rotation on R_Shoulder/R_Elbow; a jump on L/R_Hip
                    // (1/2) and root_pos[1]. If these are all near identity
                    // (xyz≈0, w≈1) on the first frame the daemon produced
                    // a no-op clip and the lack of visible motion is
                    // upstream, not in cc_render.
                    let q_rsh = s.joint_quats[17];
                    let q_relb = s.joint_quats[19];
                    let q_lhip = s.joint_quats[1];
                    let q_rhip = s.joint_quats[2];
                    tracing::info!(
                        target: "cc_render",
                        "skeletal: first pose of new stream \
                         pts_ms={} root=({:.3},{:.3},{:.3}) \
                         R_Shoulder=({:.3},{:.3},{:.3},{:.3}) \
                         R_Elbow=({:.3},{:.3},{:.3},{:.3}) \
                         L_Hip=({:.3},{:.3},{:.3},{:.3}) \
                         R_Hip=({:.3},{:.3},{:.3},{:.3})",
                        s.pts_ms,
                        s.root_pos[0], s.root_pos[1], s.root_pos[2],
                        q_rsh[0], q_rsh[1], q_rsh[2], q_rsh[3],
                        q_relb[0], q_relb[1], q_relb[2], q_relb[3],
                        q_lhip[0], q_lhip[1], q_lhip[2], q_lhip[3],
                        q_rhip[0], q_rhip[1], q_rhip[2], q_rhip[3],
                    );
                }
                *self.last_skel_recv_at.lock() = Some(std::time::Instant::now());
                self.renderer.push_skeletal_pose(Some(s));
            }
            Some(RendererInput::Office(cmd)) => self.renderer.office_command(cmd),
            Some(RendererInput::BargeIn) => {
                self.renderer.push_pose(ArkitPose::default());
                if self.renderer.scene_state().office.is_none() {
                    self.renderer.push_skeletal_pose(None);
                }
                let mut anchor = self.anchor_pts_ms.lock().await;
                *anchor = None;
                // Drop buffered audio too so we don't replay stale TTS.
                self.audio_buf.lock().cancel();
            }
            None => {}
        }
        Ok(0)
    }

    async fn process_control_message(
        &self,
        message: RuntimeData,
        _session_id: Option<String>,
    ) -> Result<bool> {
        if matches!(aux_port_of(&message), Some("shutdown")) {
            let renderer = self.renderer.clone();
            tokio::task::spawn_blocking(move || renderer.shutdown())
                .await
                .map_err(|e| Error::Execution(format!("renderer shutdown task failed: {e}")))?
                .map_err(|e| Error::Execution(format!("renderer shutdown failed: {e}")))?;
            return Ok(true);
        }
        if matches!(aux_port_of(&message), Some(BARGE_IN_PORT)) {
            self.renderer.push_pose(ArkitPose::default());
            // Also drop any active skeletal stream so the avatar
            // snaps back to baked rest on barge-in.
            if self.renderer.scene_state().office.is_none() {
                self.renderer.push_skeletal_pose(None);
            }
            let mut anchor = self.anchor_pts_ms.lock().await;
            *anchor = None;
            self.audio_buf.lock().cancel();
            return Ok(true);
        }
        Ok(false)
    }

    // ─── Pacing-domains: ClockedToOutboundMedia ───────────────────────
    //
    // The Bevy thread free-runs at `config.fps` and pushes frames into
    // the renderer's channel. The Pacer ticks this node at the bound
    // wire clock (or wall fallback at `config.fps`); `tick()` drains
    // whatever frames Bevy has produced since the last call and emits
    // them as `RuntimeData::Video`. Decoupling drain from blendshape
    // arrival means video keeps flowing during silence (idle pose) and
    // doesn't piggyback on unrelated heartbeat envelopes.

    fn pacing_nature(&self) -> PacingNature {
        PacingNature::ClockedToOutboundMedia
    }

    fn fallback_rate_hz(&self) -> u32 {
        self.config.fps.max(1)
    }

    async fn tick(
        &self,
        tick: Tick,
        _session_id: Option<String>,
        mut callback: Box<dyn FnMut(RuntimeData) -> std::result::Result<(), Error> + Send>,
    ) -> Result<()> {
        // Gate emission on scene readiness. Bevy loads the GLB
        // asynchronously after spawn — frames produced before
        // `is_ready()` flips contain only the clear color (no avatar
        // geometry). Forwarding them poisons the output stream with
        // a blank pre-roll. We also drain the renderer's internal
        // ring during this phase so the unready-pre-roll frames
        // don't accumulate and back-pressure Bevy's frame channel.
        if !self.renderer.is_ready() {
            let mut scratch = Vec::new();
            self.renderer.drain_frames(&mut scratch);
            if !scratch.is_empty() {
                tracing::trace!(
                    target: "cc_render",
                    "CcRenderNode: dropping {} pre-ready frame(s) (scene still loading)",
                    scratch.len()
                );
            }
            return Ok(());
        }

        // One-shot: announce renderer readiness on output_rx so the
        // upstream driver can open its gate. Without this, the gate
        // could open while `bind.captured` is still false and Bevy's
        // `apply_skeletal_pose` system early-returns — paced pose
        // pushes get burst-collapsed into the watch channel and only
        // the LAST one survives. The smoke binary AND-s this with the
        // first audio2face blendshape before sending `render_ready`.
        if !self.ready_signaled.swap(true, Ordering::AcqRel) {
            tracing::debug!(
                target: "timing",
                stage = "ccrender_ready",
                pts_ms = -1_i64,
            );
            let envelope = RuntimeData::Json(serde_json::json!({
                "kind":   "renderer_ready",
                "source": "cc_render",
            }));
            callback(envelope)?;
        }

        let office_state = self.renderer.scene_state();
        if office_state.office.is_some() {
            callback(RuntimeData::Json(
                serde_json::json!({"kind":"scene_state","state":self.observed_scene()}),
            ))?;
        }
        if let Some(office) = office_state.office.as_ref() {
            if let Some(error) = &office.motion_error {
                if self.motion_driver.lock().fail() {
                    callback(RuntimeData::Json(serde_json::json!({"kind":"motion_error",
                        "error":error,"rejection":office.motion_rejection})))?;
                }
            }
        }
        let displayed = office_state.office.as_ref().map_or(0, |o| o.avatar.displayed_pts_ms);
        let now_ms = self.motion_clock.elapsed().as_millis() as u64;
        if let Some(error) = self.motion_driver.lock().watchdog(displayed, now_ms) {
            callback(RuntimeData::Json(serde_json::json!({"kind":"motion_error", "error":error})))?;
        }
        if let Some(office) = office_state.office.as_ref() {
            if let Some(event) = self.observe_scene_lifecycle(office)? {
                callback(RuntimeData::Json(event))?;
            }
            self.start_pending_scene(office)?;
        }
        {
            // Same lock order as input submission: the phase and request must
            // remain one transaction even when control arrives during a tick.
            let delivery = self.scene_delivery.lock();
            let task = self.scene_task.lock();
            let mut driver = self.motion_driver.lock();
            // An ACK-complete batch may not yet be reflected in the renderer's
            // observation. Give its queued prefix time to become visible before
            // starting another old-phase generation and starving the handoff.
            let awaiting_drain = delivery.pending.as_ref().is_some_and(|(goal, _)| driver.generated_until().is_some_and(|end|
                super::office::scene_delivery::SceneDelivery::request_anchor(goal, displayed, end).is_some()));
            let goal = if awaiting_drain { None } else { driver.next_request(displayed, now_ms) };
            let reserved = goal.as_ref().is_some_and(|goal| delivery.reserved_for(goal));
            drop(driver);
            drop(delivery);
            if let Some(goal) = goal {
                let output = if let Some(task) = task.as_ref() {
                    let office = office_state.office.as_ref().ok_or_else(|| Error::Execution("office context unavailable".into()))?;
                    let revision = self.scene_context_revision.fetch_add(1, Ordering::Relaxed) + 1;
                    let snapshot = super::office::scene_context::SceneSnapshot::from_office(office, goal.session_id(), revision)
                        .map_err(Error::Execution)?;
                    let caption = task.caption(office);
                    let mut signer = self.scene_signer.lock();
                    signer.as_mut().ok_or_else(|| Error::Execution("scene signer unavailable".into()))?.0
                        .sign(&goal, &snapshot, task.phase_name(), &caption, "right").map_err(Error::Execution)?
                } else { serde_json::to_value(&goal).unwrap() };
                if let Some(outcome) = goal.scene_outcome().filter(|o| o.operation == super::office::motion_intent::SceneOperation::Sit) {
                    self.renderer.office_command(super::office::controller::OfficeCommand::AuthorizeNativeSeat {
                        session_id: goal.session_id().into(), generation_revision: goal.revision(), target_id: outcome.target_id.clone(),
                    });
                }
                if goal.scene_outcome().is_none() && !reserved {
                    self.renderer.office_command(super::office::controller::OfficeCommand::NativeMotionRevision {
                        generation_revision: goal.revision(),
                    });
                }
                callback(RuntimeData::Json(output))?;
            }
        }
        let mut frames = Vec::new();
        self.renderer.drain_frames(&mut frames);
        let buf_samples = self.audio_buf.lock().samples.len() as u64;
        tracing::debug!(
            target: "timing",
            stage = "ccrender_tick",
            pts_ms = -1_i64,
            drained = frames.len() as u64,
            audio_buf_samples = buf_samples,
        );
        if frames.is_empty() && !self.config.realtime_mode {
            return Ok(());
        }
        tracing::debug!(
            target: "cc_render",
            "CcRenderNode: drained {} frame(s) from Bevy",
            frames.len()
        );

        // Realtime fast path: this is the live-WebRTC mode. Behaviour:
        //
        //   - Always emit Video frames once the renderer is ready
        //     (so the avatar/scene is visible during idle / model
        //     warmup, not just during active TTS).
        //   - Drain the audio buffer (fed by `audio2face_lipsync →
        //     live2d_render`) at the V emission rate and emit a
        //     paired Audio chunk per V frame. Frame counter drives
        //     both pts streams, so V_n and A_n carry timestamps that
        //     advance together at content rate.
        //   - When the audio buffer is empty (idle / between
        //     utterances) emit V only — never silence-pad. WebRTC's
        //     audio jitter buffer handles the gap.
        //
        // This is the "single pacer" pattern: Audio2Face emits audio
        // + blendshapes as fast as it can; CcRenderNode buffers both
        // and emits paired V+A at content rate. No sleep-based
        // pacing anywhere. WebRTC RTP timestamps stay in lockstep
        // because they're derived from the same `frame_number`
        // counter on the V side and the same `samples_emitted`
        // counter on the A side.
        if self.config.realtime_mode {
            let fps = self.config.fps.max(1) as u64;
            let mut emitted = 0u64;

            // Pacing: one tick = one outbound media frame.
            //
            // `pacing_nature() = ClockedToOutboundMedia` ticks this node
            // at the encoded RTP cadence (~fps). Bevy free-runs on its
            // own thread and pushes into a 64-slot ring; if it renders
            // faster than tick drains (typical on a fast GPU rendering
            // a 512² avatar — easily 1000+ fps), the ring fills with
            // stale renders.
            //
            // Earlier we emitted every drained frame, which amplified
            // the source rate to N×fps and then asked the encoder /
            // dispatcher to absorb it. The encoder couldn't, the
            // dispatcher logged thousands of "video shard full" drops,
            // and we burned CPU on YUV conversion + memcpy for frames
            // that were going to be dropped anyway.
            //
            // Instead, take the *latest* rendered frame per tick — the
            // freshest pose / facial expression — and discard the rest
            // as stale. If Bevy is keeping up there's at most one frame
            // here so this is a no-op; if it's overrunning we drop the
            // backlog at the source where it's cheapest.
            let stale_dropped = frames.len().saturating_sub(1);
            let latest = frames.pop();
            if stale_dropped > 0 {
                let total = self
                    .stale_frames_dropped
                    .fetch_add(stale_dropped as u64, Ordering::Relaxed)
                    + stale_dropped as u64;
                // Log on first stale drop and then once per ~1s of
                // ticks (fps ticks ≈ 1 s). The metric is cumulative so
                // we can spot-check growth without spamming stderr.
                if total == stale_dropped as u64 || total % (fps * 10) < stale_dropped as u64 {
                    tracing::debug!(
                        target: "cc_render",
                        "CcRenderNode: dropped {} stale Bevy frame(s) this tick (cumulative {}); Bevy is rendering faster than outbound cadence",
                        stale_dropped, total
                    );
                }
            }
            // Speech content advances only as samples are played. Queue facial
            // frames until their content deadline; fast A2F inference must never
            // overwrite an entire sentence in the renderer watch channel.
            let face_was_started = {
                let mut audio = self.audio_buf.lock();
                let started = audio.face_started;
                let content_ms = if audio.sample_rate == 0 {
                    0
                } else {
                    audio.samples_emitted * 1000 / audio.sample_rate as u64
                };
                let mut pose = None;
                while audio.faces.front().is_some_and(|p| p.pts_ms <= content_ms) {
                    pose = audio.faces.pop_front();
                }
                if let Some(pose) = pose {
                    self.renderer.push_pose(pose);
                    audio.face_started = true;
                }
                started
            };

            {
                let frame_pts_us = tick.pts_us;

                // Pull this frame's audio chunk from the buffer.
                // Only emits A when the buffer holds REAL samples
                // (not silence-pad) — during idle the audio sink
                // simply receives no Audio events for this tick.
                let audio_chunk = {
                    let mut ab = self.audio_buf.lock();
                    if ab.sample_rate == 0 || ab.samples.is_empty() || !face_was_started {
                        None
                    } else {
                        // Exactly one frame of samples, including the fractional
                        // remainder at rates not divisible by fps. Idle time must
                        // never turn into a catch-up burst of queued speech.
                        let want = audio_samples_per_tick(tick.frame_idx, ab.sample_rate, fps);
                        if want == 0 {
                            None
                        } else {
                            let until_end =
                                ab.speech_ends.front().map_or(usize::MAX, |(end, _)| {
                                    end.saturating_sub(ab.samples_emitted) as usize
                                });
                            let real_take = want.min(ab.samples.len()).min(until_end);
                            if real_take == 0 {
                                None
                            } else {
                                let mut chunk: Vec<f32> = Vec::with_capacity(real_take);
                                for _ in 0..real_take {
                                    chunk.push(ab.samples.pop_front().expect("len checked"));
                                }
                                let pts_us = frame_pts_us;
                                ab.samples_emitted += chunk.len() as u64;
                                Some((
                                    chunk,
                                    ab.sample_rate,
                                    ab.channels.max(1),
                                    pts_us,
                                    ab.speech_epoch,
                                ))
                            }
                        }
                    }
                };

                if let Some(f) = latest {
                    let frame_number = self.frame_counter.fetch_add(1, Ordering::AcqRel);
                    callback(RuntimeData::Video {
                        pixel_data: f.pixels,
                        width: f.width,
                        height: f.height,
                        format: PixelFormat::Rgba32,
                        codec: None,
                        frame_number,
                        timestamp_us: frame_pts_us,
                        is_keyframe: true,
                        stream_id: Some(self.config.video_stream_id.clone()),
                        arrival_ts_us: None,
                    })?;
                    emitted += 1;
                }

                if let Some((chunk, sr, ch, pts_us, epoch)) = audio_chunk {
                    let audio = RuntimeData::Audio {
                        samples: AudioSamples::Vec(chunk),
                        sample_rate: sr,
                        channels: ch,
                        stream_id: None,
                        timestamp_us: Some(pts_us),
                        arrival_ts_us: None,
                        metadata: Some(serde_json::json!({"speech_epoch":epoch})),
                    };
                    callback(audio)?;
                    emitted += 1;
                }
                // Forward utterance boundaries only after their audio has played.
                // Downstream resamplers can now flush the exact final samples.
                let ends = {
                    let mut audio = self.audio_buf.lock();
                    let mut ends = Vec::new();
                    while audio
                        .speech_ends
                        .front()
                        .is_some_and(|(end, _)| *end <= audio.samples_emitted)
                    {
                        ends.push(audio.speech_ends.pop_front().unwrap().1);
                    }
                    ends
                };
                for end in ends {
                    callback(RuntimeData::Json(end))?;
                }
            }
            let total = self.frame_counter.load(Ordering::Relaxed);
            if total == 1 || total % 60 == 0 {
                tracing::info!(
                    target: "cc_render",
                    "CcRenderNode: total Video frames emitted (realtime) = {}",
                    total
                );
            } else {
                tracing::trace!(
                    target: "cc_render",
                    "CcRenderNode: emitted {} Video+Audio frame(s) this tick (realtime)",
                    emitted
                );
            }
            return Ok(());
        }

        // Decide whether this tick should emit at all. We emit while
        // ANY input stream is "live" — either the audio buffer has
        // samples to consume, or audio/skeletal_pose was received
        // recently (within `TAIL_QUIET`). When audio is exhausted but
        // motion is still streaming, we emit V + silence-padded A so
        // the body animation plays through to its end without the
        // recorded MP4 cutting off mid-motion.
        let now = std::time::Instant::now();
        let audio_buf_has_samples = {
            let ab = self.audio_buf.lock();
            ab.sample_rate != 0 && !ab.samples.is_empty()
        };
        let last_audio = *self.last_audio_recv_at.lock();
        let last_skel = *self.last_skel_recv_at.lock();
        let audio_recent = last_audio
            .map(|t| now.duration_since(t) < TAIL_QUIET)
            .unwrap_or(false);
        let skel_recent = last_skel
            .map(|t| now.duration_since(t) < TAIL_QUIET)
            .unwrap_or(false);
        let audio_age_ms = last_audio
            .map(|t| now.duration_since(t).as_millis() as i64)
            .unwrap_or(-1);
        let skel_age_ms = last_skel
            .map(|t| now.duration_since(t).as_millis() as i64)
            .unwrap_or(-1);
        // Skel queue depth: poses cc_render has enqueued but Bevy
        // hasn't drained yet. While > 0 the motion is still mid-
        // animation — keep emitting so it plays to its end even after
        // the last `push_skeletal_pose` was a while ago (especially
        // important on slow GPUs where Bevy drains slower than the
        // gate paces inputs).
        let skel_queue_depth = self.renderer.skeletal_queue_depth();
        let motion_in_flight = skel_queue_depth > 0;
        tracing::debug!(
            target: "timing",
            stage = "ccrender_tick_gate",
            pts_ms = -1_i64,
            audio_buf = audio_buf_has_samples as u64,
            audio_recent = audio_recent as u64,
            skel_recent = skel_recent as u64,
            skel_queue = skel_queue_depth as u64,
            audio_age_ms = audio_age_ms,
            skel_age_ms = skel_age_ms,
            frames_drained = frames.len() as u64,
        );
        // Pre-emission gate: at session start we need to wait for the
        // FIRST audio chunk to arrive (so we know the WAV format and
        // so we don't emit silence ahead of speech). After that, we
        // emit as long as anything is recent OR the buffer has samples
        // OR Bevy still has motion poses to play through.
        if last_audio.is_none() {
            return Ok(());
        }
        // Lipsync gate: hold V emission until the first blendshape has
        // been received AND Bevy has had at least one frame interval to
        // apply it. Without this gate, V_0 captures the default rest
        // pose (because LastAppliedPts is 0 by default) while the audio
        // buffer drains content_time=0; the mouth then lags audio by
        // the gate→bevy_apply latency (~900 ms in practice) for the
        // rest of the stream.
        let first_bs = *self.first_bs_recv_at.lock();
        match first_bs {
            None => {
                tracing::debug!(
                    target: "cc_render",
                    "tick: holding V emit — no blendshape received yet"
                );
                return Ok(());
            }
            Some(t) if now.duration_since(t) < FIRST_BS_APPLY_GRACE => {
                tracing::debug!(
                    target: "cc_render",
                    "tick: holding V emit — first bs received {} ms ago, grace {} ms",
                    now.duration_since(t).as_millis(),
                    FIRST_BS_APPLY_GRACE.as_millis(),
                );
                return Ok(());
            }
            _ => {}
        }
        if !audio_buf_has_samples && !audio_recent && !skel_recent && !motion_in_flight {
            tracing::info!(
                target: "cc_render",
                "tick: streams quiet (audio_age={}ms, skel_age={}ms, skel_queue={}) ≥ {}ms — stopping emission",
                audio_age_ms, skel_age_ms, skel_queue_depth, TAIL_QUIET.as_millis()
            );
            return Ok(());
        }
        let mut emitted = 0u64;
        let fps = self.config.fps.max(1) as u64;
        // Bevy may have rendered multiple frames since the last tick.
        // For lipsync we only emit pairs as long as audio is available;
        // any frames left after the buffer drains are dropped (older
        // poses don't matter — the latest is always preferred).
        //
        // First-emit lipsync alignment: the visible mouth pose at the
        // first captured frame has a *content* pts of `f.pts_ms` (set by
        // `LastAppliedPts` on the Bevy side). But the audio buffer holds
        // samples from content_time=0. If we drained naively, V_0 would
        // show the mouth at content_time=warmup_gap_ms while A_0 plays
        // content_time=0 — speech ahead of mouth movement by the entire
        // warmup gap. Fix: on the first emission with a non-zero pose
        // pts, pop that many samples off the front of the buffer (no
        // bump to `samples_emitted`) so wav sample 0 aligns with the
        // visible mouth's content time.
        {
            let mut anchor = self.anchor_pts_ms.lock().await;
            if anchor.is_none() {
                if let Some(f0) = frames.first() {
                    let mut ab = self.audio_buf.lock();
                    if ab.sample_rate != 0 && f0.pts_ms > 0 {
                        let skip = ((f0.pts_ms as u128 * ab.sample_rate as u128) / 1000) as usize;
                        let actual = skip.min(ab.samples.len());
                        for _ in 0..actual {
                            ab.samples.pop_front();
                        }
                        tracing::debug!(
                            target: "timing",
                            stage = "ccrender_lipsync_align",
                            pts_ms = f0.pts_ms as i64,
                            samples_skipped = actual as u64,
                            samples_remaining = ab.samples.len() as u64,
                        );
                    }
                    *anchor = Some(f0.pts_ms);
                }
            }
        }
        for f in frames {
            // Pull the audio chunk for this frame's content slot. If
            // the buffer has the full chunk → use it. Else, fall back
            // to silence padding (so motion can play to completion
            // even after speech finishes). We only emit silence when
            // motion (or audio) was active recently — the outer gate
            // above already ensures we don't pad before any audio has
            // ever been seen, or after both streams have gone quiet
            // for `TAIL_QUIET`.
            let audio = {
                let mut ab = self.audio_buf.lock();
                if ab.sample_rate == 0 {
                    None
                } else {
                    let frames_so_far = self.frame_counter.load(Ordering::Acquire);
                    let target_total = ((frames_so_far + 1) * ab.sample_rate as u64) / fps;
                    let already = ab.samples_emitted;
                    let want = target_total.saturating_sub(already) as usize;
                    if want == 0 {
                        None
                    } else {
                        // Drain up to `want` real samples from the
                        // head, then pad the remainder with zeros.
                        // This handles three cases in one path:
                        //   - buf has ≥ want → take want real samples
                        //   - buf has < want → take what's there, pad
                        //   - buf empty → all zeros (motion-only tail)
                        // Critically: we ALWAYS pop_front the real
                        // samples we consume, so `samples.is_empty()`
                        // flips true once the real audio is done —
                        // letting the outer gate eventually stop
                        // emission instead of holding `audio_buf=1`
                        // forever.
                        let real_take = want.min(ab.samples.len());
                        let mut chunk: Vec<f32> = Vec::with_capacity(want);
                        for _ in 0..real_take {
                            chunk.push(ab.samples.pop_front().expect("len checked"));
                        }
                        for _ in real_take..want {
                            chunk.push(0.0);
                        }
                        let pts_us = (ab.samples_emitted * 1_000_000) / ab.sample_rate as u64;
                        ab.samples_emitted += chunk.len() as u64;
                        let sr = ab.sample_rate;
                        let ch = ab.channels.max(1);
                        if real_take < want {
                            tracing::debug!(
                                target: "cc_render",
                                "tick: audio underrun (real={}/{} pad={}) — silence-padded chunk",
                                real_take,
                                want,
                                want - real_take,
                            );
                        }
                        Some(RuntimeData::Audio {
                            samples: AudioSamples::Vec(chunk),
                            sample_rate: sr,
                            channels: ch,
                            stream_id: None,
                            timestamp_us: Some(pts_us),
                            arrival_ts_us: None,
                            metadata: None,
                        })
                    }
                }
            };
            let Some(audio) = audio else {
                tracing::debug!(
                    target: "cc_render",
                    "tick: no audio format yet — skipping frame"
                );
                break;
            };

            // V wire pts: derive from monotonic emit counter so the
            // stream stays evenly paced at `fps` regardless of whether
            // pose pts advanced this frame (Bevy may render multiple
            // captures against the same applied pose — we don't want
            // wire pts to stall). Audio uses `samples_emitted` (also
            // monotonic) so V and A wire pts both start at 0 and grow
            // in lockstep.
            let frame_number = self.frame_counter.fetch_add(1, Ordering::AcqRel);
            let frame_pts_us = (frame_number * 1_000_000) / fps;
            tracing::debug!(
                target: "timing",
                stage = "ccrender_emit_v",
                pts_ms = (frame_pts_us / 1000) as i64,
                frame_idx = frame_number,
                pose_pts_ms = f.pts_ms as i64,
            );
            tracing::debug!(
                target: "cc_render",
                "tick: emit V+A pair #{frame_number}: video_pts_us={frame_pts_us}, audio samples=…",
            );
            let video = RuntimeData::Video {
                pixel_data: f.pixels,
                width: f.width,
                height: f.height,
                format: PixelFormat::Rgba32,
                codec: None,
                frame_number,
                timestamp_us: frame_pts_us,
                is_keyframe: true,
                stream_id: Some(self.config.video_stream_id.clone()),
                arrival_ts_us: None,
            };
            callback(video)?;
            emitted += 1;
            // Log A emission AFTER pulling samples (emit pts_us is on the
            // Audio struct). Extract before move.
            if let RuntimeData::Audio {
                samples,
                timestamp_us,
                ..
            } = &audio
            {
                tracing::debug!(
                    target: "timing",
                    stage = "ccrender_emit_a",
                    pts_ms = timestamp_us.map(|u| (u / 1000) as i64).unwrap_or(-1),
                    samples = samples.len() as u64,
                );
            }
            callback(audio)?;
            emitted += 1;
        }
        let total = self.frame_counter.load(Ordering::Relaxed);
        if total == 1 || total % 60 == 0 {
            tracing::info!(
                target: "cc_render",
                "CcRenderNode: total Video frames emitted = {}",
                total
            );
        } else {
            tracing::trace!(
                target: "cc_render",
                "CcRenderNode: emitted {} Video frame(s) this tick",
                emitted
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod speech_clock_tests {
    use super::*;
    #[test]
    fn idle_never_becomes_audio_catchup() {
        assert_eq!(audio_samples_per_tick(300, 16000, 30), 533);
        assert_eq!(
            (300..330)
                .map(|f| audio_samples_per_tick(f, 16000, 30))
                .sum::<usize>(),
            16000
        );
    }
}

#[cfg(test)]
#[path = "node_navigation_tests.rs"]
mod navigation_tests;

#[cfg(test)]
mod realtime_speech_tests {
    use super::*;
    use crate::cc_render::renderer::{RenderedFrame, SkeletalPose};
    #[derive(Default)]
    struct MockRenderer {
        poses: parking_lot::Mutex<Vec<ArkitPose>>,
        skeletal: parking_lot::Mutex<Vec<SkeletalPose>>,
        miss_capture: AtomicBool,
        office: parking_lot::Mutex<Option<super::super::office::controller::OfficeSnapshot>>,
        commands: parking_lot::Mutex<Vec<super::super::office::controller::OfficeCommand>>,
        huds: parking_lot::Mutex<Vec<Option<super::super::office::home_hud::HomeHud>>>,
    }
    impl Renderer for MockRenderer {
        fn home_hud(&self, hud: Option<super::super::office::home_hud::HomeHud>) {
            self.huds.lock().push(hud);
        }
        fn scene_state(&self) -> super::super::renderer::SceneState {
            super::super::renderer::SceneState { office: self.office.lock().clone(), ..Default::default() }
        }
        fn office_command(&self, cmd: super::super::office::controller::OfficeCommand) {
            self.commands.lock().push(cmd);
        }
        fn push_pose(&self, pose: ArkitPose) {
            self.poses.lock().push(pose);
        }
        fn push_skeletal_pose(&self, pose: Option<SkeletalPose>) {
            if let Some(pose) = pose {
                self.skeletal.lock().push(pose);
            }
        }
        fn drain_frames(&self, sink: &mut Vec<RenderedFrame>) {
            if self.miss_capture.load(Ordering::Relaxed) {
                return;
            }
            sink.push(RenderedFrame {
                pixels: vec![0; 4],
                width: 1,
                height: 1,
                pts_ms: self.poses.lock().last().map_or(0, |p| p.pts_ms),
            });
        }
    }
    #[test]
    fn scene_capability_needs_startup_key_and_is_not_replaceable() {
        use std::io::Write;
        let mut key = tempfile::NamedTempFile::new().unwrap(); key.write_all(&[9;32]).unwrap();
        let mut avatar = tempfile::NamedTempFile::new().unwrap(); avatar.write_all(b"avatar").unwrap();
        let digest: String = hmac_sha256::Hash::hash(b"avatar").iter().map(|b| format!("{b:02x}")).collect();
        let mut value = serde_json::json!({"kind":"motion_capabilities","scene_sampler":{
            "protocol":3,"context_version":1,"policy":"model-owned-scene-v1",
            "avatar_sha256":digest,"profile_identity":"0".repeat(64),
            "authenticated_context":true,"server_instance":"1".repeat(32)}});
        let baseline = CcRenderNode::with_renderer(CcRenderConfig::default(), Arc::new(MockRenderer::default()));
        baseline.accept_motion_control(&RuntimeData::Json(value.clone())).unwrap();
        assert!(baseline.scene_signer.lock().is_none());
        let node = CcRenderNode::with_renderer(CcRenderConfig {
            glb_path: avatar.path().into(), scene_key_file: Some(key.path().into()), ..Default::default()
        }, Arc::new(MockRenderer::default()));
        node.accept_motion_control(&RuntimeData::Json(value.clone())).unwrap();
        node.accept_motion_control(&RuntimeData::Json(value.clone())).unwrap(); // Idempotent, no sequence reset.
        assert!(node.scene_signer.lock().is_some());
        value["scene_sampler"]["server_instance"] = "2".repeat(32).into();
        assert!(node.accept_motion_control(&RuntimeData::Json(value)).is_err());
    }

    #[test]
    fn home_hud_is_display_only_and_closed() {
        let renderer = Arc::new(MockRenderer::default());
        let node = CcRenderNode::with_renderer(CcRenderConfig::default(), renderer.clone());
        let hud = super::super::office::home_hud::fixture();
        assert!(node.accept_motion_control(&RuntimeData::Json(hud.clone())).unwrap());
        assert!(node.accept_motion_control(&RuntimeData::Json(
            serde_json::json!({"kind":"home_hud","version":1,"hud":null}))).unwrap());
        let mut invalid = hud;
        invalid["hud"]["goal"] = serde_json::json!({"operation":"sit","target_id":"chair"});
        assert!(node.accept_motion_control(&RuntimeData::Json(invalid)).is_err());
        let huds = renderer.huds.lock();
        assert_eq!(huds.len(), 2);
        assert!(huds[0].is_some() && huds[1].is_none());
        // Overlay state never becomes a goal, lifecycle or office command.
        assert!(node.motion_driver.lock().current_goal().is_none());
        assert!(node.scene_task.lock().is_none() && node.scene_delivery.lock().pending.is_none());
        assert!(renderer.commands.lock().is_empty());
    }

    fn node_with_pending_scene() -> (CcRenderNode, serde_json::Value) {
        use super::super::office::controller::OfficeController;
        let renderer = Arc::new(MockRenderer::default());
        *renderer.office.lock() = Some(OfficeController::default().snapshot());
        let node = CcRenderNode::with_renderer(CcRenderConfig::default(), renderer);
        let goal = serde_json::json!({"kind":"motion_goal","version":2,"session_id":"replacement",
            "generation_revision":1,"pts_ms":0,"seed":42,"goal":{"description":"A person stands still."}});
        node.accept_motion_control(&RuntimeData::Json(goal.clone())).unwrap();
        node.motion_driver.lock().next_request(0, 0).unwrap();
        let mut pending = goal.clone();
        pending["version"] = 3.into();
        pending["generation_revision"] = 3.into();
        pending["goal"] = serde_json::json!({"operation":"navigate","target_id":"chair"});
        node.scene_delivery.lock().pending = Some((serde_json::from_value(pending).unwrap(), false));
        (node, goal)
    }

    #[test]
    fn pending_scene_is_reported_as_requested_owner_not_displayed_completion() {
        let (node, _) = node_with_pending_scene();
        let scene = node.observed_scene();
        assert_eq!(scene["office"]["motion"]["generation_revision"], 3);
        assert_eq!(scene["office"]["motion"]["goal"]["operation"], "navigate");
        assert!(scene["office"]["scene_lifecycle"].is_null());
        assert_eq!(node.motion_driver.lock().current_goal().unwrap().revision(), 1);
    }

    #[test]
    fn semantic_replacement_respects_pending_revision_and_in_flight_ack() {
        let (node, mut goal) = node_with_pending_scene();
        for revision in [2, 3] {
            goal["generation_revision"] = revision.into();
            assert!(node.accept_motion_control(&RuntimeData::Json(goal.clone())).is_err());
            assert_eq!(node.scene_delivery.lock().pending.as_ref().unwrap().0.revision(), 3);
            assert_eq!(node.motion_driver.lock().current_goal().unwrap().revision(), 1);
        }
        goal["generation_revision"] = 4.into();
        node.accept_motion_control(&RuntimeData::Json(goal)).unwrap();
        assert_eq!(node.scene_delivery.lock().pending.as_ref().unwrap().0.revision(), 4);
        {
            let mut driver = node.motion_driver.lock();
            assert_eq!(driver.current_goal().unwrap().revision(), 1);
            assert!(driver.next_request(1500, 100).is_none());
            driver.acknowledge("replacement", 3, 0); // Wrong request cannot release the old slot.
            assert!(driver.in_flight());
            driver.observe_frame("replacement", 1, 4000);
            driver.acknowledge("replacement", 1, 0);
        }
        let mut office = super::super::office::controller::OfficeController::default().snapshot();
        office.avatar.ready = true;
        office.avatar.displayed_pts_ms = 1500;
        office.avatar.buffered_until_pts_ms = Some(4000);
        node.start_pending_scene(&office).unwrap();
        assert!(node.scene_delivery.lock().pending.is_none());
        let mut driver = node.motion_driver.lock();
        assert_eq!(driver.generated_until(), None); // Old output cannot own the new horizon.
        assert!(node.scene_delivery.lock().reserved_for(driver.current_goal().unwrap()));
        assert_eq!(driver.next_request(1500, 200).unwrap().revision(), 4);
    }

    #[tokio::test]
    async fn pending_navigation_keeps_native_producer_then_signs_reserved_handoff() {
        use std::io::Write;
        use super::super::office::{controller::{OfficeController, OfficeCommand}, timeline::StreamIdentity};
        let mut key = tempfile::NamedTempFile::new().unwrap(); key.write_all(&[9;32]).unwrap();
        let mut avatar = tempfile::NamedTempFile::new().unwrap(); avatar.write_all(b"avatar").unwrap();
        let renderer = Arc::new(MockRenderer::default());
        let mut office = OfficeController::default().snapshot();
        office.avatar.ready = true; office.avatar.position_valid = true;
        office.avatar.pelvis = [3., 1., 3.];
        office.avatar.displayed_pts_ms = 1500; office.avatar.buffered_until_pts_ms = Some(2000);
        office.avatar.native_stream = Some(StreamIdentity {session_id:"test".into(), generation_revision:1,
            skeleton_id:"smpl22".into(), root_mode:"world".into()});
        *renderer.office.lock() = Some(office);
        let node = CcRenderNode::with_renderer(CcRenderConfig {
            glb_path:avatar.path().into(), scene_key_file:Some(key.path().into()), ..Default::default()
        }, renderer.clone());
        let digest: String = hmac_sha256::Hash::hash(b"avatar").iter().map(|b| format!("{b:02x}")).collect();
        node.accept_motion_control(&RuntimeData::Json(serde_json::json!({"kind":"motion_capabilities",
            "scene_sampler":{"protocol":3,"context_version":1,"policy":"model-owned-scene-v1",
            "avatar_sha256":digest,"profile_identity":"0".repeat(64),"authenticated_context":true,
            "server_instance":"1".repeat(32)}}))).unwrap();
        let baseline = serde_json::json!({"kind":"motion_goal","version":2,"session_id":"test",
            "generation_revision":1,"pts_ms":0,"seed":42,"goal":{"description":"A person stands still."}});
        node.accept_motion_control(&RuntimeData::Json(baseline.clone())).unwrap();
        {
            let mut driver = node.motion_driver.lock();
            driver.next_request(0, 0).unwrap(); driver.observe_frame("test", 1, 2000);
            driver.acknowledge("test", 1, 0);
        }
        let mut goal = baseline; goal["version"] = 3.into(); goal["generation_revision"] = 2.into();
        goal["goal"] = serde_json::json!({"operation":"navigate","target_id":"chair"});
        node.accept_motion_control(&RuntimeData::Json(goal)).unwrap();
        let outputs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let out = outputs.clone();
        node.tick(Tick {clock_id:"avatar".into(),pts_us:0,frame_idx:0,deadline_us:0}, None,
            Box::new(move |v| {out.lock().push(v); Ok(())})).await.unwrap();
        assert!(outputs.lock().iter().any(|v| matches!(v, RuntimeData::Json(j) if j["kind"] == "motion_goal" && j["version"] == 2)));
        assert!(node.scene_task.lock().is_none());
        {
            let mut driver = node.motion_driver.lock(); driver.observe_frame("test", 1, 4000);
            driver.acknowledge("test", 1, 1500);
        }
        {
            let mut office = renderer.office.lock(); let office = office.as_mut().unwrap();
            office.avatar.displayed_pts_ms = 1566; office.avatar.buffered_until_pts_ms = Some(4000);
        }
        let out = outputs.clone();
        node.tick(Tick {clock_id:"avatar".into(),pts_us:33000,frame_idx:1,deadline_us:0}, None,
            Box::new(move |v| {out.lock().push(v); Ok(())})).await.unwrap();
        let outputs = outputs.lock();
        let envelope = outputs.iter().find_map(|v| match v {
            RuntimeData::Json(j) if j["kind"] == "scene_step" => Some(j), _ => None
        }).unwrap();
        let payload = envelope["payload"].as_str().unwrap();
        let job: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(job["request"]["pts_ms"], 1566);
        assert_eq!(job["snapshot"]["displayed_pts_ms"], 1566);
        assert_eq!(job["phase"], "arrive");
        let signature: String = hmac_sha256::HMAC::mac(payload.as_bytes(), [9;32]).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(envelope["signature"], signature);
        assert!(renderer.commands.lock().iter().any(|c| matches!(c,
            OfficeCommand::NativeSceneHandoff { generation_revision:2, history_pts_ms:4000 })));
        assert!(!renderer.commands.lock().iter().any(|c| matches!(c,
            OfficeCommand::NativeMotionRevision { generation_revision:2 })));
        drop(outputs);
        {
            let mut driver = node.motion_driver.lock();
            driver.observe_frame("test", 2, 5800);
            driver.acknowledge("test", 2, 1566);
            node.scene_delivery.lock().pending = Some((driver.current_goal().unwrap().next_revision().unwrap(), false));
        }
        {
            let mut state = renderer.office.lock(); let office = state.as_mut().unwrap();
            office.avatar.displayed_pts_ms = 3500;
            office.avatar.buffered_until_pts_ms = Some(5750);
            office.avatar.native_stream.as_mut().unwrap().generation_revision = 2;
        }
        node.tick(Tick {clock_id:"avatar".into(),pts_us:66000,frame_idx:2,deadline_us:0}, None,
            Box::new(|_| Ok(()))).await.unwrap();
        assert!(!node.motion_driver.lock().in_flight(), "partial drain must not launch another old-phase request");
        {
            let mut state = renderer.office.lock(); let office = state.as_mut().unwrap();
            office.avatar.displayed_pts_ms = 3533;
            office.avatar.buffered_until_pts_ms = Some(5800);
        }
        node.tick(Tick {clock_id:"avatar".into(),pts_us:99000,frame_idx:3,deadline_us:0}, None,
            Box::new(|_| Ok(()))).await.unwrap();
        assert_eq!(node.motion_driver.lock().current_goal().unwrap().revision(), 3);
    }

    #[tokio::test]
    async fn semantic_input_emits_native_goal_not_legacy_motion() {
        let node = CcRenderNode::with_renderer(CcRenderConfig::default(), Arc::new(MockRenderer::default()));
        let goal = serde_json::json!({"kind":"motion_goal", "version":2,
            "session_id":"native-test", "generation_revision":1, "pts_ms":0,
            "seed":42, "goal":{"description":"A person stands up from a squat."}});
        node.process_streaming(RuntimeData::Json(goal.clone()), None, |_| Ok(())).await.unwrap();
        let outputs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let out = outputs.clone();
        node.tick(Tick { clock_id:"avatar".into(), pts_us:0, frame_idx:0, deadline_us:0 },
            None, Box::new(move |data| { out.lock().push(data); Ok(()) })).await.unwrap();
        assert!(outputs.lock().iter().any(|v| matches!(v, RuntimeData::Json(value) if value == &goal)));
        assert!(node.process_streaming(RuntimeData::Json(serde_json::json!({"kind":"motion_request"})),
            None, |_| Ok(())).await.is_err());
    }

    #[tokio::test]
    async fn python_json_channel_delivers_skeletal_frames() {
        let renderer = Arc::new(MockRenderer::default());
        let node = CcRenderNode::with_renderer(CcRenderConfig::default(), renderer.clone());
        let envelope = serde_json::json!({"kind":"skeletal_pose", "pts_ms":50,
            "joint_quats_xyzw":vec![[0.0,0.0,0.0,1.0];22], "root_pos":[0.0,1.0,0.0]});
        let input = RuntimeData::Text(remotemedia_plugin_sdk::types::tag_text_str(
            &envelope.to_string(),
            "json",
        ));
        node.process_streaming(input, None, |_| Ok(()))
            .await
            .unwrap();
        assert_eq!(renderer.skeletal.lock().len(), 1);
        assert_eq!(renderer.skeletal.lock()[0].pts_ms, 50);
    }
    #[tokio::test]
    async fn idle_then_speech_keeps_samples_and_uses_shared_presentation_time() {
        let renderer = Arc::new(MockRenderer::default());
        let node = CcRenderNode::with_renderer(
            CcRenderConfig {
                realtime_mode: true,
                ..Default::default()
            },
            renderer.clone(),
        );
        node.frame_counter.store(300, Ordering::Release);
        {
            let mut audio = node.audio_buf.lock();
            audio.sample_rate = 16000;
            audio.channels = 1;
            audio.samples.extend(vec![0.5; 16000]);
            for pts_ms in [0, 33, 66, 100] {
                audio.faces.push_back(ArkitPose {
                    pts_ms,
                    ..Default::default()
                });
            }
        }
        let outputs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        for index in 300..304 {
            let out = outputs.clone();
            node.tick(
                Tick {
                    clock_id: "avatar".into(),
                    pts_us: index * 1_000_000 / 30,
                    frame_idx: index,
                    deadline_us: 0,
                },
                None,
                Box::new(move |data| {
                    out.lock().push(data);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        }
        let out = outputs.lock();
        let audio: Vec<_> = out
            .iter()
            .filter_map(|d| {
                if let RuntimeData::Audio {
                    samples,
                    timestamp_us,
                    ..
                } = d
                {
                    Some((samples.len(), timestamp_us.unwrap()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(audio.len(), 3);
        assert!(audio.iter().all(|(n, _)| *n <= 534));
        assert_eq!(audio[0].1, 301 * 1_000_000 / 30);
        assert_eq!(
            node.audio_buf.lock().samples.len(),
            16000 - audio.iter().map(|(n, _)| n).sum::<usize>()
        );
        assert!(renderer.poses.lock().len() >= 2);
    }

    #[tokio::test]
    async fn missed_video_capture_does_not_pause_speech_clock() {
        let renderer = Arc::new(MockRenderer::default());
        renderer.miss_capture.store(true, Ordering::Relaxed);
        let node = CcRenderNode::with_renderer(
            CcRenderConfig {
                realtime_mode: true,
                ..Default::default()
            },
            renderer,
        );
        {
            let mut audio = node.audio_buf.lock();
            audio
                .push_audio(&vec![0.5; 16000], 16000, 1, Some(0))
                .unwrap();
            audio.face_started = true;
        }
        let outputs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        for index in 0..30 {
            let outputs = outputs.clone();
            node.tick(
                Tick {
                    clock_id: "avatar".into(),
                    pts_us: index * 1_000_000 / 30,
                    frame_idx: index,
                    deadline_us: 0,
                },
                None,
                Box::new(move |out| {
                    outputs.lock().push(out);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        }
        let mut total = 0;
        for (index, out) in outputs
            .lock()
            .iter()
            .filter(|out| !matches!(out, RuntimeData::Json(_)))
            .enumerate()
        {
            let RuntimeData::Audio {
                samples,
                timestamp_us,
                ..
            } = out
            else {
                panic!("unexpected video");
            };
            assert_eq!(*timestamp_us, Some(index as u64 * 1_000_000 / 30));
            total += samples.len();
        }
        assert_eq!(total, 16000);
        assert!(node.audio_buf.lock().samples.is_empty());
    }

    #[tokio::test]
    async fn speech_end_follows_playback_and_precedes_next_utterance() {
        let node = CcRenderNode::with_renderer(
            CcRenderConfig {
                realtime_mode: true,
                ..Default::default()
            },
            Arc::new(MockRenderer::default()),
        );
        {
            let mut audio = node.audio_buf.lock();
            audio
                .push_audio(&vec![0.25; 100], 16000, 1, Some(0))
                .unwrap();
            audio.face_started = true;
        }
        assert!(node
            .accept_speech_end(&RuntimeData::Json(
                serde_json::json!({"kind":"speech_end", "speech_epoch":0, "pts_us":6250})
            ))
            .unwrap());
        node.audio_buf
            .lock()
            .push_audio(&vec![0.5; 1000], 16000, 1, Some(0))
            .unwrap();
        let outputs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let out = outputs.clone();
        node.tick(
            Tick {
                clock_id: "avatar".into(),
                pts_us: 0,
                frame_idx: 0,
                deadline_us: 0,
            },
            None,
            Box::new(move |data| {
                out.lock().push(data);
                Ok(())
            }),
        )
        .await
        .unwrap();
        let output = outputs.lock();
        let audio_index = output
            .iter()
            .position(|v| matches!(v, RuntimeData::Audio { .. }))
            .unwrap();
        let RuntimeData::Audio {
            samples, metadata, ..
        } = &output[audio_index]
        else {
            unreachable!()
        };
        assert_eq!(samples.len(), 100);
        assert_eq!(metadata.as_ref().unwrap()["speech_epoch"], 0);
        assert!(
            matches!(&output[audio_index+1], RuntimeData::Json(v) if v["kind"] == "speech_end")
        );
        assert_eq!(node.audio_buf.lock().samples.len(), 1000);
    }
}
