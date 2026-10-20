//! Character Creator 5 shaping sliders outside CC5.
//!
//! - [`pack`]: load a slider pack (`pack.json` + `pack.bin`, built from a CC5
//!   capture) and evaluate a slider mix into vertex and joint deltas.
//! - [`glb`]: write those deltas into a character GLB (bind mesh, joints and
//!   inverse bind matrices).
//! - [`subdiv`]: drive CC5's HD (subdivided) mesh from the level-0 pack.
//! - [`expr`]: facial expressions (CC5's ~400 morph targets, more than Bevy's 64
//!   per mesh) as sparse vertex deltas in the same space, for CPU morphing.
//!
//! With the `python` feature this crate is also the CPython module `cc5_shape`.

pub mod expr;
pub mod glb;
pub mod pack;
pub mod subdiv;

#[cfg(feature = "python")]
mod python;

pub use expr::{ArkitMap, ExpressionPack};
pub use glb::{shape_glb, shape_glb_hd};
pub use pack::{ShapeDelta, ShapePack, Slider, StateDeltas};
pub use subdiv::SubdivMap;
