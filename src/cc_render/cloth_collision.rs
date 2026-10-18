//! Discrete surface contact against the solver's fixed convex colliders.
//! Triangle witnesses distribute collision response to material vertices.
//! Swept fixed-body contacts run first; the coupled solver also resolves self-contact.
use super::*;
#[cfg(test)]
#[path = "cloth_contact_probe.rs"]
mod probe;
use bevy_rapier3d::rapier::parry::{
    math::Point,
    query::{contact, PointQueryWithLocation},
    shape::Triangle,
};

impl FixedClothCollider {
    pub(super) fn triangle_contact(
        &self,
        vertices: [DVec3; 3],
        radius: f64,
    ) -> Result<Option<([f64; 3], f64, DVec3)>, ClothError> {
        let min = vertices[0].min(vertices[1]).min(vertices[2]);
        let max = vertices[0].max(vertices[1]).max(vertices[2]);
        if !self.intersects_bounds(min, max, radius) {
            return Ok(None);
        }
        let center = (vertices[0] + vertices[1] + vertices[2]) / 3.;
        let (distance, normal) = self.distance_and_normal(center);
        // For a convex collider this is a supporting plane. If all corners
        // clear it, the entire triangle clears it. The 1 nm tolerance only
        // avoids querying roundoff at already-solved contact, not real overlap.
        if vertices
            .iter()
            .all(|p| (*p - center).dot(normal) + distance >= radius - 1e-9)
        {
            return Ok(None);
        }
        let weights = match self.triangle_core_weights(vertices, radius) {
            Ok(Some(weights)) => weights,
            Ok(None) => return Ok(None),
            Err(ClothError::SweptContactUnresolved) => {
                // A triangle inside the polyhedral core has no separated
                // closest-point witness. Retain penetration recovery there.
                let Some(weights) = self.penetrating_triangle_weights(vertices, radius)? else {
                    return Ok(None);
                };
                weights
            }
            Err(error) => return Err(error),
        };
        let point = vertices.into_iter().zip(weights).map(|(p, w)| p * w).sum();
        // A support-map witness alone never triggers correction. Re-evaluate
        // actual signed distance and gradient with the analytic f64 collider.
        let (distance, normal) = self.distance_and_normal(point);
        let depth = radius - distance;
        Ok((depth > 0.).then_some((weights, depth, normal)))
    }

    /// Double-precision contact against the cuboid core; the rounded border
    /// is included in the query margin. Shared by discrete and swept contact.
    pub(super) fn triangle_core_weights(
        &self,
        vertices: [DVec3; 3],
        margin: f64,
    ) -> Result<Option<[f64; 3]>, ClothError> {
        use parry_ccd::{
            math::{Isometry, Point},
            query::{closest_points, ClosestPoints, PointQueryWithLocation},
            shape::Triangle,
        };
        let points = vertices.map(|p| self.rotation.conjugate() * (p - self.center));
        let [a, b, c] = points.map(|p| Point::new(p.x, p.y, p.z));
        let triangle = Triangle::new(a, b, c);
        let result = closest_points(
            &Isometry::identity(),
            self.sweep_shape.as_ref(),
            &Isometry::identity(),
            &triangle,
            margin + self.border,
        )
        .map_err(|_| ClothError::UnsupportedContact)?;
        let point = match result {
            ClosestPoints::WithinMargin(_, point) => point,
            ClosestPoints::Disjoint => return Ok(None),
            ClosestPoints::Intersecting => return Err(ClothError::SweptContactUnresolved),
        };
        let (_, location) = triangle.project_local_point_and_get_location(&point, false);
        Ok(Some(witness_weights(
            location
                .barycentric_coordinates()
                .ok_or(ClothError::InvalidInput)?,
        )?))
    }

