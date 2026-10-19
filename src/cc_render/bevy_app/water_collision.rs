//! Read-only swept render-surface queries. Reuses the tissue point/triangle CCD.
pub(super) use super::tissue_collision::swept_bounds as bounds;
use bevy::{math::DVec3, prelude::*};
use parry_ccd::{
    bounding_volume::{Aabb, BoundingVolume},
    math::Point,
    partitioning::{Qbvh, QbvhUpdateWorkspace},
    query::PointQueryWithLocation,
    shape::Triangle,
};

pub(super) const RADIUS: f64 = 0.0015;

#[derive(Default, Resource)]
pub(super) struct WaterSurfaces {
    pub old: Vec<DVec3>,
    pub current: Vec<DVec3>,
    pub triangles: Vec<[usize; 3]>,
    tree: Qbvh<usize>,
    leaves: Vec<usize>,
    workspace: QbvhUpdateWorkspace,
    neighbors: Vec<Vec<usize>>,
    valid_faces: Vec<bool>,
    pub error: Option<String>,
    pub meshes: usize,
    pub revision: u64,
    pub topology_revision: u64,
    pub update_ms: f64,
    pub degenerate_triangles: usize,
    pub history_valid: bool,
    /// Read-only palm regions for contact diagnostics; never a collision substitute.
    pub palms: [Option<DVec3>; 2],
}

pub(super) struct Hit {
    pub face: usize,
    pub weights: [f64; 3],
    pub normal: DVec3,
    pub fraction: f64,
}

