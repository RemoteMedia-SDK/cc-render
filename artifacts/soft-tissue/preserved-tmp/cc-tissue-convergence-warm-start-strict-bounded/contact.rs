//! Frictionless normal contact at embedded skin samples. Geometry queries and
//! swept-contact generation belong to the caller; these planes are not CCD.
//! The unilateral penalty energy is solved together with material strain.
use super::{material::Tet, TissueError};
use bevy::math::DVec3;
use serde::Serialize;

/// A rendered skin sample expressed in the tetrahedral degrees of freedom.
/// `offset` preserves its residual from the cage and its kinematic skin share.
#[derive(Clone, Copy, Debug)]
pub struct EmbeddedPoint {
    pub tetrahedron: usize,
    pub barycentric: [f64; 4],
    pub influence: f64,
    pub offset: DVec3,
}

impl EmbeddedPoint {
    pub(super) fn position(&self, positions: &[DVec3], tets: &[Tet]) -> DVec3 {
        self.offset
            + tets[self.tetrahedron]
                .ids
                .iter()
                .zip(self.barycentric)
                .map(|(i, w)| positions[*i] * (self.influence * w))
                .sum::<DVec3>()
    }
}

/// The permitted side is `normal.dot(point - surface_point) >= 0`.
/// Supply the plane and embedded offset at the physical substep endpoint.
/// Stiffness is N/m for this quadrature sample, including its represented area;
/// splitting a sample requires splitting its stiffness, not duplicating it.
#[derive(Clone, Copy, Debug)]
pub struct PlaneContact {
    pub point: EmbeddedPoint,
    pub surface_point: DVec3,
    pub normal: DVec3,
    pub stiffness_n_per_m: f64,
}

impl PlaneContact {
    pub(super) fn validate(&self, tetrahedra: usize) -> Result<(), TissueError> {
        if self.point.tetrahedron >= tetrahedra
            || !self.point.offset.is_finite()
            || self
                .point
                .barycentric
                .iter()
                .any(|w| !w.is_finite() || *w < 0.0)
            || (self.point.barycentric.iter().sum::<f64>() - 1.0).abs() > 1e-8
            || !self.point.influence.is_finite()
            || self.point.influence <= 0.0
            || self.point.influence > 1.0
            || !self.surface_point.is_finite()
            || !self.normal.is_finite()
            || (self.normal.length_squared() - 1.0).abs() > 1e-8
            || !self.stiffness_n_per_m.is_finite()
            || self.stiffness_n_per_m <= 0.0
        {
            return Err(TissueError::InvalidInput("embedded contact plane"));
        }
        Ok(())
    }

    pub(super) fn depth(&self, positions: &[DVec3], tets: &[Tet]) -> f64 {
        (-self
            .normal
            .dot(self.point.position(positions, tets) - self.surface_point))
        .max(0.0)
    }

    pub(super) fn energy(&self, positions: &[DVec3], tets: &[Tet]) -> f64 {
        0.5 * self.stiffness_n_per_m * self.depth(positions, tets).powi(2)
    }
}

/// Residual overlap is reported, never hidden with a render-position clamp.
/// Penalty contact permits finite overlap; callers must enforce their tolerance.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ContactMetrics {
    pub active_contacts: usize,
    pub maximum_penetration_m: f64,
    pub normal_force_n: f64,
    pub elastic_energy_j: f64,
}

pub(super) fn metrics(
    contacts: &[PlaneContact],
    x: &[DVec3],
    tets: &[Tet],
) -> Result<ContactMetrics, TissueError> {
    let mut result = ContactMetrics::default();
    for contact in contacts {
        let depth = contact.depth(x, tets);
        result.active_contacts += usize::from(depth > 0.0);
        result.maximum_penetration_m = result.maximum_penetration_m.max(depth);
        result.normal_force_n += contact.stiffness_n_per_m * depth;
        result.elastic_energy_j += 0.5 * contact.stiffness_n_per_m * depth * depth;
    }
    if !result.normal_force_n.is_finite() || !result.elastic_energy_j.is_finite() {
        return Err(TissueError::Nonfinite);
    }
    Ok(result)
}
