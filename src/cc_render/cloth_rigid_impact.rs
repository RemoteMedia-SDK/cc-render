//! Frictionless, zero-restitution impact at an observed cloth/rigid contact.
//! Produces equal-and-opposite impulses; does not integrate, repair positions,
//! or write live bodies. Inputs must all describe the same time of impact.
use super::{sweep::moving::RigidSweepHit, ClothError, DVec3};
use bevy::math::DMat3;

pub(crate) struct RigidContactBody {
    pub center_of_mass: DVec3,
    pub linear_velocity: DVec3,
    pub angular_velocity: DVec3,
    /// World-axis inverse translational masses; zero locks that axis.
    pub inverse_mass: DVec3,
    /// B such that B * B.transpose() is the world inverse inertia tensor.
    /// This factor representation preserves positive semidefiniteness, also
    /// for locked angular degrees of freedom, without eigenvalue tolerances.
    pub inverse_inertia_sqrt: DMat3,
}

impl RigidContactBody {
    /// Read actual Rapier state, including locked axes. The caller must arrange
    /// sampling at the contact's physics time; this does not read render poses.
    pub(crate) fn from_rapier(
        body: &bevy_rapier3d::rapier::dynamics::RigidBody,
    ) -> Result<Self, ClothError> {
        use bevy_rapier3d::rapier::na::Vector3;
        let vector = |v: Vector3<f32>| DVec3::new(v.x as f64, v.y as f64, v.z as f64);
        let mass = body.mass_properties();
        let root = mass.effective_world_inv_inertia_sqrt;
        let snapshot = Self {
            center_of_mass: vector(body.center_of_mass().coords),
            linear_velocity: if body.is_fixed() {
                DVec3::ZERO
            } else {
                vector(*body.linvel())
            },
            angular_velocity: if body.is_fixed() {
                DVec3::ZERO
            } else {
                vector(*body.angvel())
            },
            inverse_mass: if body.is_dynamic() {
                vector(mass.effective_inv_mass)
            } else {
                DVec3::ZERO
            },
            inverse_inertia_sqrt: if body.is_dynamic() {
                DMat3::from_cols(
                    vector(root * Vector3::x()),
                    vector(root * Vector3::y()),
                    vector(root * Vector3::z()),
                )
            } else {
                DMat3::ZERO
            },
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn validate(&self) -> Result<(), ClothError> {
        if !self.center_of_mass.is_finite()
            || !self.linear_velocity.is_finite()
            || !self.angular_velocity.is_finite()
            || !self.inverse_mass.is_finite()
            || self.inverse_mass.min_element() < 0.
            || !self.inverse_inertia_sqrt.is_finite()
        {
            return Err(ClothError::InvalidInput);
        }
        Ok(())
    }

    fn inertia_response(&self, torque: DVec3) -> DVec3 {
        self.inverse_inertia_sqrt * (self.inverse_inertia_sqrt.transpose() * torque)
    }
}

#[derive(Debug, Default)]
pub(crate) struct NormalImpact {
    /// Impulse on the cloth. The rigid body receives its negative at contact.
    /// The velocity deltas below describe this same impulse, not extra kicks.
    pub impulse: DVec3,
    pub cloth_delta_velocity: [DVec3; 3],
    pub body_delta_linear_velocity: DVec3,
    pub body_delta_angular_velocity: DVec3,
}

impl RigidSweepHit {
    /// Instantaneous response only. After applying it, the caller must rebuild
    /// both remaining trajectories and resolve further contacts. The original
    /// rigid sweep is stale once its velocity changes; this is not a complete
    /// step or a certificate that the remaining path is clear.
    pub(crate) fn normal_impact(
        &self,
        velocity: [DVec3; 3],
        inverse_mass: [f64; 3],
        body: &RigidContactBody,
    ) -> Result<NormalImpact, ClothError> {
        body.validate()?;
        self.validate_impact_inputs(velocity, inverse_mass)?;
        let lever = self.point - body.center_of_mass;
        let relative_velocity = velocity
            .into_iter()
            .zip(self.weights)
            .map(|(v, w)| v * w)
            .sum::<DVec3>()
            - body.linear_velocity
            - body.angular_velocity.cross(lever);
        let closing = relative_velocity.dot(self.normal);
        let cloth_inverse_mass = inverse_mass
            .into_iter()
            .zip(self.weights)
            .map(|(m, w)| m * w * w)
            .sum::<f64>();
        let angular = body.inverse_inertia_sqrt.transpose() * lever.cross(self.normal);
        // J M^-1 J^T includes the cloth's barycentric effective mass and
        // the rigid body's translation and rotation about its centre of mass.
        let effective_inverse_mass = cloth_inverse_mass
            + (body.inverse_mass * self.normal).dot(self.normal)
            + angular.length_squared();
        if !closing.is_finite() || !effective_inverse_mass.is_finite() {
            return Err(ClothError::NonfiniteState);
        }
        if closing >= 0. {
            return Ok(NormalImpact::default());
        }
        if effective_inverse_mass <= 0. {
            return Err(ClothError::RigidContactUnresolved);
        }
        let impulse = self.normal * (-closing / effective_inverse_mass);
        let result = NormalImpact {
            impulse,
            cloth_delta_velocity: std::array::from_fn(|i| {
                impulse * (inverse_mass[i] * self.weights[i])
            }),
            body_delta_linear_velocity: -impulse * body.inverse_mass,
            body_delta_angular_velocity: -body.inertia_response(lever.cross(impulse)),
        };
        if !result.impulse.is_finite()
            || result.cloth_delta_velocity.iter().any(|v| !v.is_finite())
            || !result.body_delta_linear_velocity.is_finite()
            || !result.body_delta_angular_velocity.is_finite()
        {
            return Err(ClothError::NonfiniteState);
        }
        Ok(result)
    }

    fn validate_impact_inputs(
        &self,
        velocity: [DVec3; 3],
        inverse_mass: [f64; 3],
    ) -> Result<(), ClothError> {
        if !self.point.is_finite()
            || !self.normal.is_finite()
            || (self.normal.length_squared() - 1.).abs() > 1e-10
            || !self.fraction.is_finite()
            || !(0. ..=1.).contains(&self.fraction)
            || !self.separation.is_finite()
            || self.weights.iter().any(|w| !w.is_finite() || *w < 0.)
            || (self.weights.iter().sum::<f64>() - 1.).abs() > 1e-10
            || velocity.iter().any(|v| !v.is_finite())
            || inverse_mass.iter().any(|m| !m.is_finite() || *m < 0.)
        {
            return Err(ClothError::InvalidInput);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "cloth_rigid_impact_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cloth_rigid_rapier_tests.rs"]
mod rapier_tests;
