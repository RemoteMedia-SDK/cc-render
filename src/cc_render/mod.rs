//! CC5 / glTF avatar renderer node — Bevy-based.
//!
//! See `node.rs` for the streaming-pipeline contract; `renderer.rs`
//! owns the dedicated Bevy thread + the inbound pose / outbound frame
//! channels.
//!
//! Architecture (one renderer per session, in-process):
//!
//! ```text
//!   blendshape stream   ┌──────────────────┐    RuntimeData::Video
//!     (ARKit-52)    ──→ │ CcRenderNode     │ ──→ (RGBA frames, 30fps)
//!                       │  └─ CcRenderer ──┼──→ ╔══════════════════╗
//!                       └──────────────────┘    ║ Bevy thread      ║
//!                              ↑                ║   apply pose     ║
//!                              │                ║   render → PNG   ║
//!                              └────────────────╢   readback →     ║
//!                                  frame_rx     ║   frame_tx       ║
//!                                               ╚══════════════════╝
//! ```
//!
//! In this Path-3 plugin all `avatar-render-cc` /
//! `avatar-render-cc-physics` cargo features from the host crate are
//! dropped — the plugin always ships the full renderer + physics stack.

pub mod bevy_app;
pub mod node;
pub mod renderer;

pub use node::{CcRenderConfig, CcRenderNode};
#[allow(unused_imports)]
pub use renderer::{ArkitPose, CcRenderer, RenderedFrame, Renderer, RendererConfig, SkeletalPose};

#[allow(unused_imports)]
pub use renderer::{
    GrabCmd, GrabConfig, GrabFailure, GrabOrRelease, GrabSnapshot, GraspAnchor, Hand,
    PhysicsConfig, PlayVolume, PropCmd, PropId, PropShape, PropSnapshot, SceneState,
};