    fn penetrating_triangle_weights(
        &self,
        vertices: [DVec3; 3],
        radius: f64,
    ) -> Result<Option<[f64; 3]>, ClothError> {
        let points = vertices.map(|p| self.rotation.conjugate() * (p - self.center));
        let [a, b, c] = points.map(|p| Point::new(p.x as f32, p.y as f32, p.z as f32));
        let triangle = Triangle::new(a, b, c);
        let Some(hit) = contact(
            &Isometry::identity(),
            self.shape.as_ref(),
            &Isometry::identity(),
            &triangle,
            radius as f32,
        )
        .map_err(|_| ClothError::UnsupportedContact)?
        else {
            return Ok(None);
        };
        let (_, location) = triangle.project_local_point_and_get_location(&hit.point2, false);
        let weights = location
            .barycentric_coordinates()
            .ok_or(ClothError::InvalidInput)?
            .map(f64::from);
        Ok(Some(witness_weights(weights)?))
    }

    /// Returns whether a correction was applied.
    fn project_triangle(
        &self,
        indices: [usize; 3],
        particles: &mut GripParticles<'_>,
        radius: f64,
    ) -> Result<bool, ClothError> {
        let vertices = indices.map(|i| particles.positions[i]);
        let Some((weights, depth, normal)) = self.triangle_contact(vertices, radius)? else {
            return Ok(false);
        };
        let inverse_mass = indices
            .into_iter()
            .zip(weights)
            .map(|(i, w)| particles.inverse_mass[i] * w * w)
            .sum::<f64>();
        let displacement = indices
            .into_iter()
            .zip(weights)
            .map(|(i, w)| (particles.positions[i] - particles.previous[i]) * w)
            .sum::<DVec3>();
        let tangent = displacement - normal * displacement.dot(normal);
        let friction = if tangent.length() > 0. {
            tangent * (self.friction * depth / tangent.length()).min(1.)
        } else {
            DVec3::ZERO
        };
        let correction = (normal * depth - friction) / inverse_mass;
        for (i, w) in indices.into_iter().zip(weights) {
            particles.positions[i] += correction * (particles.inverse_mass[i] * w);
        }
        Ok(true)
    }
}

pub(super) fn witness_weights(weights: [f64; 3]) -> Result<[f64; 3], ClothError> {
    // Parry's f32 face projection can return a tiny negative weight at a
    // vertex (observed -2.83e-10). Restrict to roundoff, then evaluate the
    // actual point and signed distance again; never expand collision geometry.
    let tolerance = 8. * f32::EPSILON as f64;
    if weights.iter().any(|w| !w.is_finite() || *w < -tolerance)
        || (weights.iter().sum::<f64>() - 1.).abs() > tolerance
    {
        return Err(ClothError::Surface(SurfaceError::InvalidWitness));
    }
    let weights = weights.map(|w| w.max(0.));
    let sum = weights.iter().sum::<f64>();
    Ok(weights.map(|w| w / sum))
}

/// Exact change tracking for repeated fixed-contact passes in one substep.
/// Every particle and face correction depends only on its own vertices (and
/// the substep start). An element none of whose vertices changed since the
/// previous pass began was processed then with these same inputs, without
/// effect, so processing it again cannot change anything.
pub(super) struct ContactStamps<'a> {
    /// Layer-local stamps: the clock value of each vertex's last change.
    pub stamps: &'a mut [u64],
    /// Elements whose vertices are all stamped below this are clean.
    pub threshold: u64,
    pub clock: &'a mut u64,
    /// Layer-local supporting planes (outward normal, collider friction) a
    /// swept face or particle response clamped each vertex to in this
    /// substep. They act as supports for the rest of the substep's contact
    /// solve (`SweptSupports`).
    pub planes: &'a mut [Option<(DVec3, f64)>],
}

impl ContactStamps<'_> {
    pub(super) fn clean(&self, ids: &[usize]) -> bool {
        ids.iter().all(|&i| self.stamps[i] < self.threshold)
    }

    pub(super) fn touch(&mut self, i: usize) {
        *self.clock += 1;
        self.stamps[i] = *self.clock;
    }

    /// Run `apply` on an element unless it is clean; stamp what it moved.
    pub(super) fn apply<const N: usize, R>(
        &mut self,
        ids: [usize; N],
        positions: &mut [DVec3],
        apply: impl FnOnce(&mut [DVec3]) -> R,
    ) -> Option<R> {
        if self.clean(&ids) {
            return None;
        }
        let before = ids.map(|i| positions[i]);
        let result = apply(positions);
        for (i, old) in ids.into_iter().zip(before) {
            if positions[i] != old {
                self.touch(i);
            }
        }
        Some(result)
    }
}

