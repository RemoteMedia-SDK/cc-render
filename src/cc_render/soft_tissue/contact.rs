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
/// `max_force_n` caps the reaction (a bounded contact pressure times the same
/// area share); beyond it the energy grows linearly and overlap is permitted.
/// `f64::INFINITY` keeps the unbounded quadratic penalty.
#[derive(Clone, Copy, Debug)]
pub struct PlaneContact {
    pub point: EmbeddedPoint,
    pub surface_point: DVec3,
    pub normal: DVec3,
    pub stiffness_n_per_m: f64,
    pub max_force_n: f64,
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
            || self.max_force_n.is_nan()
            || self.max_force_n <= 0.0
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

    /// Depth where the quadratic penalty reaches the force cap.
    fn limit_depth(&self) -> f64 {
        self.max_force_n / self.stiffness_n_per_m
    }

    pub fn is_force_limited(&self, depth: f64) -> bool {
        depth > self.limit_depth()
    }

    /// Normal reaction for `depth`: the penalty, bounded by the cap.
    pub(super) fn force(&self, depth: f64) -> f64 {
        (self.stiffness_n_per_m * depth).min(self.max_force_n)
    }

    /// Derivative of `force`; zero on the capped (linear-energy) branch.
    pub(super) fn tangent(&self, depth: f64) -> f64 {
        if self.is_force_limited(depth) { 0.0 } else { self.stiffness_n_per_m }
    }

    pub(super) fn energy(&self, positions: &[DVec3], tets: &[Tet]) -> f64 {
        self.depth_energy(self.depth(positions, tets))
    }

    pub(super) fn depth_energy(&self, depth: f64) -> f64 {
        if self.is_force_limited(depth) {
            let limit = self.limit_depth();
            self.max_force_n * (depth - 0.5 * limit)
        } else {
            0.5 * self.stiffness_n_per_m * depth * depth
        }
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
    /// Active samples at the force cap and their deepest overlap. Their
    /// penetration is permitted by the bounded pressure, not hidden.
    pub force_limited_contacts: usize,
    pub maximum_force_limited_penetration_m: f64,
    /// Deepest overlap among samples below the cap: the penalty residual.
    pub maximum_unlimited_penetration_m: f64,
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
        result.normal_force_n += contact.force(depth);
        result.elastic_energy_j += contact.depth_energy(depth);
        if contact.is_force_limited(depth) {
            result.force_limited_contacts += 1;
            result.maximum_force_limited_penetration_m = result.maximum_force_limited_penetration_m.max(depth);
        } else {
            result.maximum_unlimited_penetration_m = result.maximum_unlimited_penetration_m.max(depth);
        }
    }
    if !result.normal_force_n.is_finite() || !result.elastic_energy_j.is_finite() {
        return Err(TissueError::Nonfinite);
    }
    Ok(result)
}
