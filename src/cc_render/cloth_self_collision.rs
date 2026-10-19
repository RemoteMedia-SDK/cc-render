//! Swept spatial broad phase and shared-vertex exclusions for self contact.
use super::pair::FeaturePair;
use super::*;
use parry_ccd::{bounding_volume::Aabb, math::Point, partitioning::Qbvh};

pub(super) struct ContactLayer<'a> {
    pub range: std::ops::Range<usize>,
    pub triangles: &'a [[usize; 3]],
    pub radius: f64,
    pub fixed: &'a [FixedClothCollider],
}

fn layer_for<'a, 'b>(layers: &'a [ContactLayer<'b>], vertex: usize) -> &'a ContactLayer<'b> {
    layers
        .iter()
        .find(|layer| layer.range.contains(&vertex))
        .expect("validated cloth layer partition")
}

fn fixed_contacts(
    particles: &mut GripParticles<'_>,
    layers: &[ContactLayer<'_>],
) -> Result<(), ClothError> {
    for layer in layers {
        let r = layer.range.clone();
        let mut local = GripParticles {
            positions: &mut particles.positions[r.clone()],
            previous: &particles.previous[r.clone()],
            inverse_mass: &particles.inverse_mass[r],
            h: particles.h,
        };
        collision::collide_surface(&mut local, layer.triangles, layer.fixed, layer.radius)?;
    }
    Ok(())
}

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
        let layers = [ContactLayer {
            range: 0..particles.positions.len(),
            triangles,
            radius: thickness * 0.5,
            fixed,
        }];
        self.resolve_layers(particles, triangles, &layers)
    }

    pub(super) fn resolve_layers(
        &mut self,
        particles: &mut GripParticles<'_>,
        triangles: &[[usize; 3]],
        layers: &[ContactLayer<'_>],
    ) -> Result<(), ClothError> {
        fixed_contacts(particles, layers)?;
        for iteration in 0..512 {
            // Start with a 0.4 micrometre projection reserve; difficult coupled
            // contacts get 1 micrometre after 64 passes.
            // This is not an acceptance slop: the unpadded feature distance and
            // complete swept path must still certify the original thickness.
            let reserve = if iteration < 64 { 4e-7 } else { 1e-6 };
            if !self.project_layers(particles, triangles, layers, reserve)? {
                return Ok(());
            }
            fixed_contacts(particles, layers)?;
            if self.resolved(particles, triangles, layers)? {
                return Ok(());
            }
        }
        Err(ClothError::SelfContactUnresolved)
    }

    fn project_layers(
        &mut self,
        particles: &mut GripParticles<'_>,
        triangles: &[[usize; 3]],
        layers: &[ContactLayer<'_>],
        reserve: f64,
    ) -> Result<bool, ClothError> {
        let thickness = layers.iter().map(|l| 2. * l.radius).fold(0., f64::max);
        self.candidates(particles, triangles, thickness);
        let supports = face_supports(particles.positions, layers)?;
        let inverse_mass = particles.inverse_mass;
        let mut changed = false;
        for (pair, axis) in self.pairs.iter().zip(&mut self.axes) {
            let ids = pair.indices();
            let gap =
                layer_for(layers, ids[0]).radius + layer_for(layers, ids[pair.split()]).radius;
            changed |= pair.project_with_support(particles, gap + reserve, axis, |ids, p, d| {
                let d = std::array::from_fn(|k| {
                    let layer = layer_for(layers, ids[k]);
                    pair::fixed_support_direction(layer.fixed, layer.radius, p[k], d[k])
                });
                supports.project(ids, inverse_mass, d)
            })?;
        }
        Ok(changed)
    }

    fn resolved(
        &mut self,
        particles: &GripParticles<'_>,
        triangles: &[[usize; 3]],
        layers: &[ContactLayer<'_>],
    ) -> Result<bool, ClothError> {
        let thickness = layers.iter().map(|l| 2. * l.radius).fold(0., f64::max);
        self.candidates(particles, triangles, thickness);
        for (pair, axis) in self.pairs.iter().zip(&self.axes) {
            let ids = pair.indices();
            let gap =
                layer_for(layers, ids[0]).radius + layer_for(layers, ids[pair.split()]).radius;
            if !pair.resolved(particles, gap, *axis)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

struct FaceSupport {
    ids: [usize; 3],
    weights: [f64; 3],
    normal: DVec3,
}

pub(super) struct FaceSupports {
    constraints: Vec<FaceSupport>,
    adjacent: Vec<Vec<usize>>,
}

impl FaceSupports {
    pub(super) fn project<const N: usize>(
        &self,
        ids: [usize; N],
        inverse_mass: &[f64],
        mut directions: [DVec3; N],
    ) -> [DVec3; N] {
        let mut rows: Vec<_> = ids
            .into_iter()
            .flat_map(|i| self.adjacent[i].iter().copied())
            .collect();
        rows.sort_unstable();
        rows.dedup();
        // Dykstra corrections recover the mass-metric projection onto the
        // intersection. Plain cyclic clipping can add excessive support
        // motion and even reverse the response to the original load.
        let mut corrections = vec![0.; rows.len()];
        for _ in 0..128 {
            let before = directions;
            for (slot, &row) in rows.iter().enumerate() {
                let contact = &self.constraints[row];
                let weights: [f64; N] = ids.map(|i| {
                    contact
                        .ids
                        .iter()
                        .position(|j| *j == i)
                        .map(|k| contact.weights[k])
                        .unwrap_or(0.)
                });
                let mass: f64 = ids
                    .into_iter()
                    .zip(weights)
                    .map(|(i, w)| inverse_mass[i] * w * w)
                    .sum();
                for k in 0..N {
                    directions[k] +=
                        contact.normal * (corrections[slot] * inverse_mass[ids[k]] * weights[k]);
                }
                let approach: f64 = directions
                    .iter()
                    .zip(weights)
                    .map(|(d, w)| d.dot(contact.normal) * w)
                    .sum();
                corrections[slot] = approach.min(0.) / mass;
                for k in 0..N {
                    directions[k] -=
                        contact.normal * (corrections[slot] * inverse_mass[ids[k]] * weights[k]);
                }
            }
            if directions
                .iter()
                .zip(before)
                .all(|(a, b)| a.distance(b) < 1e-10)
            {
                break;
            }
        }
        directions
    }
}

pub(super) fn face_supports(
    positions: &[DVec3],
    layers: &[ContactLayer<'_>],
) -> Result<FaceSupports, ClothError> {
    let mut supports = FaceSupports {
        constraints: Vec::new(),
        adjacent: vec![Vec::new(); positions.len()],
    };
    for layer in layers {
        let (min, max) = collision::surface_bounds(&positions[layer.range.clone()]);
        let fixed: Vec<_> = layer
            .fixed
            .iter()
            .filter(|collider| collider.intersects_bounds(min, max, layer.radius + 1e-8))
            .collect();
        for triangle in layer.triangles {
            let ids = triangle.map(|i| i + layer.range.start);
            let vertices = ids.map(|i| positions[i]);
            for collider in &fixed {
                if let Some((weights, _, normal)) =
                    collider.triangle_contact(vertices, layer.radius + 1e-8)?
                {
                    let row = supports.constraints.len();
                    supports.constraints.push(FaceSupport {
                        ids,
                        weights,
                        normal,
                    });
                    for (i, weight) in ids.into_iter().zip(weights) {
                        if weight > 0. {
                            supports.adjacent[i].push(row);
                        }
                    }
                }
            }
        }
    }
    Ok(supports)
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

    #[test]
    fn coupled_supports_do_not_reverse_the_applied_load() {
        let supports = FaceSupports {
            constraints: [[0.9, 0.1, 0.], [0.1, 0.9, 0.]]
                .map(|weights| FaceSupport {
                    ids: [0, 1, 2],
                    weights,
                    normal: DVec3::Y,
                })
                .into_iter()
                .collect(),
            adjacent: vec![vec![0, 1], vec![0, 1], vec![], vec![]],
        };
        let load = [-DVec3::Y * 0.4, -DVec3::Y * 0.6, DVec3::ZERO, DVec3::ZERO];
        let response = supports.project([0, 1, 2, 3], &[1.; 4], load);
        // This load is inside the cone of support reactions: the closest
        // feasible response is zero. Sequential clipping instead leaves an
        // upward response that performs negative work against the load.
        assert!(response.iter().all(|d| d.length() < 1e-8), "{response:?}");
        assert!(
            response
                .into_iter()
                .zip(load)
                .map(|(d, f)| d.dot(f))
                .sum::<f64>()
                >= -1e-10
        );
    }

    #[test]
    fn off_center_contact_on_supported_face_can_tilt_without_penetrating() {
        let previous = vec![
            DVec3::new(-1., 0.0005, -1.),
            DVec3::new(1., 0.0005, -1.),
            DVec3::new(0., 0.0005, 1.),
            DVec3::new(0.35, 0.2, -0.05),
            DVec3::new(0.45, 0.2, -0.05),
            DVec3::new(0.4, 0.2, 0.05),
        ];
        let mut positions = previous.clone();
        for p in &mut positions[3..] {
            p.y = 0.0012;
        }
        let triangles = [[0, 1, 2], [3, 4, 5]];
        let fixed = [FixedClothCollider::new(
            SharedShape::cuboid(0.1, 0.1, 0.1),
            Isometry::translation(0., -0.1, 0.),
            0.,
        )
        .unwrap()];
        SelfCollisionTopology::new(&triangles)
            .resolve(
                &mut GripParticles {
                    positions: &mut positions,
                    previous: &previous,
                    inverse_mass: &[1.; 6],
                    h: 1. / 60.,
                },
                &triangles,
                &fixed,
                0.001,
            )
            .unwrap();
        assert!(
            positions[..3].iter().any(|p| p.y < 0.0004999),
            "support must allow tilting"
        );
        assert!(
            positions[..3].iter().any(|p| p.y > 0.0005001),
            "support reaction missing"
        );
        for i in 3..6 {
            let mut copy = positions.clone();
            assert!(
                !FeaturePair::VertexFace([i, 0, 1, 2])
                    .project(
                        &mut GripParticles {
                            positions: &mut copy,
                            previous: &positions,
                            inverse_mass: &[1.; 6],
                            h: 1. / 60.
                        },
                        0.001,
                    )
                    .unwrap(),
                "resolved layer gap must not need more correction"
            );
        }
        for triangle in triangles {
            if let Some((_, depth, _)) = fixed[0]
                .triangle_contact(triangle.map(|i| positions[i]), 0.0005)
                .unwrap()
            {
                assert!(depth < 1e-8, "fixed support penetration {depth}");
            }
        }
    }

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