impl WaterSurfaces {
    pub fn update(
        &mut self,
        positions: Vec<DVec3>,
        triangles: Vec<[usize; 3]>,
        regions: &[Aabb],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            positions.iter().all(|p| p.is_finite())
                && triangles.iter().flatten().all(|i| *i < positions.len()),
            "invalid water collision geometry"
        );
        let topology_changed = self.current.len() != positions.len() || self.triangles != triangles;
        self.old = if self.history_valid
            && self.current.len() == positions.len()
            && self.triangles == triangles
        {
            std::mem::replace(&mut self.current, positions)
        } else {
            self.topology_revision += 1;
            self.current = positions;
            self.current.clone()
        };
        self.history_valid = true;
        self.triangles = triangles;
        if topology_changed {
            self.build_neighbors();
        }
        self.degenerate_triangles = 0;
        self.valid_faces.resize(self.triangles.len(), true);
        let selected: Vec<_> = self
            .triangles
            .iter()
            .enumerate()
            .filter_map(|(id, t)| {
                let a = t.map(|i| self.old[i]);
                let b = t.map(|i| self.current[i]);
                let flat =
                    |v: [DVec3; 3]| (v[1] - v[0]).cross(v[2] - v[0]).length_squared() <= 1e-24;
                if flat(a) && flat(b) && flat(std::array::from_fn(|i| a[i].lerp(b[i], 0.5))) {
                    self.degenerate_triangles += 1;
                    self.valid_faces[id] = false;
                } else {
                    self.valid_faces[id] = true;
                }
                let aabb = bounds(
                    t.iter().flat_map(|i| [self.old[*i], self.current[*i]]),
                    RADIUS,
                );
                regions
                    .iter()
                    .any(|r| r.intersects(&aabb))
                    .then_some((id, aabb))
            })
            .collect();
        let leaves: Vec<_> = selected.iter().map(|(id, _)| *id).collect();
        if self.leaves != leaves || self.revision % 120 == 0 {
            self.tree.clear_and_rebuild(selected.into_iter(), 0.0);
            self.leaves = leaves;
        } else {
            for id in &self.leaves {
                self.tree.pre_update_or_insert(*id);
            }
            let mut boxes = vec![Aabb::new_invalid(); self.triangles.len()];
            for (id, aabb) in selected {
                boxes[id] = aabb;
            }
            self.tree.refit(0., &mut self.workspace, |id| boxes[*id]);
        }
        self.revision += 1;
        self.error = None;
        Ok(())
    }

    pub fn triangle(&self, face: usize, fraction: f64) -> [DVec3; 3] {
        self.triangles[face].map(|i| self.old[i].lerp(self.current[i], fraction))
    }

    fn build_neighbors(&mut self) {
        let mut edges = std::collections::HashMap::new();
        self.neighbors = vec![Vec::new(); self.triangles.len()];
        for (face, t) in self.triangles.iter().enumerate() {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                if a == b {
                    continue;
                }
                if let Some(other) = edges.insert((a.min(b), a.max(b)), face) {
                    self.neighbors[face].push(other);
                    self.neighbors[other].push(face);
                }
            }
        }
        for faces in &mut self.neighbors {
            faces.sort_unstable();
            faces.dedup();
        }
    }

    pub fn point(&self, face: usize, weights: [f64; 3], fraction: f64) -> DVec3 {
        self.triangle(face, fraction)
            .into_iter()
            .zip(weights)
            .map(|(p, w)| p * w)
            .sum()
    }

    pub fn sweep(
        &self,
        start: DVec3,
        end: DVec3,
        from: f64,
        to: f64,
    ) -> anyhow::Result<Option<Hit>> {
        let mut candidates = Vec::new();
        self.tree
            .intersect_aabb(&bounds([start, end], RADIUS), &mut candidates);
        let mut nearest: Option<Hit> = None;
        for face in candidates {
            if !self.valid_faces[face] {
                continue;
            }
            let a = self.triangle(face, from);
            let b = self.triangle(face, to);
            if let Some(hit) = water_sweep([start, a[0], a[1], a[2]], [end, b[0], b[1], b[2]], 4)? {
                // Adjacent faces can overlap a parcel's contact shell. A
                // separating initial contact must not repeatedly reattach it.
                let surface_motion: DVec3 =
                    (0..3).map(|i| (b[i] - a[i]) * hit.barycentric[i]).sum();
                if hit.fraction == 0.0 && ((end - start) - surface_motion).dot(hit.normal) >= 0.0 {
                    continue;
                }
                if nearest
                    .as_ref()
                    .is_none_or(|old| hit.fraction < old.fraction)
                {
                    nearest = Some(Hit {
                        face,
                        weights: hit.barycentric,
                        normal: hit.normal,
                        fraction: hit.fraction,
                    });
                }
            }
        }
        Ok(nearest)
    }

    pub fn project(&self, face: usize, point: DVec3, normal: DVec3, fraction: f64) -> Option<Hit> {
        // Follow the connected surface, not unrelated nearby fingers, hair
        // cards, or the opposite side of the body. A boundary sheds the film.
        let mut current = face;
        let mut best = f64::INFINITY;
        let mut result = None;
        for _ in 0..32 {
            let previous = current;
            for face in std::iter::once(previous).chain(self.neighbors[previous].iter().copied()) {
                if !self.valid_faces[face] {
                    continue;
                }
                let verts = self.triangle(face, fraction);
                let Some(mut n) = (verts[1] - verts[0])
                    .cross(verts[2] - verts[0])
                    .try_normalize()
                else {
                    continue;
                };
                if n.dot(normal) < 0.0 {
                    n = -n;
                }
                if n.dot(normal) < 0.3 {
                    continue;
                }
                let tri = Triangle::new(
                    verts[0].to_array().into(),
                    verts[1].to_array().into(),
                    verts[2].to_array().into(),
                );
                let (p, loc) =
                    tri.project_local_point_and_get_location(&Point::from(point.to_array()), false);
                let distance = point.distance(DVec3::from_array(p.point.coords.into()));
                if distance < best {
                    let Some(weights) = loc.barycentric_coordinates() else {
                        continue;
                    };
                    best = distance;
                    current = face;
                    result = Some(Hit {
                        face,
                        weights,
                        normal: n,
                        fraction,
                    });
                }
            }
            if current == previous {
                break;
            }
        }
        result.filter(|_| best < 0.003)
    }
}

fn water_sweep(
    old: [DVec3; 4],
    end: [DVec3; 4],
    splits: u8,
) -> anyhow::Result<Option<super::tissue_collision::Witness>> {
    match super::tissue_collision::sweep_with_tolerance(old, end, RADIUS, 1e-5) {
        Ok(hit) => Ok(hit),
        Err(error) if splits == 0 => Err(error),
        Err(_) => {
            // Split the same linear trajectory; never skip an unresolved interval.
            let middle = std::array::from_fn(|i| old[i].lerp(end[i], 0.5));
            if let Some(mut hit) = water_sweep(old, middle, splits - 1)? {
                hit.fraction *= 0.5;
                return Ok(Some(hit));
            }
            let mut hit = water_sweep(middle, end, splits - 1)?;
            if let Some(hit) = &mut hit {
                hit.fraction = 0.5 + hit.fraction * 0.5;
            }
            Ok(hit)
        }
    }
}
