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
    /// Global triangle indices incident to each vertex, for local supports.
    vertex_triangles: Vec<Vec<usize>>,
    envelopes: Vec<(DVec3, DVec3)>,
    pairs: Vec<FeaturePair>,
    /// Per-vertex contact radius the cached candidates were inflated with.
    radii: Vec<f64>,
    axes: Vec<DVec3>,
    /// Exact change tracking inside one contact solve. A pair evaluation is a
    /// pure function of its four vertices (plus its own cached axis), so a
    /// pair found inactive stays inactive until one of them moves.
    clock: u64,
    stamps: Vec<u64>,
    pair_clean: Vec<u64>,
    pair_resolved: Vec<u64>,
}

impl SelfCollisionTopology {
    pub(super) fn new(triangles: &[[usize; 3]]) -> Self {
        let mut edges = std::collections::BTreeSet::new();
        let vertices = triangles.iter().flatten().map(|v| v + 1).max().unwrap_or(0);
        let mut vertex_triangles = vec![Vec::new(); vertices];
        for (t, &[a, b, c]) in triangles.iter().enumerate() {
            for [a, b] in [[a, b], [b, c], [c, a]] {
                edges.insert([a.min(b), a.max(b)]);
            }
            for v in [a, b, c] {
                vertex_triangles[v].push(t);
            }
        }
        Self {
            edges: edges.into_iter().collect(),
            vertex_triangles,
            envelopes: Vec::new(),
            pairs: Vec::new(),
            radii: Vec::new(),
            axes: Vec::new(),
            clock: 0,
            stamps: Vec::new(),
            pair_clean: Vec::new(),
            pair_resolved: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn candidates(
        &mut self,
        particles: &GripParticles<'_>,
        triangles: &[[usize; 3]],
        thickness: f64,
    ) -> &[FeaturePair] {
        let radii = vec![thickness * 0.5; particles.positions.len()];
        self.candidates_with(particles, triangles, &radii);
        &self.pairs
    }

    fn candidates_with(
        &mut self,
        particles: &GripParticles<'_>,
        triangles: &[[usize; 3]],
        radii: &[f64],
    ) {
        // A fat swept bound is a reusable spatial certificate, not a timed
        // refresh. Every endpoint must still fit; secondary corrections which
        // leave an envelope force a rebuild before the next contact pass.
        let fits = self.radii == radii
            && self.envelopes.len() == particles.positions.len()
            && self.envelopes.iter().enumerate().all(|(i, (lo, hi))| {
                [particles.previous[i], particles.positions[i]]
                    .into_iter()
                    .all(|p| p.cmpge(*lo).all() && p.cmple(*hi).all())
            });
        if !fits {
            profile_scope!("self_candidates_rebuild");
            self.pairs = self.collect_candidates(particles, triangles, radii);
            self.axes = vec![DVec3::ZERO; self.pairs.len()];
            self.pair_clean = vec![0; self.pairs.len()];
            self.pair_resolved = vec![0; self.pairs.len()];
            self.envelopes = particles
                .previous
                .iter()
                .zip(particles.positions.iter())
                .map(|(a, b)| {
                    (
                        a.min(*b) - DVec3::splat(CANDIDATE_PADDING),
                        a.max(*b) + DVec3::splat(CANDIDATE_PADDING),
                    )
                })
                .collect();
            self.radii = radii.to_vec();
        }
    }

    fn collect_candidates(
        &self,
        particles: &GripParticles<'_>,
        triangles: &[[usize; 3]],
        radii: &[f64],
    ) -> Vec<FeaturePair> {
        // Each feature is inflated by its own layer radius, the projection
        // reserve and the envelope padding. Overlap therefore includes every
        // pair that can come within its contact gap while all vertices stay
        // inside their envelopes.
        let inflate = |ids: &[usize]| radii[ids[0]] + CONTACT_RESERVE_LIMIT + CANDIDATE_PADDING;
        let mut tree = Qbvh::new();
        tree.clear_and_rebuild(
            triangles
                .iter()
                .enumerate()
                .map(|(i, ids)| (i, bounds(ids, particles, inflate(ids)))),
            0.,
        );
        let mut candidates = Vec::new();
        let mut hits = Vec::new();
        for vertex in 0..particles.positions.len() {
            hits.clear();
            tree.intersect_aabb(&bounds(&[vertex], particles, inflate(&[vertex])), &mut hits);
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
                .map(|(i, ids)| (i, bounds(ids, particles, inflate(ids)))),
            0.,
        );
        for (i, &[a, b]) in self.edges.iter().enumerate() {
            hits.clear();
            tree.intersect_aabb(&bounds(&[a, b], particles, inflate(&[a, b])), &mut hits);
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

    /// Record which vertices a whole-surface pass actually moved.
    fn mark_moved(&mut self, before: &[DVec3], after: &[DVec3]) {
        let mut stamped = false;
        for (i, (a, b)) in before.iter().zip(after).enumerate() {
            if a != b {
                if !stamped {
                    self.clock += 1;
                    stamped = true;
                }
                self.stamps[i] = self.clock;
            }
        }
    }

    fn fixed_contacts_tracked(
        &mut self,
        particles: &mut GripParticles<'_>,
        layers: &[ContactLayer<'_>],
        scratch: &mut Vec<DVec3>,
    ) -> Result<(), ClothError> {
        scratch.clear();
        scratch.extend_from_slice(particles.positions);
        fixed_contacts(particles, layers)?;
        self.mark_moved(scratch, particles.positions);
        Ok(())
    }

    pub(super) fn resolve_layers(
        &mut self,
        particles: &mut GripParticles<'_>,
        triangles: &[[usize; 3]],
        layers: &[ContactLayer<'_>],
    ) -> Result<(), ClothError> {
        let radii: Vec<_> = (0..particles.positions.len())
            .map(|i| layer_for(layers, i).radius)
            .collect();
        // Every vertex moved during integration and material projection.
        self.clock += 1;
        self.stamps = vec![self.clock; particles.positions.len()];
        self.pair_clean.fill(0);
        self.pair_resolved.fill(0);
        let mut scratch = Vec::with_capacity(particles.positions.len());
        profiled!("fixed_contacts", fixed_contacts(particles, layers))?;
        for iteration in 0..512 {
            #[cfg(test)]
            crate::cc_render::cloth_profile::count("self_iterations", 1);
            // Start with a 0.4 micrometre projection reserve; difficult coupled
            // contacts get 1 micrometre after 64 passes.
            // This is not an acceptance slop: the unpadded feature distance and
            // complete swept path must still certify the original thickness.
            let reserve = if iteration < 64 {
                4e-7
            } else {
                CONTACT_RESERVE_LIMIT
            };
            if iteration == 64 {
                self.pair_clean.fill(0);
            }
            if !profiled!(
                "self_project",
                self.project_layers(particles, triangles, layers, &radii, reserve)
            )? {
                return Ok(());
            }
            profiled!(
                "fixed_contacts_repeat",
                self.fixed_contacts_tracked(particles, layers, &mut scratch)
            )?;
            if profiled!(
                "self_resolved",
                self.resolved(particles, triangles, layers, &radii)
            )? {
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
        radii: &[f64],
        reserve: f64,
    ) -> Result<bool, ClothError> {
        self.candidates_with(particles, triangles, radii);
        // Supports describe the fixed contacts at the start of this pass.
        // They are evaluated lazily, per incident face, only for pairs that
        // actually need a correction.
        let snapshot = particles.positions.to_vec();
        let supports = LazySupports {
            positions: &snapshot,
            triangles,
            layers,
            vertex_triangles: &self.vertex_triangles,
            rows: Default::default(),
        };
        #[cfg(test)]
        crate::cc_render::cloth_profile::count("self_pairs", self.pairs.len() as u64);
        let inverse_mass = particles.inverse_mass;
        let mut changed = false;
        let mut evaluations = 0u64;
        let Self {
            pairs,
            axes,
            clock,
            stamps,
            pair_clean,
            pair_resolved,
            ..
        } = self;
        for (k, (pair, axis)) in pairs.iter().zip(axes.iter_mut()).enumerate() {
            let ids = pair.indices();
            if pair_clean[k] > ids.map(|i| stamps[i]).into_iter().max().unwrap() {
                continue;
            }
            evaluations += 1;
            let gap = radii[ids[0]] + radii[ids[pair.split()]];
            pair_resolved[k] = 0;
            let moved =
                pair.project_with_support(particles, gap + reserve, axis, |ids, p, d| {
                    let d = std::array::from_fn(|k| {
                        let layer = layer_for(layers, ids[k]);
                        pair::fixed_support_direction(layer.fixed, layer.radius, p[k], d[k])
                    });
                    supports.project(ids, inverse_mass, d)
                })?;
            if moved {
                *clock += 1;
                for i in ids {
                    stamps[i] = *clock;
                }
                changed = true;
            } else {
                pair_clean[k] = *clock + 1;
            }
        }
        #[cfg(test)]
        crate::cc_render::cloth_profile::count("self_pair_evaluations", evaluations);
        let _ = evaluations;
        Ok(changed)
    }

    fn resolved(
        &mut self,
        particles: &GripParticles<'_>,
        triangles: &[[usize; 3]],
        layers: &[ContactLayer<'_>],
        radii: &[f64],
    ) -> Result<bool, ClothError> {
        let _ = layers;
        self.candidates_with(particles, triangles, radii);
        for (k, (pair, axis)) in self.pairs.iter().zip(&self.axes).enumerate() {
            let ids = pair.indices();
            if self.pair_resolved[k] > ids.map(|i| self.stamps[i]).into_iter().max().unwrap() {
                continue;
            }
            let gap = radii[ids[0]] + radii[ids[pair.split()]];
            if !pair.resolved(particles, gap, *axis)? {
                return Ok(false);
            }
            self.pair_resolved[k] = self.clock + 1;
        }
        Ok(true)
    }
}

/// Envelope padding for cached self-contact candidates, in metres.
const CANDIDATE_PADDING: f64 = 0.01;
/// Largest projection reserve the contact passes add to a pair's gap.
const CONTACT_RESERVE_LIMIT: f64 = 1e-6;

struct FaceSupport {
    ids: [usize; 3],
    weights: [f64; 3],
    normal: DVec3,
}

/// Dykstra corrections recover the mass-metric projection onto the
/// intersection of support half-spaces. Plain cyclic clipping can add
/// excessive support motion and even reverse the response to the load.
fn project_supports<const N: usize>(
    rows: &[&FaceSupport],
    ids: [usize; N],
    inverse_mass: &[f64],
    mut directions: [DVec3; N],
) -> [DVec3; N] {
    let mut corrections = vec![0.; rows.len()];
    for _ in 0..128 {
        let before = directions;
        for (slot, contact) in rows.iter().enumerate() {
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

#[cfg(test)]
struct FaceSupports {
    constraints: Vec<FaceSupport>,
    adjacent: Vec<Vec<usize>>,
}

#[cfg(test)]
impl FaceSupports {
    fn project(&self, ids: [usize; 4], inverse_mass: &[f64], directions: [DVec3; 4]) -> [DVec3; 4] {
        let mut rows: Vec<_> = ids
            .into_iter()
            .flat_map(|i| self.adjacent[i].iter().copied())
            .collect();
        rows.sort_unstable();
        rows.dedup();
        let rows: Vec<_> = rows.into_iter().map(|r| &self.constraints[r]).collect();
        project_supports(&rows, ids, inverse_mass, directions)
    }
}

/// Fixed-collider supports of material faces, evaluated on demand from one
/// position snapshot. Rows keep the (face, collider) order of a full sweep.
struct LazySupports<'a> {
    positions: &'a [DVec3],
    triangles: &'a [[usize; 3]],
    layers: &'a [ContactLayer<'a>],
    vertex_triangles: &'a [Vec<usize>],
    rows: std::cell::RefCell<std::collections::HashMap<usize, std::rc::Rc<Vec<FaceSupport>>>>,
}

impl LazySupports<'_> {
    fn face(&self, triangle: usize) -> Result<std::rc::Rc<Vec<FaceSupport>>, ClothError> {
        if let Some(rows) = self.rows.borrow().get(&triangle) {
            return Ok(rows.clone());
        }
        let ids = self.triangles[triangle];
        let layer = layer_for(self.layers, ids[0]);
        let vertices = ids.map(|i| self.positions[i]);
        let mut rows = Vec::new();
        for collider in layer.fixed {
            if let Some((weights, _, normal)) =
                collider.triangle_contact(vertices, layer.radius + 1e-8)?
            {
                rows.push(FaceSupport {
                    ids,
                    weights,
                    normal,
                });
            }
        }
        let rows = std::rc::Rc::new(rows);
        self.rows.borrow_mut().insert(triangle, rows.clone());
        Ok(rows)
    }

    fn project(
        &self,
        ids: [usize; 4],
        inverse_mass: &[f64],
        directions: [DVec3; 4],
    ) -> Result<[DVec3; 4], ClothError> {
        let mut faces: Vec<_> = ids
            .iter()
            .flat_map(|&i| self.vertex_triangles.get(i).into_iter().flatten().copied())
            .collect();
        faces.sort_unstable();
        faces.dedup();
        let faces = faces
            .into_iter()
            .map(|t| self.face(t))
            .collect::<Result<Vec<_>, _>>()?;
        let rows: Vec<_> = faces
            .iter()
            .flat_map(|rows| rows.iter())
            .filter(|row| {
                row.ids
                    .iter()
                    .zip(row.weights)
                    .any(|(i, w)| w > 0. && ids.contains(i))
            })
            .collect();
        if rows.is_empty() {
            return Ok(directions);
        }
        Ok(project_supports(&rows, ids, inverse_mass, directions))
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
