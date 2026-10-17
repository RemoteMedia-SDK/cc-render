//! Swept spatial broad phase and shared-vertex exclusions for self contact.
use super::pair::FeaturePair;
use super::*;
use parry_ccd::{bounding_volume::Aabb, math::Point, partitioning::Qbvh};

pub(super) struct SelfCollisionTopology {
    edges: Vec<[usize; 2]>,
    envelopes: Vec<(DVec3, DVec3)>,
    pairs: Vec<FeaturePair>,
    thickness: f64,
    axes: Vec<DVec3>,
}

impl SelfCollisionTopology {
    pub(super) fn new(triangles: &[[usize; 3]]) -> Self {
        let mut edges = std::collections::BTreeSet::new();
        for &[a, b, c] in triangles {
            for [a, b] in [[a, b], [b, c], [c, a]] {
                edges.insert([a.min(b), a.max(b)]);
            }
        }
        Self {
            edges: edges.into_iter().collect(),
            envelopes: Vec::new(),
            pairs: Vec::new(),
            thickness: 0.,
            axes: Vec::new(),
        }
    }

    pub(super) fn candidates(
        &mut self,
        particles: &GripParticles<'_>,
        triangles: &[[usize; 3]],
        thickness: f64,
    ) -> &[FeaturePair] {
        // A fat swept bound is a reusable spatial certificate, not a timed
        // refresh. Every endpoint must still fit; secondary corrections which
        // leave an envelope force a rebuild before the next contact pass.
        let fits = self.thickness == thickness
            && self.envelopes.len() == particles.positions.len()
            && self.envelopes.iter().enumerate().all(|(i, (lo, hi))| {
                [particles.previous[i], particles.positions[i]]
                    .into_iter()
                    .all(|p| p.cmpge(*lo).all() && p.cmple(*hi).all())
            });
        if !fits {
            let padding = 0.01;
            self.pairs = self.collect_candidates(particles, triangles, thickness + 2. * padding);
            self.axes = vec![DVec3::ZERO; self.pairs.len()];
            self.envelopes = particles
                .previous
                .iter()
                .zip(particles.positions.iter())
                .map(|(a, b)| {
                    (
                        a.min(*b) - DVec3::splat(padding),
                        a.max(*b) + DVec3::splat(padding),
                    )
                })
                .collect();
            self.thickness = thickness;
        }
        &self.pairs
    }

