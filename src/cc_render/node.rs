//! `CcRenderNode` — streaming-pipeline wrapper around [`super::CcRenderer`].
//!
//! Consumes JSON envelopes (`kind = "blendshapes" | "skeletal_pose" |
//! "barge_in"`) on its reactive input path and emits
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
    /// blendshape arrival. Skips A+V pairing and the audio buffer drain
    /// entirely — audio is routed via a separate sink in real-time
    /// pipelines (e.g. WebRTC's `audio` resample node).
    ///
    /// Default `false` preserves the smoke binary's strict A+V pairing
    /// for offline recording. Set to `true` for live WebRTC so the
    /// avatar / scene is visible immediately on connect, during model
    /// warmup, and during silence.
    pub realtime_mode: bool,
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
}

impl AudioBuf {
    fn new() -> Self {
        Self {
            samples: std::collections::VecDeque::with_capacity(48_000 * 4),
            sample_rate: 0,
            channels: 0,
            samples_emitted: 0,
        }
    }
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
        }
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
    fn skeletal_pose_from_json(v: &serde_json::Value) -> Option<SkeletalPose> {
        let arr = v.get("joint_quats_xyzw")?.as_array()?;
        if arr.len() != 22 {
            tracing::debug!(
                target: "cc_render",
                "skeletal_pose: expected 22 joints, got {}; dropping",
                arr.len()
            );
            return None;
        }
        let mut joint_quats = [[0.0_f32; 4]; 22];
        for (i, joint) in arr.iter().enumerate() {
            let q = joint.as_array()?;
            if q.len() != 4 {
                return None;
            }
            for k in 0..4 {
                joint_quats[i][k] = q[k].as_f64()? as f32;
            }
        }
        let pts_ms = v.get("pts_ms").and_then(|p| p.as_u64()).unwrap_or(0);
        let root_pos = v
            .get("root_pos")
            .and_then(|r| r.as_array())
            .and_then(|a| {
                if a.len() != 3 {
                    return None;
                }
                Some([
                    a[0].as_f64()? as f32,
                    a[1].as_f64()? as f32,
                    a[2].as_f64()? as f32,
                ])
            })
            .unwrap_or([0.0, 0.0, 0.0]);
        Some(SkeletalPose {
            joint_quats,
            root_pos,
            pts_ms,
        })
    }
}

