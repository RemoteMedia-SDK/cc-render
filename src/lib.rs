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
pub mod cc_render;
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
/// `realtime_mode`, optional `env_background` /
/// `env_background_brightness`, optional `cc5_materials`, optional
/// `studio_lighting`.
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
    pub scene_package_path: Option<PathBuf>,
    /// Show the environment map (the one lighting the avatar) as the
    /// background instead of a flat colour.
    pub env_background: bool,
    /// Its brightness in cd/m² (default: the map's own values at the
    /// camera's exposure, `cc_render::renderer::env_background_brightness`).
    pub env_background_brightness: Option<f32>,
    /// Render materials carrying CC5 shader data (`extras.cc5`) with the
    /// CC5 shaders, as the avatar editor does.
    pub cc5_materials: bool,
    /// Light and grade like the avatar editor (`cc_render::studio`).
    /// Default true; false = the renderer's earlier lighting.
    pub studio_lighting: bool,
    /// When true, emit Video continuously once ready (skips A+V pairing
    /// and audio buffer drain). Used by live WebRTC pipelines.
    pub realtime_mode: bool,
    /// Private deployment binding for authenticated renderer scene jobs.
    pub scene_key_file: Option<PathBuf>,
    pub scene_geometry_file: Option<PathBuf>,
    /// `{"kind": "capture"}` (default: frames come back as Video) or
    /// `{"kind": "overlay", "anchor": "bottom-right", "margin": 24,
    /// "size": 320, "monitor": 0, "position": [x, y], "hotkey": "Ctrl+Alt+O"}`
    /// (drawn on top of the desktop, no Video emitted).
    pub present: crate::cc_render::Present,
    /// `"low" | "medium" | "high"`; absent: the full studio look.
    pub quality: Option<crate::cc_render::RenderQuality>,
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
            scene_package_path: None,
            env_background: false,
            env_background_brightness: None,
            cc5_materials: false,
            studio_lighting: true,
            realtime_mode: false,
            scene_key_file: None,
            scene_geometry_file: None,
            present: Default::default(),
            quality: None,
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
        scene_package_path: params.scene_package_path,
        env_background: params.env_background,
        env_background_brightness: params.env_background_brightness,
        cc5_materials: params.cc5_materials,
        studio_lighting: params.studio_lighting,
        realtime_mode: params.realtime_mode,
        scene_key_file: params.scene_key_file,
        scene_geometry_file: params.scene_geometry_file,
        present: params.present,
        quality: params.quality,
    };

    CcRenderNode::new(config).map_err(|e| format!("CcRenderNode spawn failed: {e}"))
}

#[derive(Default)]
pub struct CcRenderNodeFactory;

impl CcRenderNodeFactory {
    /// Authored outputs for in-process hosts to negotiate media before SDP.
    pub fn output_schema() -> remotemedia_plugin_sdk::traits::schema::NodeSchema {
        use remotemedia_plugin_sdk::traits::schema::{NodeSchema, RuntimeDataType};
        NodeSchema::new("CcRenderNode")
            .accepts([RuntimeDataType::Audio, RuntimeDataType::Json, RuntimeDataType::Text])
            .produces([RuntimeDataType::Video, RuntimeDataType::Audio, RuntimeDataType::Json])
    }
}

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

#[cfg(test)]
mod params_tests {
    use super::*;
    use crate::cc_render::overlay::Anchor;
    use crate::cc_render::{Present, RenderQuality};

    #[test]
    fn capture_and_no_quality_by_default() {
        let p: CcRenderManifestParams = serde_json::from_str(r#"{"glb_path":"a.glb"}"#).unwrap();
        assert_eq!(p.present, Present::Capture);
        assert_eq!(p.quality, None);
    }

    #[test]
    fn overlay_and_quality_parse() {
        let p: CcRenderManifestParams = serde_json::from_str(
            r#"{"present":{"kind":"overlay","anchor":"top-right","size":256,"position":[10,-20]},"quality":"medium"}"#,
        )
        .unwrap();
        let Present::Overlay(o) = p.present else { panic!("not an overlay: {:?}", p.present) };
        assert_eq!(o.anchor, Anchor::TopRight);
        assert_eq!(o.size, 256);
        assert_eq!(o.position, Some([10, -20]));
        assert_eq!(o.margin, 24);
        assert_eq!(p.quality, Some(RenderQuality::Medium));
        assert!(serde_json::from_str::<CcRenderManifestParams>(r#"{"quality":"ultra"}"#).is_err());
    }

    #[test]
    fn quality_presets() {
        use RenderQuality::*;
        assert_eq!(Low.shadow_map(), None);
        assert_eq!(Medium.shadow_map(), Some(1024));
        assert_eq!(High.shadow_map(), Some(4096));
        assert_eq!((Low.max_texture(), Medium.max_texture(), High.max_texture()), (Some(512), Some(1024), None));
        assert!(!Medium.physics() && !Medium.post_effects());
        assert!(High.physics() && High.post_effects());
        assert_eq!("HIGH".parse::<RenderQuality>(), Ok(High));
    }
}
