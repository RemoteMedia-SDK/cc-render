//! Conservative interval search for linearly deforming faces against fixed convex bodies.
//! A face's maximum vertex displacement bounds its Hausdorff motion over the step.
use super::*;

#[cfg(test)]
#[path = "cloth_sweep_diagnostics.rs"]
mod diagnostics;

struct Witness {
    point: DVec3,
    normal: DVec3,
    distance: f64,
}

impl FixedClothCollider {
    fn project_swept_contact(
        &self,
        position: &mut DVec3,
        old: DVec3,
        witness: &Witness,
        radius: f64,
    ) {
        let depth = radius - witness.distance - (*position - witness.point).dot(witness.normal);
        if depth <= 0. {
            return;
        }
        *position += witness.normal * depth;
        let displacement = *position - old;
        let tangent = displacement - witness.normal * displacement.dot(witness.normal);
        if tangent.length() > 0. {
            *position -= tangent * (self.friction * depth / tangent.length()).min(1.);
        }
    }

    fn project_swept_particle(
        &self,
        end: &mut DVec3,
        old: DVec3,
        radius: f64,
    ) -> Result<(), ClothError> {
        let motion = *end - old;
        let speed = motion.length();
        if speed <= radius || !self.intersects_bounds(old.min(*end), old.max(*end), radius) {
            return Ok(());
        }
        let tolerance = 1e-8;
        if self.distance_and_normal(old).0 < radius - tolerance {
            return Ok(());
        }
        let mut t = 0.;
        for _ in 0..128 {
            let at = old + motion * t;
            let (distance, normal) = self.distance_and_normal(at);
            if distance <= radius + 2. * tolerance {
                self.project_swept_contact(
                    end,
                    old,
                    &Witness {
                        point: at,
                        normal,
                        distance,
                    },
                    radius,
                );
                return Ok(());
            }
            // The supporting plane bounds the entire remaining straight path.
            // Tangential motion does not close its gap and must not stall CCD.
            let closing = -motion.dot(normal);
            if closing <= 0. || distance - closing * (1. - t) >= radius {
                return Ok(());
            }
            let increment = (distance - radius - tolerance) / closing;
            if t + increment > 1. {
                return Ok(());
            }
            t += increment * 0.9;
        }
        Err(ClothError::SweptContactUnresolved)
    }

    fn sweep_witness(
        &self,
        vertices: [DVec3; 3],
        margin: f64,
    ) -> Result<Option<Witness>, ClothError> {
        let Some(weights) = self.triangle_core_weights(vertices, margin)? else {
            return Ok(None);
        };
        let point: DVec3 = vertices.into_iter().zip(weights).map(|(p, w)| p * w).sum();
        let (distance, normal) = self.distance_and_normal(point);
        Ok(Some(Witness {
            point,
            normal,
            distance,
        }))
    }

