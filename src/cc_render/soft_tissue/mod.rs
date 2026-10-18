//! Volumetric secondary tissue. This module never writes an articulated body pose.
//!
//! World-space tetrahedra, volume-lumped mass, compliant distributed attachment,
//! and a coupled implicit co-rotated material solve. A caller supplies the
//! attachment frame at *each physical step*, not once per rendered frame.
mod asset;
mod contact;
mod material;
mod isochoric;
mod pressure;
mod solver;

pub use asset::{RegionAsset, SurfaceBinding, TissueAsset, VertexBinding};
pub use contact::{ContactMetrics, EmbeddedPoint, PlaneContact};
pub use pressure::PressureDiscretization;
pub use solver::{Attachment, Frame, Material, Tissue, TissueError, TissueMetrics, TimeIntegration, Viscosity};