/// Particle projection against every fixed collider the surface can reach.
fn collide_particles(
    particles: &mut GripParticles<'_>,
    colliders: &[FixedClothCollider],
    radius: f64,
    stamps: &mut ContactStamps<'_>,
) {
    let (mut min, mut max) = surface_bounds(particles.positions);
    for collider in colliders {
        if !collider.intersects_bounds(min, max, radius) {
            continue;
        }
        let mut bounds = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for i in 0..particles.positions.len() {
            let old = particles.previous[i];
            stamps.apply([i], particles.positions, |p| {
                collider.project(&mut p[i], old, radius)
            });
            bounds.0 = bounds.0.min(particles.positions[i]);
            bounds.1 = bounds.1.max(particles.positions[i]);
        }
        (min, max) = bounds;
    }
}

/// One full fixed-contact pass over every particle and face.
#[cfg(test)]
pub(super) fn collide_surface(
    particles: &mut GripParticles<'_>,
    triangles: &[[usize; 3]],
    colliders: &[FixedClothCollider],
    radius: f64,
) -> Result<(), ClothError> {
    let mut stamps = vec![0; particles.positions.len()];
    let mut planes = vec![None; particles.positions.len()];
    let mut clock = 0;
    collide_surface_tracked(
        particles,
        triangles,
        colliders,
        radius,
        &mut ContactStamps {
            stamps: &mut stamps,
            threshold: 0,
            clock: &mut clock,
            planes: &mut planes,
        },
    )
}

pub(super) fn collide_surface_tracked(
    particles: &mut GripParticles<'_>,
    triangles: &[[usize; 3]],
    colliders: &[FixedClothCollider],
    radius: f64,
    stamps: &mut ContactStamps<'_>,
) -> Result<(), ClothError> {
    #[cfg(test)]
    let mut probe = probe::ContactProbe::new(particles);
    profiled!(
        "fixed_swept",
        super::sweep::collide_swept(particles, triangles, colliders, radius, stamps)
    )?;
    #[cfg(test)]
    probe.record("swept", particles);
    collide_particles(particles, colliders, radius, stamps);
    #[cfg(test)]
    probe.record("particles_before_triangles", particles);
    profile_scope!("fixed_triangles");
    // Face bounds are taken once at stage start. A face is skipped only when
    // its start bound, grown by the largest vertex displacement any correction
    // in this stage has made, misses the collider: its current bound then
    // misses too, and `triangle_contact` would return no contact.
    let start = particles.positions.to_vec();
    let boxes: Vec<_> = triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| start[i]);
            (a.min(b).min(c), a.max(b).max(c))
        })
        .collect();
    let mut displaced = 0f64;
    let (mut min, mut max) = surface_bounds(particles.positions);
    for collider in colliders {
        if !collider.intersects_bounds(min, max, radius) {
            continue;
        }
        for (triangle, (lo, hi)) in triangles.iter().zip(&boxes) {
            if !collider.intersects_bounds(*lo, *hi, radius + displaced) || stamps.clean(triangle) {
                continue;
            }
            let before = triangle.map(|i| particles.positions[i]);
            if collider.project_triangle(*triangle, particles, radius)? {
                for (i, old) in triangle.iter().zip(before) {
                    displaced = displaced.max(particles.positions[*i].distance(start[*i]));
                    if particles.positions[*i] != old {
                        stamps.touch(*i);
                    }
                }
            }
        }
        (min, max) = surface_bounds(particles.positions);
    }
    #[cfg(test)]
    probe.record("triangles", particles);
    collide_particles(particles, colliders, radius, stamps);
    #[cfg(test)]
    probe.record("particles_after_triangles", particles);
    Ok(())
}

pub(super) fn surface_bounds(positions: &[DVec3]) -> (DVec3, DVec3) {
    positions.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(min, max), p| (min.min(*p), max.max(*p)),
    )
}

#[cfg(test)]
#[path = "cloth_collision_tests.rs"]
mod tests;