    fn swept_triangle(
        &self,
        old: [DVec3; 3],
        end: [DVec3; 3],
        radius: f64,
    ) -> Result<Option<Witness>, ClothError> {
        let motion = std::array::from_fn::<_, 3, _>(|i| end[i] - old[i]);
        let speed = motion.iter().map(|v| v.length()).fold(0., f64::max);
        // Below one skin radius a face cannot pass completely through the
        // two-sided thickness envelope. Discrete contact handles this regime.
        if speed <= radius {
            return Ok(None);
        }
        let (min, max) = old.into_iter().chain(end).fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(p), hi.max(p)),
        );
        if !self.intersects_bounds(min, max, radius) {
            return Ok(None);
        }
        // Cheap whole-trajectory rejection before any support-map query.
        // A plane supporting the fixed convex body separates the entire
        // linearly moving face when all six endpoint vertices clear it.
        let center = (old[0] + old[1] + old[2]) / 3.;
        let (distance, normal) = self.distance_and_normal(center);
        if old
            .into_iter()
            .chain(end)
            .all(|p| (p - center).dot(normal) + distance >= radius)
        {
            return Ok(None);
        }
        let tolerance = 1e-8;
        // Visit intervals in time order. A convex supporting plane certifies
        // the whole interval when all six endpoint vertices clear it. Bisect
        // uncertain intervals instead of taking arbitrarily small advancement
        // steps when one remote corner moves faster than the contact region.
        let mut intervals = vec![(0., 1.)];
        for _ in 0..8192 {
            let Some((start, finish)) = intervals.pop() else {
                return Ok(None);
            };
            let vertices = std::array::from_fn(|i| old[i] + motion[i] * start);
            let witness =
                match self.sweep_witness(vertices, speed * (finish - start) + radius + tolerance) {
                    Ok(Some(witness)) => witness,
                    Ok(None) => continue,
                    Err(ClothError::SweptContactUnresolved) if start == 0. => {
                        // Initial intersection has no separated GJK witness. Use
                        // the existing measured contact to choose its recovery
                        // side; never skip CCD merely because old contact exists.
                        let Some((weights, _, normal)) = self.triangle_contact(old, radius)? else {
                            return Err(ClothError::SweptContactUnresolved);
                        };
                        let point = old.into_iter().zip(weights).map(|(p, w)| p * w).sum();
                        Witness {
                            point,
                            normal,
                            distance: self.distance_and_normal(point).0,
                        }
                    }
                    Err(error) => return Err(error),
                };
            let separation = |points: [DVec3; 3]| {
                points
                    .into_iter()
                    .map(|p| (p - witness.point).dot(witness.normal) + witness.distance)
                    .fold(f64::INFINITY, f64::min)
            };
            let interval_end = std::array::from_fn(|i| old[i] + motion[i] * finish);
            if separation(vertices) >= radius && separation(interval_end) >= radius {
                continue;
            }
            if witness.distance <= radius + 2. * tolerance {
                return Ok((separation(end) < radius).then_some(witness));
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

    fn project_swept_triangle(
        &self,
        indices: [usize; 3],
        particles: &mut GripParticles<'_>,
        radius: f64,
    ) -> Result<(), ClothError> {
        let old = indices.map(|i| particles.previous[i]);
        let end = indices.map(|i| particles.positions[i]);
        let result = self.swept_triangle(old, end, radius);
        #[cfg(test)]
        if let Err(error) = &result {
            diagnostics::capture(self, &old, &end, radius, error);
        }
        let Some(witness) = result? else {
            return Ok(());
        };
        // A confirmed fast face impact constrains that face to the convex
        // supporting plane for this substep. This conservative local response
        // prevents the other corners crossing behind the contact witness.
        // Ordinary slow contacts retain the mass-weighted point response.
        for i in indices {
            self.project_swept_contact(
                &mut particles.positions[i],
                particles.previous[i],
                &witness,
                radius,
            );
        }
        Ok(())
    }
}

pub(super) fn collide_swept(
    particles: &mut GripParticles<'_>,
    triangles: &[[usize; 3]],
    colliders: &[FixedClothCollider],
    radius: f64,
) -> Result<(), ClothError> {
    // Same motion threshold as each particle/face, evaluated once for settled
    // cloth instead of repeating it for every face and every scene collider.
    if particles
        .positions
        .iter()
        .zip(particles.previous)
        .all(|(end, old)| (*end - *old).length() <= radius)
    {
        return Ok(());
    }
    let (old_min, old_max) = collision::surface_bounds(particles.previous);
    let (mut min, mut max) = collision::surface_bounds(particles.positions);
    for collider in colliders {
        if !collider.intersects_bounds(old_min.min(min), old_max.max(max), radius) {
            continue;
        }
        for (end, old) in particles.positions.iter_mut().zip(particles.previous) {
            let result = collider.project_swept_particle(end, *old, radius);
            #[cfg(test)]
            if let Err(error) = &result {
                diagnostics::capture(collider, &[*old], &[*end], radius, error);
            }
            result?;
        }
        for triangle in triangles {
            collider.project_swept_triangle(*triangle, particles, radius)?;
        }
        (min, max) = collision::surface_bounds(particles.positions);
    }
    Ok(())
}
