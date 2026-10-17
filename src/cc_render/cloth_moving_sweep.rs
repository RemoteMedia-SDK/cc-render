//! Continuous queries against a supplied rigid transform over one physics
//! interval. Reads object motion; never writes a rigid body or avatar pose.
//! Runtime clock/history integration and contact response remain separate work.
use super::*;

/// Rotation is an explicit world-space axis-angle displacement, not inferred
/// from endpoint quaternions (which would lose full turns). `origin` is the
/// initial pivot and `translation` is its displacement during this interval.
pub(crate) struct RigidClothSweep<'a> {
    collider: &'a FixedClothCollider,
    origin: DVec3,
    translation: DVec3,
    rotation: DVec3,
}

#[derive(Debug)]
pub(crate) struct RigidSweepHit {
    pub fraction: f64,
    pub point: DVec3,
    pub normal: DVec3,
    pub separation: f64,
    pub weights: [f64; 3],
}

impl<'a> RigidClothSweep<'a> {
    pub(crate) fn new(
        collider: &'a FixedClothCollider,
        origin: DVec3,
        translation: DVec3,
        rotation: DVec3,
    ) -> Result<Self, ClothError> {
        if !origin.is_finite() || !translation.is_finite() || !rotation.length().is_finite() {
            return Err(ClothError::InvalidInput);
        }
        Ok(Self {
            collider,
            origin,
            translation,
            rotation,
        })
    }

    fn turn(&self, fraction: f64) -> DQuat {
        DQuat::from_scaled_axis(self.rotation * fraction)
    }

    /// Express a world point in the collider's initial world frame. This keeps
    /// the existing exact cuboid/rounded-cuboid signed-distance implementation.
    fn unmove(&self, point: DVec3, fraction: f64) -> DVec3 {
        self.origin
            + self.turn(fraction).conjugate() * (point - self.origin - self.translation * fraction)
    }

    fn hit(&self, fraction: f64, witness: Witness, weights: [f64; 3]) -> RigidSweepHit {
        let turn = self.turn(fraction);
        RigidSweepHit {
            fraction,
            point: self.origin + turn * (witness.point - self.origin) + self.translation * fraction,
            normal: turn * witness.normal,
            separation: witness.distance,
            weights,
        }
    }

    /// Conservative first-contact query for a linearly deforming cloth face.
    /// Rotation makes its relative path curved. For f(t)=R(-t)(a+bt),
    /// |f'| <= |b|+w|a+bt| and |f''| <= w*w|a+bt|+2*w|b|.
    /// A chord over width h deviates by at most max|f''|*h*h/8. A supporting
    /// plane may certify an interval only after accounting for that deviation.
    pub(crate) fn triangle(
        &self,
        old: [DVec3; 3],
        end: [DVec3; 3],
        radius: f64,
    ) -> Result<Option<RigidSweepHit>, ClothError> {
        if !radius.is_finite() || radius <= 0. || old.into_iter().chain(end).any(|p| !p.is_finite())
        {
            return Err(ClothError::InvalidInput);
        }
        let motion = std::array::from_fn::<_, 3, _>(|i| end[i] - old[i]);
        let (speed, acceleration) = self.motion_bounds(old, end)?;
        let at = |t| std::array::from_fn(|i| self.unmove(old[i] + motion[i] * t, t));
        let tolerance = 1e-8;
        let mut intervals = vec![(0., 1.)];
        for _ in 0..8192 {
            let Some((start, finish)) = intervals.pop() else {
                return Ok(None);
            };
            let vertices = at(start);
            let Some((witness, weights)) = self.witness(
                vertices,
                speed * (finish - start) + radius + tolerance,
                radius,
            )?
            else {
                continue;
            };
            let chord_error = acceleration * (finish - start).powi(2) / 8.;
            let plane_gap = vertices
                .into_iter()
                .chain(at(finish))
                .map(|p| (p - witness.point).dot(witness.normal) + witness.distance)
                .fold(f64::INFINITY, f64::min);
            if plane_gap >= radius + chord_error + tolerance {
                continue;
            }
            if witness.distance <= radius + 2. * tolerance {
                return Ok(Some(self.hit(start, witness, weights)));
            }
            let middle = (start + finish) * 0.5;
            if middle == start || middle == finish {
                return Err(ClothError::SweptContactUnresolved);
            }
            intervals.push((middle, finish));
            intervals.push((start, middle));
        }
        Err(ClothError::SweptContactUnresolved)
    }

    // Uniform derivative bounds for every material point of the triangle.
    fn motion_bounds(&self, old: [DVec3; 3], end: [DVec3; 3]) -> Result<(f64, f64), ClothError> {
        let motion = std::array::from_fn::<_, 3, _>(|i| end[i] - old[i]);
        let relative_speed = motion
            .iter()
            .map(|v| (*v - self.translation).length())
            .fold(0., f64::max);
        let reach = (0..3)
            .flat_map(|i| {
                [
                    (old[i] - self.origin).length(),
                    (end[i] - self.origin - self.translation).length(),
                ]
            })
            .fold(0., f64::max);
        let angular = self.rotation.length();
        let speed = relative_speed + angular * reach;
        let acceleration = angular * angular * reach + 2. * angular * relative_speed;
        if !speed.is_finite() || !acceleration.is_finite() {
            return Err(ClothError::InvalidInput);
        }
        Ok((speed, acceleration))
    }

    /// Velocity of the object's surface point nearest the cloth witness.
    /// Includes rotation about the translated pivot; duration is the physical
    /// interval represented by this sweep, never a render-frame duration.
    pub(crate) fn contact_velocity(
        &self,
        hit: &RigidSweepHit,
        duration: f64,
    ) -> Result<DVec3, ClothError> {
        if !duration.is_finite() || duration <= 0. {
            return Err(ClothError::InvalidInput);
        }
        let surface_point = hit.point - hit.normal * hit.separation;
        let pivot = self.origin + self.translation * hit.fraction;
        let velocity = (self.translation + self.rotation.cross(surface_point - pivot)) / duration;
        if !velocity.is_finite() {
            return Err(ClothError::NonfiniteState);
        }
        Ok(velocity)
    }

    fn witness(
        &self,
        vertices: [DVec3; 3],
        margin: f64,
        radius: f64,
    ) -> Result<Option<(Witness, [f64; 3])>, ClothError> {
        let weights = match self.collider.triangle_core_weights(vertices, margin) {
            Ok(Some(weights)) => weights,
            Ok(None) => return Ok(None),
            Err(ClothError::SweptContactUnresolved) => {
                let Some((weights, _, _)) = self.collider.triangle_contact(vertices, radius)?
                else {
                    return Err(ClothError::SweptContactUnresolved);
                };
                weights
            }
            Err(error) => return Err(error),
        };
        let point = vertices.into_iter().zip(weights).map(|(p, w)| p * w).sum();
        let (distance, normal) = self.collider.distance_and_normal(point);
        Ok(Some((
            Witness {
                point,
                normal,
                distance,
            },
            weights,
        )))
    }
}

#[cfg(test)]
#[path = "cloth_moving_sweep_tests.rs"]
mod tests;
