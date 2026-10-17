//! Frictionless material-feature contact for deforming cloth surfaces.
//! Both features receive inverse-mass-weighted response; no fixed-side proxy.
use super::*;
use parry_ccd::{
    math::{Isometry, Point},
    query::{details::closest_points_segment_segment_with_locations, PointQueryWithLocation},
    shape::{Segment, Triangle},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum FeaturePair {
    VertexFace([usize; 4]),
    EdgeEdge([usize; 4]),
}

struct PairWitness {
    weights: [f64; 4],
    normal: DVec3,
    distance: f64,
}

impl FeaturePair {
    pub(super) fn indices(self) -> [usize; 4] {
        match self {
            Self::VertexFace(ids) | Self::EdgeEdge(ids) => ids,
        }
    }

    fn split(self) -> usize {
        match self {
            Self::VertexFace(_) => 1,
            Self::EdgeEdge(_) => 2,
        }
    }

    fn witness(self, positions: [DVec3; 4]) -> Result<PairWitness, ClothError> {
        // Translation-local f64 geometry avoids losing small cloth gaps far
        // from the scene origin. Reconstruct material points from source data.
        let [a, b, c, d] = positions.map(|p| {
            let p = p - positions[0];
            Point::new(p.x, p.y, p.z)
        });
        let weights = match self {
            Self::VertexFace(_) => {
                let (_, location) =
                    Triangle::new(b, c, d).project_local_point_and_get_location(&a, false);
                let w = collision::witness_weights(
                    location
                        .barycentric_coordinates()
                        .ok_or(ClothError::SelfContactUnresolved)?,
                )?;
                [1., -w[0], -w[1], -w[2]]
            }
            Self::EdgeEdge(_) => {
                let (a, b) = closest_points_segment_segment_with_locations(
                    &Isometry::identity(),
                    &Segment::new(a, b),
                    &Segment::new(c, d),
                );
                let a = a.barycentric_coordinates();
                let b = b.barycentric_coordinates();
                [a[0], a[1], -b[0], -b[1]]
            }
        };
        let relative: DVec3 = positions
            .into_iter()
            .zip(weights)
            .map(|(p, w)| (p - positions[0]) * w)
            .sum();
        let distance = relative.length();
        if !distance.is_finite() || distance < 1e-12 {
            // An initially intersecting surface has no trustworthy contact side.
            return Err(ClothError::SelfContactUnresolved);
        }
        Ok(PairWitness {
            weights,
            normal: relative / distance,
            distance,
        })
    }

    fn separation(self, positions: [DVec3; 4], normal: DVec3) -> f64 {
        let split = self.split();
        positions[..split]
            .iter()
            .flat_map(|a| {
                positions[split..]
                    .iter()
                    .map(move |b| (*a - *b).dot(normal))
            })
            .fold(f64::INFINITY, f64::min)
    }

    fn bounds_separate(self, old: [DVec3; 4], end: [DVec3; 4], thickness: f64) -> bool {
        let split = self.split();
        let feature_bounds = |range: std::ops::Range<usize>| {
            range.flat_map(|i| [old[i], end[i]]).fold(
                (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
                |(lo, hi), p| (lo.min(p), hi.max(p)),
            )
        };
        let (alo, ahi) = feature_bounds(0..split);
        let (blo, bhi) = feature_bounds(split..4);
        if alo.cmpgt(bhi + DVec3::splat(thickness)).any()
            || blo.cmpgt(ahi + DVec3::splat(thickness)).any()
        {
            return true;
        }
        false
    }

    fn sweep(
        self,
        old: [DVec3; 4],
        end: [DVec3; 4],
        thickness: f64,
    ) -> Result<Option<PairWitness>, ClothError> {
        // Proximity targets the full thickness. CCD guards an inner envelope,
        // leaving room to resolve contact without a grazing search starting
        // at its numerical boundary on every subsequent substep.
        let radius = thickness * 0.5;
        let motion: [DVec3; 4] = std::array::from_fn(|i| end[i] - old[i]);
        let tolerance = 1e-8;
        let mut intervals = vec![(0., 1.)];
        for _ in 0..8192 {
            let Some((start, finish)) = intervals.pop() else {
                return Ok(None);
            };
            let at = std::array::from_fn(|i| old[i] + motion[i] * start);
            let witness = self.witness(at)?;
            // Use a separating-plane lower bound to certify a clear interval.
            // The numerical closest-point distance is only an upper bound;
            // it can confirm contact but cannot safely certify separation.
            let interval_end = std::array::from_fn(|i| old[i] + motion[i] * finish);
            let gap_start = self.separation(at, witness.normal);
            let gap_end = self.separation(interval_end, witness.normal);
            if gap_start >= radius - tolerance && gap_end >= radius - tolerance {
                continue;
            }
            if witness.distance <= radius + tolerance {
                let predicted: DVec3 = end
                    .into_iter()
                    .zip(witness.weights)
                    .map(|(p, w)| (p - end[0]) * w)
                    .sum();
                if predicted.dot(witness.normal) < thickness - tolerance {
                    return Ok(Some(witness));
                }
                // Initial shallow overlap may separate outward, provided the
                // complete features stay on the same side and end fully clear.
                if start == 0. && self.separation(end, witness.normal) >= thickness {
                    return Ok(None);
                }
            }
            let middle = (start + finish) * 0.5;
            if middle == start || middle == finish {
                return Err(ClothError::SelfContactUnresolved);
            }
            intervals.push((middle, finish));
            intervals.push((start, middle));
        }
        Err(ClothError::SelfContactUnresolved)
    }

    #[cfg(test)]
    pub(super) fn project(
        self,
        particles: &mut GripParticles<'_>,
        thickness: f64,
    ) -> Result<bool, ClothError> {
        self.project_with_axis(particles, thickness, &[], &mut DVec3::ZERO)
    }

    pub(super) fn project_with_axis(
        self,
        particles: &mut GripParticles<'_>,
        thickness: f64,
        fixed: &[FixedClothCollider],
        axis: &mut DVec3,
    ) -> Result<bool, ClothError> {
        let ids = self.indices();
        let end = ids.map(|i| particles.positions[i]);
        let old = ids.map(|i| particles.previous[i]);
        if axis.length_squared() > 0.5
            && self.separation(old, *axis) >= thickness
            && self.separation(end, *axis) >= thickness
        {
            return Ok(false);
        }
        if self.bounds_separate(old, end, thickness) {
            return Ok(false);
        }
        let witness = match self.sweep(old, end, thickness)? {
            Some(witness) => witness,
            None => {
                let witness = self.witness(end)?;
                *axis = witness.normal;
                if witness.distance >= thickness - 1e-8 {
                    return Ok(false);
                }
                witness
            }
        };
        let relative: DVec3 = end
            .into_iter()
            .zip(witness.weights)
            .map(|(p, w)| (p - end[0]) * w)
            .sum();
        let depth = thickness - relative.dot(witness.normal);
        *axis = witness.normal;
        // A fixed support removes only the component directed into it. Its
        // reaction then participates in this constraint's effective mass;
        // supported lower cloth need not be repeatedly pushed through a floor.
        let directions = std::array::from_fn::<_, 4, _>(|k| {
            let i = ids[k];
            let mut direction = witness.normal * (witness.weights[k] * particles.inverse_mass[i]);
            for collider in fixed {
                let (distance, normal) = collider.distance_and_normal(particles.positions[i]);
                if distance <= thickness * 0.5 + 1e-8 {
                    direction -= normal * direction.dot(normal).min(0.);
                }
            }
            direction
        });
        let mass = directions
            .into_iter()
            .zip(witness.weights)
            .map(|(v, w)| w * v.dot(witness.normal))
            .sum::<f64>();
        if mass <= 0. || !mass.is_finite() {
            return Err(ClothError::SelfContactUnresolved);
        }
        for (i, direction) in ids.into_iter().zip(directions) {
            particles.positions[i] += direction * (depth / mass);
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "cloth_pair_tests.rs"]
mod tests;
