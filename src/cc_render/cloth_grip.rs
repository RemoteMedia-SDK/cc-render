//! Finite-force, compliant material-point clamp for the cloth solver.
//!
//! This is an abstract contact clamp, not a finger/pinch model. Acquisition
//! queries the current shape against the sheet and captures its existing frame.
//! Only an observed kinematic collider may supply subsequent poses. No task
//! target, avatar transform writer or automatic runtime admission is installed.
use bevy::math::{DQuat, DVec3};
use bevy_rapier3d::rapier::parry::{math::Isometry, shape::Shape};

use super::{
    cloth_solver::ClothError,
    deformable_surface::{DeformedSurface, MaterialPoint},
};

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(crate) struct ClampMaterial {
    /// Positive inverse stiffness, metres/newton. Zero would be a rigid pin.
    pub compliance_m_n: f64,
    pub damping_ratio: f64,
    /// Generic clamp capacity, not a task force command.
    pub capacity_n: f64,
}

impl ClampMaterial {
    fn validate(self) -> Result<(), ClothError> {
        if !self.compliance_m_n.is_finite()
            || self.compliance_m_n <= 0.0
            || !self.capacity_n.is_finite()
            || self.capacity_n <= 0.0
            || !self.damping_ratio.is_finite()
            || self.damping_ratio < 0.0
        {
            return Err(ClothError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClampState {
    Held,
    Released,
    Overloaded,
}

#[derive(Clone, Copy)]
struct ColliderFrame {
    translation: DVec3,
    rotation: DQuat,
}

impl ColliderFrame {
    fn from_pose(pose: &Isometry<f32>) -> Result<Self, ClothError> {
        let t = pose.translation.vector;
        let q = pose.rotation.quaternion();
        let rotation = DQuat::from_xyzw(q.i as f64, q.j as f64, q.k as f64, q.w as f64);
        let translation = DVec3::new(t.x as f64, t.y as f64, t.z as f64);
        if !translation.is_finite()
            || !rotation.is_finite()
            || (rotation.length_squared() - 1.0).abs() > 1e-4
        {
            return Err(ClothError::InvalidInput);
        }
        Ok(Self {
            translation,
            rotation: rotation.normalize(),
        })
    }
}

/// Opaque acquired material identity and collider-relative frame. Cloning is
/// used only for transactional solver steps, never to attach another surface.
#[derive(Clone)]
pub(crate) struct ClothGrip {
    point: MaterialPoint,
    vertices: [usize; 3],
    weights: [f64; 3],
    local_point: DVec3,
    previous_frame: ColliderFrame,
    observed_frame: ColliderFrame,
    material: ClampMaterial,
    state: ClampState,
    peak_required_force_n: f64,
    peak_applied_force_n: f64,
}

pub(super) struct GripParticles<'a> {
    pub positions: &'a mut [DVec3],
    pub previous: &'a [DVec3],
    pub inverse_mass: &'a [f64],
    pub h: f64,
}

impl ClothGrip {
    /// Caller selects a material face, but cannot supply a stale witness or
    /// manufacture contact using a point/UV. Returns None if that face is not
    /// touching the supplied current shape. Does not establish finger closure.
    pub(crate) fn capture(
        surface: &DeformedSurface,
        shape: &dyn Shape,
        pose: &Isometry<f32>,
        triangle: usize,
        material: ClampMaterial,
    ) -> Result<Option<Self>, ClothError> {
        material.validate()?;
        let frame = ColliderFrame::from_pose(pose)?;
        let Some(contact) = surface
            .contacts(shape, pose)?
            .into_iter()
            .find(|contact| contact.triangle() == triangle)
        else {
            return Ok(None);
        };
        let point = surface
            .material_point_from_contact(&contact)
            .ok_or(ClothError::InvalidInput)?;
        let position = surface
            .resolve(&point)
            .ok_or(ClothError::InvalidInput)?
            .position
            .as_dvec3();
        Ok(Some(Self {
            vertices: surface
                .material_vertices(&point)
                .ok_or(ClothError::InvalidInput)?,
            weights: point.barycentric().map(f64::from),
            point,
            local_point: frame.rotation.inverse() * (position - frame.translation),
            previous_frame: frame,
            observed_frame: frame,
            material,
            state: ClampState::Held,
            peak_required_force_n: 0.0,
            peak_applied_force_n: 0.0,
        }))
    }

    /// Observed collider pose at the next solver tick, not a desired hand pose.
    /// Adjacent poses are interpolated during substeps; release never rebinds.
    pub(crate) fn observe_pose(&mut self, pose: &Isometry<f32>) -> Result<(), ClothError> {
        self.observed_frame = ColliderFrame::from_pose(pose)?;
        Ok(())
    }

    pub(crate) fn release(&mut self) {
        if self.state == ClampState::Held {
            self.state = ClampState::Released;
        }
    }

    pub(crate) fn state(&self) -> ClampState {
        self.state
    }
    pub(crate) fn material_point(&self) -> &MaterialPoint {
        &self.point
    }
    pub(crate) fn peak_required_force_n(&self) -> f64 {
        self.peak_required_force_n
    }
    pub(crate) fn peak_applied_force_n(&self) -> f64 {
        self.peak_applied_force_n
    }

    pub(crate) fn anchor(&self, fraction: f64) -> DVec3 {
        self.previous_frame
            .translation
            .lerp(self.observed_frame.translation, fraction)
            + self
                .previous_frame
                .rotation
                .slerp(self.observed_frame.rotation, fraction)
                * self.local_point
    }

    pub(super) fn prepare(&mut self, surface: &DeformedSurface) -> Result<(), ClothError> {
        surface
            .resolve(&self.point)
            .ok_or(ClothError::InvalidInput)?;
        self.peak_required_force_n = 0.0;
        self.peak_applied_force_n = 0.0;
        Ok(())
    }

    pub(super) fn finish(&mut self) {
        self.previous_frame = self.observed_frame;
    }

    fn weighted_point(&self, positions: &[DVec3]) -> DVec3 {
        self.vertices
            .iter()
            .zip(self.weights)
            .map(|(i, w)| positions[*i] * w)
            .sum()
    }

    pub(super) fn project(
        &mut self,
        p: &mut GripParticles<'_>,
        interval: [f64; 2],
    ) -> Result<(), ClothError> {
        if self.state != ClampState::Held {
            return Ok(());
        }
        let mass = self
            .vertices
            .iter()
            .zip(self.weights)
            .map(|(i, b)| p.inverse_mass[*i] * b * b)
            .sum::<f64>();
        let anchor = self.anchor(interval[1]);
        let value = self.weighted_point(p.positions);
        let relative =
            value - self.weighted_point(p.previous) - (anchor - self.anchor(interval[0]));
        // Vector form of Small Steps Eq. 8, lambda reset each substep.
        let gamma =
            2.0 * self.material.damping_ratio * (self.material.compliance_m_n / mass).sqrt() / p.h;
        let lambda = -(value - anchor + gamma * relative)
            / ((1.0 + gamma) * mass + self.material.compliance_m_n / (p.h * p.h));
        let force = lambda.length() / (p.h * p.h);
        if !force.is_finite() {
            return Err(ClothError::NonfiniteState);
        }
        self.peak_required_force_n = self.peak_required_force_n.max(force);
        if force > self.material.capacity_n {
            // Break before applying an over-capacity correction; no sticky
            // follower, forced snap or automatic reacquisition after failure.
            self.state = ClampState::Overloaded;
            return Ok(());
        }
        self.peak_applied_force_n = self.peak_applied_force_n.max(force);
        for (i, b) in self.vertices.iter().zip(self.weights) {
            p.positions[*i] += lambda * (p.inverse_mass[*i] * b);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "cloth_grip_tests.rs"]
mod tests;
