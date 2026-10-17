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

    fn project_triangle(
        &self,
        indices: [usize; 3],
        particles: &mut GripParticles<'_>,
        radius: f64,
    ) -> Result<(), ClothError> {
        let vertices = indices.map(|i| particles.positions[i]);
        let Some((weights, depth, normal)) = self.triangle_contact(vertices, radius)? else {
            return Ok(());
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
        Ok(())
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

pub(super) fn collide_surface(
    particles: &mut GripParticles<'_>,
    triangles: &[[usize; 3]],
    colliders: &[FixedClothCollider],
    radius: f64,
) -> Result<(), ClothError> {
    #[cfg(test)]
    let mut probe = probe::ContactProbe::new(particles);
    profiled!(
        "fixed_swept",
        super::sweep::collide_swept(particles, triangles, colliders, radius)
    )?;
    #[cfg(test)]
    probe.record("swept", particles);
    collide_particles(particles.positions, particles.previous, colliders, radius);
    #[cfg(test)]
    probe.record("particles_before_triangles", particles);
    profile_scope!("fixed_triangles");
    let (mut min, mut max) = surface_bounds(particles.positions);
    for collider in colliders {
        if !collider.intersects_bounds(min, max, radius) {
            continue;
        }
        for triangle in triangles {
            collider.project_triangle(*triangle, particles, radius)?;
        }
        (min, max) = surface_bounds(particles.positions);
    }
    #[cfg(test)]
    probe.record("triangles", particles);
    collide_particles(particles.positions, particles.previous, colliders, radius);
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