    fn collect_candidates(
        &self,
        particles: &GripParticles<'_>,
        triangles: &[[usize; 3]],
        thickness: f64,
    ) -> Vec<FeaturePair> {
        let mut tree = Qbvh::new();
        tree.clear_and_rebuild(
            triangles
                .iter()
                .enumerate()
                .map(|(i, ids)| (i, bounds(ids, particles, thickness * 0.5))),
            0.,
        );
        let mut candidates = Vec::new();
        let mut hits = Vec::new();
        for vertex in 0..particles.positions.len() {
            hits.clear();
            tree.intersect_aabb(&bounds(&[vertex], particles, thickness * 0.5), &mut hits);
            for &face in &hits {
                let [a, b, c] = triangles[face];
                if ![a, b, c].into_iter().any(|v| vertex == v) {
                    candidates.push(FeaturePair::VertexFace([vertex, a, b, c]));
                }
            }
        }
        tree.clear_and_rebuild(
            self.edges
                .iter()
                .enumerate()
                .map(|(i, ids)| (i, bounds(ids, particles, thickness * 0.5))),
            0.,
        );
        for (i, &[a, b]) in self.edges.iter().enumerate() {
            hits.clear();
            tree.intersect_aabb(&bounds(&[a, b], particles, thickness * 0.5), &mut hits);
            for &j in &hits {
                if j <= i {
                    continue;
                }
                let [c, d] = self.edges[j];
                if ![a, b]
                    .into_iter()
                    .any(|v| [c, d].into_iter().any(|w| v == w))
                {
                    candidates.push(FeaturePair::EdgeEdge([a, b, c, d]));
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    pub(super) fn resolve(
        &mut self,
        particles: &mut GripParticles<'_>,
        triangles: &[[usize; 3]],
        fixed: &[FixedClothCollider],
        thickness: f64,
    ) -> Result<(), ClothError> {
        collision::collide_surface(particles, triangles, fixed, thickness * 0.5)?;
        for _ in 0..64 {
            if !self.project(particles, triangles, thickness, fixed)? {
                return Ok(());
            }
            // Recheck fixed contacts after self-response. Stop only when both
            // passes agree; no last-writer permission to publish intersections.
            collision::collide_surface(particles, triangles, fixed, thickness * 0.5)?;
        }
        Err(ClothError::SelfContactUnresolved)
    }

    pub(super) fn project(
        &mut self,
        particles: &mut GripParticles<'_>,
        triangles: &[[usize; 3]],
        thickness: f64,
        fixed: &[FixedClothCollider],
    ) -> Result<bool, ClothError> {
        let mut changed = false;
        self.candidates(particles, triangles, thickness);
        for (pair, axis) in self.pairs.iter().zip(&mut self.axes) {
            changed |= pair.project_with_axis(particles, thickness, fixed, axis)?;
        }
        Ok(changed)
    }
}

fn bounds(ids: &[usize], particles: &GripParticles<'_>, radius: f64) -> Aabb {
    let (min, max) = ids
        .iter()
        .flat_map(|&i| [particles.previous[i], particles.positions[i]])
        .fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(p), hi.max(p)),
        );
    let min = min - DVec3::splat(radius);
    let max = max + DVec3::splat(radius);
    Aabb::new(
        Point::new(min.x, min.y, min.z),
        Point::new(max.x, max.y, max.z),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layers(gap: f64) -> Vec<DVec3> {
        [0., gap]
            .into_iter()
            .flat_map(|y| {
                [
                    DVec3::new(-1., y, -1.),
                    DVec3::new(1., y, -1.),
                    DVec3::new(0., y, 1.),
                ]
            })
            .collect()
    }

    #[test]
    fn cached_spatial_envelopes_rebuild_before_admitting_a_new_crossing() {
        let triangles = [[0, 1, 2], [3, 4, 5]];
        let mut topology = SelfCollisionTopology::new(&triangles);
        let previous = layers(0.5);
        let mut positions = previous.clone();
        let mut particles = GripParticles {
            positions: &mut positions,
            previous: &previous,
            inverse_mass: &[1.; 6],
            h: 1. / 60.,
        };
        assert!(topology
            .candidates(&particles, &triangles, 0.001)
            .is_empty());
        let original = topology.envelopes.clone();
        for p in &mut particles.positions[3..] {
            p.y += 0.001;
        }
        assert!(topology
            .candidates(&particles, &triangles, 0.001)
            .is_empty());
        assert_eq!(
            topology.envelopes, original,
            "motion inside certified envelopes should reuse the tree result"
        );
        for p in &mut particles.positions[3..] {
            p.y = -0.5;
        }
        let candidates = topology.candidates(&particles, &triangles, 0.001);
        assert!(candidates.contains(&FeaturePair::VertexFace([3, 0, 1, 2])));
        assert!(candidates
            .iter()
            .any(|p| matches!(p, FeaturePair::EdgeEdge(_))));
        for pair in candidates {
            let mut ids = pair.indices();
            ids.sort_unstable();
            assert!(ids.windows(2).all(|w| w[0] != w[1]));
        }
        assert_ne!(topology.envelopes, original);
    }

    #[test]
    fn fat_bounds_include_crossings_before_a_cache_refresh_is_needed() {
        let triangles = [[0, 1, 2], [3, 4, 5]];
        let mut topology = SelfCollisionTopology::new(&triangles);
        let previous = layers(0.005);
        let mut positions = previous.clone();
        let mut particles = GripParticles {
            positions: &mut positions,
            previous: &previous,
            inverse_mass: &[1.; 6],
            h: 1. / 60.,
        };
        assert!(topology
            .candidates(&particles, &triangles, 0.001)
            .contains(&FeaturePair::VertexFace([3, 0, 1, 2])));
        let original = topology.envelopes.clone();
        for p in &mut particles.positions[3..] {
            p.y = -0.004;
        }
        assert!(topology
            .candidates(&particles, &triangles, 0.001)
            .contains(&FeaturePair::VertexFace([3, 0, 1, 2])));
        assert_eq!(topology.envelopes, original);
        assert!(FeaturePair::VertexFace([3, 0, 1, 2])
            .project(&mut particles, 0.001)
            .unwrap());
    }
}
