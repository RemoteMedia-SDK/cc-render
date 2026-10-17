//! Discrete surface contact against the solver's fixed convex colliders.
//! Triangle witnesses distribute collision response to material vertices.
//! This is not swept collision or cloth self-collision.
use super::*;
use bevy_rapier3d::rapier::parry::{
    math::Point,
    query::{contact, PointQueryWithLocation},
    shape::Triangle,
};

impl FixedClothCollider {
    fn triangle_contact(
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
        // Collider-local queries avoid losing contact precision to world origin.
        let points = vertices.map(|p| self.rotation.conjugate() * (p - self.center));
        let [a, b, c] = points.map(|p| Point::new(p.x as f32, p.y as f32, p.z as f32));
        let triangle = Triangle::new(a, b, c);
        let identity = Isometry::identity();
        let Some(hit) = contact(
            &identity,
            self.shape.as_ref(),
            &identity,
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
        let weights = witness_weights(weights)?;
        let point = vertices.into_iter().zip(weights).map(|(p, w)| p * w).sum();
        // A support-map witness alone never triggers correction. Re-evaluate
        // actual signed distance and gradient with the analytic f64 collider.
        let (distance, normal) = self.distance_and_normal(point);
        let depth = radius - distance;
        Ok((depth > 0.).then_some((weights, depth, normal)))
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

fn witness_weights(weights: [f64; 3]) -> Result<[f64; 3], ClothError> {
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
    collide_particles(particles.positions, particles.previous, colliders, radius);
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
    collide_particles(particles.positions, particles.previous, colliders, radius);
    Ok(())
}

fn surface_bounds(positions: &[DVec3]) -> (DVec3, DVec3) {
    positions.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(min, max), p| (min.min(*p), max.max(*p)),
    )
}

#[cfg(test)]
#[path = "cloth_collision_tests.rs"]
mod tests;
