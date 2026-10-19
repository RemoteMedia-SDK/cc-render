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
pub mod scene_package;
pub mod scene_mechanism;
pub mod scene_weather;
pub mod scene_water;
pub mod deformable_surface;
pub mod cloth_solver;
#[cfg(test)]
pub(crate) mod cloth_profile;
pub(crate) mod cloth_grip;
#[cfg(test)]
mod cloth_turndown_tests;
#[cfg(test)]
mod cloth_under_covers_tests;
#[cfg(test)]
mod cloth_gpu_probe_tests;
pub mod scene_cloth;
pub mod soft_tissue;

pub use node::{CcRenderConfig, CcRenderNode};
#[allow(unused_imports)]
pub use renderer::{ArkitPose, CcRenderer, RenderedFrame, Renderer, RendererConfig, SkeletalPose};

#[allow(unused_imports)]
pub use renderer::{
    GrabCmd, GrabConfig, GrabFailure, GrabOrRelease, GrabSnapshot, GraspAnchor, Hand,
    PhysicsConfig, PlayVolume, PropCmd, PropId, PropShape, PropSnapshot, SceneState,
};

pub mod office;