#[derive(Debug)]
enum RendererInput {
    Blendshape(BlendshapeFrame),
    Skeletal(SkeletalPose),
    BargeIn,
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
        _callback: F,
    ) -> Result<usize>
    where
        F: FnMut(RuntimeData) -> Result<()> + Send,
    {
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
            let mut buf = self.audio_buf.lock();
            if buf.sample_rate == 0 {
                buf.sample_rate = *sample_rate;
                buf.channels = *channels;
                tracing::info!(
                    target: "cc_render",
                    "audio buffer initialized: {} Hz, {} ch",
                    sample_rate, channels,
                );
            }
            // Mono only — sum down if stereo arrives (kokoro is mono so
            // this is defensive).
            if *channels == 1 {
                buf.samples.extend(samples.as_slice().iter().copied());
            } else {
                let n = *channels as usize;
                let it = samples
                    .as_slice()
                    .chunks_exact(n)
                    .map(|c| c.iter().sum::<f32>() / n as f32);
                buf.samples.extend(it);
            }
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
                self.renderer.push_pose(Self::frame_to_pose(&f));
            }
            Some(RendererInput::Skeletal(s)) => {
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
            Some(RendererInput::BargeIn) => {
                self.renderer.push_pose(ArkitPose::default());
                self.renderer.push_skeletal_pose(None);
                let mut anchor = self.anchor_pts_ms.lock().await;
                *anchor = None;
                // Drop buffered audio too so we don't replay stale TTS.
                let mut ab = self.audio_buf.lock();
                ab.samples.clear();
                ab.samples_emitted = 0;
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
        if matches!(aux_port_of(&message), Some(BARGE_IN_PORT)) {
            self.renderer.push_pose(ArkitPose::default());
            // Also drop any active skeletal stream so the avatar
            // snaps back to baked rest on barge-in.
            self.renderer.push_skeletal_pose(None);
            let mut anchor = self.anchor_pts_ms.lock().await;
            *anchor = None;
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
        _tick: Tick,
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
        if frames.is_empty() {
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
            let Some(f) = latest else {
                return Ok(());
            };

            // First-emit lipsync alignment: when the audio buffer
            // has its first real samples, the visible mouth pose
            // already carries `f.pts_ms` (set by `LastAppliedPts`
            // via the most-recent blendshape). The audio buffer
            // holds samples from content_time=0 (kokoro starts
            // there). Without correction, V_0 shows the mouth at
            // content_time=warmup_gap_ms while A_0 plays content=0
            // — speech leads mouth by the warmup gap. Fix: pop the
            // pose-pts worth of samples off the front of the buffer
            // on first audio-bearing frame.
            //
            // Anchored on the frame we actually emit — using
            // `frames.first()` here would skip an audio offset that
            // doesn't match the visible pose after we drop the stale
            // backlog above.
            {
                let mut anchor = self.anchor_pts_ms.lock().await;
                if anchor.is_none() {
                    let mut ab = self.audio_buf.lock();
                    if ab.sample_rate != 0 && f.pts_ms > 0 && !ab.samples.is_empty() {
                        let skip = ((f.pts_ms as u128 * ab.sample_rate as u128) / 1000) as usize;
                        let actual = skip.min(ab.samples.len());
                        for _ in 0..actual {
                            ab.samples.pop_front();
                        }
                        tracing::debug!(
                            target: "timing",
                            stage = "ccrender_lipsync_align",
                            pts_ms = f.pts_ms as i64,
                            samples_skipped = actual as u64,
                            samples_remaining = ab.samples.len() as u64,
                        );
                        *anchor = Some(f.pts_ms);
                    }
                }
            }

            {
                let frame_number = self.frame_counter.fetch_add(1, Ordering::AcqRel);
                let frame_pts_us = (frame_number * 1_000_000) / fps;

                // Pull this frame's audio chunk from the buffer.
                // Only emits A when the buffer holds REAL samples
                // (not silence-pad) — during idle the audio sink
                // simply receives no Audio events for this tick.
                let audio_chunk = {
                    let mut ab = self.audio_buf.lock();
                    if ab.sample_rate == 0 || ab.samples.is_empty() {
                        None
                    } else {
                        let target_total = ((frame_number + 1) * ab.sample_rate as u64) / fps;
                        let already = ab.samples_emitted;
                        let want = target_total.saturating_sub(already) as usize;
                        if want == 0 {
                            None
                        } else {
                            let real_take = want.min(ab.samples.len());
                            if real_take == 0 {
                                None
                            } else {
                                let mut chunk: Vec<f32> = Vec::with_capacity(real_take);
                                for _ in 0..real_take {
                                    chunk.push(ab.samples.pop_front().expect("len checked"));
                                }
                                let pts_us =
                                    (ab.samples_emitted * 1_000_000) / ab.sample_rate as u64;
                                ab.samples_emitted += chunk.len() as u64;
                                Some((chunk, ab.sample_rate, ab.channels.max(1), pts_us))
                            }
                        }
                    }
                };

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

                if let Some((chunk, sr, ch, pts_us)) = audio_chunk {
                    let audio = RuntimeData::Audio {
                        samples: AudioSamples::Vec(chunk),
                        sample_rate: sr,
                        channels: ch,
                        stream_id: None,
                        timestamp_us: Some(pts_us),
                        arrival_ts_us: None,
                        metadata: None,
                    };
                    callback(audio)?;
                    emitted += 1;
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
