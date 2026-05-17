//! `CcRenderNode` as a standalone Path 3 loadable plugin.
//!
//! Bevy 0.15 driven CC5 / glTF avatar renderer. Consumes:
//!
//! - `{kind: "blendshapes", arkit_52, pts_ms, turn_id?}` — ARKit-52
//!   blendshape envelopes from upstream lip-sync nodes
//!   (e.g. `Audio2FaceLipSyncNode`). Forwarded to the Bevy thread
//!   via a `watch` channel (latest-wins).
//! - `{kind: "skeletal_pose", joint_quats_xyzw[22], root_pos[3],
//!    pts_ms}` — SMPL-22 skeletal poses from `KimodoMotionNode`. FIFO
//!    queued so paced motion plays in order.
//! - `{kind: "barge_in"}` / aux-port `barge_in` envelopes — snap back
//!    to rest pose, clear audio buffer, release skeletal stream.
//! - `RuntimeData::Audio` (mono f32) — buffered and emitted paired
//!    with each rendered Video frame so downstream consumers receive
//!    content-time-aligned A/V pairs.
//!
//! Emits:
//!
//! - `RuntimeData::Video {format: Rgba32, ...}` at the bound media
//!   clock rate (default 30 fps).
//! - `RuntimeData::Audio` (the paired chunk for each Video frame).
//! - One-shot `RuntimeData::Json {kind: "renderer_ready"}` once the
//!   GLB scene has settled — driver bins gate their upstream gate
//!   open on this signal.
//!
//! Originally lived in `remotemedia-core` under
//! `nodes/cc_render`; extracted here so the host crate doesn't drag
//! in `bevy` + `bevy_rapier3d` + `wgpu` + `image` + `pollster` just
//! for this renderer.
//!
//! ## Node types exported
//!
//!   CcRenderNode — blendshape / skeletal_pose / audio / barge_in
//!                  envelopes → Video (Rgba32) + paired Audio at the
//!                  bound media clock rate (default 30 fps).

mod arkit;
mod cc_render;
mod session_control;

use std::path::PathBuf;

use remotemedia_plugin_sdk::abi_stable::sabi_trait::TD_Opaque;
use remotemedia_plugin_sdk::abi_stable::std_types::{RErr, ROk, RResult, RString};
use remotemedia_plugin_sdk::adapter::StreamingNodeFfiAdapter;
use remotemedia_plugin_sdk::{FfiNodeBox, FfiNodeFactory, FfiNode_TO};

use crate::cc_render::{CcRenderConfig, CcRenderNode};

// ---------------------------------------------------------------------------
// Manifest params
// ---------------------------------------------------------------------------

/// JSON-shaped params accepted by the factory. Mirrors the host's
/// historical wiring at `streaming_registry::CcRenderNodeFactory`:
/// `glb_path`, `arkit_map_path`, `framerate`, `video_stream_id`,
/// `width`, `height`, optional `scene_glb_path`, optional
/// `realtime_mode`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct CcRenderManifestParams {
    /// Path to the CC5 `.alphabaked.glb` avatar. Required.
    pub glb_path: Option<PathBuf>,
    /// Path to the matching `.arkit_map.resolved.json`. Required.
    pub arkit_map_path: Option<PathBuf>,
    /// Output framerate (frames per second). Defaults to 30.
    pub framerate: u32,
    /// Output `RuntimeData::Video.stream_id`. Defaults to
    /// `"avatar.video"`.
    pub video_stream_id: String,
    /// Render-target width in pixels (default 1280).
    pub width: u32,
    /// Render-target height in pixels (default 1280).
    pub height: u32,
    /// Optional environment-scene GLB loaded alongside the avatar.
    pub scene_glb_path: Option<PathBuf>,
    /// When true, emit Video continuously once ready (skips A+V pairing
    /// and audio buffer drain). Used by live WebRTC pipelines.
    pub realtime_mode: bool,
}

impl Default for CcRenderManifestParams {
    fn default() -> Self {
        Self {
            glb_path: None,
            arkit_map_path: None,
            framerate: 30,
            video_stream_id: "avatar.video".to_string(),
            width: 1280,
            height: 1280,
            scene_glb_path: None,
            realtime_mode: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Factory + plugin registration
// ---------------------------------------------------------------------------

fn build_node(params: CcRenderManifestParams) -> Result<CcRenderNode, String> {
    let glb_path = params
        .glb_path
        .ok_or_else(|| "CcRenderNode requires 'glb_path' (.alphabaked.glb)".to_string())?;
    let arkit_map_path = params
        .arkit_map_path
        .ok_or_else(|| "CcRenderNode requires 'arkit_map_path' (resolved JSON)".to_string())?;

    let config = CcRenderConfig {
        glb_path,
        arkit_map_path,
        width: params.width.max(1),
        height: params.height.max(1),
        fps: params.framerate.max(1),
        video_stream_id: params.video_stream_id,
        scene_glb_path: params.scene_glb_path,
        realtime_mode: params.realtime_mode,
    };

    CcRenderNode::new(config).map_err(|e| format!("CcRenderNode spawn failed: {e}"))
}

#[derive(Default)]
pub struct CcRenderNodeFactory;

impl FfiNodeFactory for CcRenderNodeFactory {
    fn node_type(&self) -> RString {
        RString::from("CcRenderNode")
    }

    fn create(&self, params: RString) -> RResult<FfiNodeBox, RString> {
        let parsed: CcRenderManifestParams = match serde_json::from_str(params.as_str()) {
            Ok(p) => p,
            Err(e) => {
                return RErr(RString::from(format!(
                    "CcRenderNode params parse failed: {e}"
                )));
            }
        };
        let node = match build_node(parsed) {
            Ok(n) => n,
            Err(e) => return RErr(RString::from(e)),
        };
        ROk(FfiNode_TO::from_value(
            StreamingNodeFfiAdapter::new(node),
            TD_Opaque,
        ))
    }
}

remotemedia_plugin_sdk::plugin_export!(CcRenderNodeFactory);
