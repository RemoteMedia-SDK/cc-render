//! Read-only, source-time skin contact. Body motion stays kinematic; only the
//! embedded tissue degrees of freedom receive the normal reaction.
//! Sweeps certify linear vertex trajectories, not the unsampled curved motion.
use super::tissue_surface::TissueSurface;
use crate::cc_render::soft_tissue::{
    ContactMetrics, EmbeddedPoint, Frame, PlaneContact, Tissue, TissueAsset, VertexBinding,
};
use anyhow::Context;
use bevy::math::DVec3;
use parry_ccd::{
    bounding_volume::{Aabb, BoundingVolume},
    math::Point,
    partitioning::Qbvh,
    query::PointQueryWithLocation,
    shape::Triangle,
};

const CLEARANCE: f64 = 0.0005;
const RESIDUAL: f64 = 0.0001;
// Numerical penalty, including each sample's represented rest area. This is
// not an anatomical material measurement or a support-garment parameter.
const PRESSURE_STIFFNESS: f64 = 1e10;
const MAX_CONTACT_ROUNDS: usize = 8;
const MAX_PENALTY_SCALE: f64 = 4096.0;

#[derive(Debug, thiserror::Error)]
#[error("tissue skin contact did not resolve within the sampled step")]
pub(super) struct ContactUnresolved;

pub(super) struct ContactPose {
    pub frames: Vec<Frame>,
    pub skin: Vec<DVec3>,
}

struct SkinSample {
    vertex: usize,
    binding: VertexBinding,
    area: f64,
}

pub(super) struct BodyContacts {
    triangles: Vec<[usize; 3]>,
    samples: Vec<SkinSample>,
}

/// Every sample query lies inside `region`, so a face whose swept bounds do not
/// intersect it cannot be a candidate. Dense local IDs avoid allocating Qbvh
/// proxies for excluded faces. Queries restore stable original face order.
struct LocalTree {
    tree: Qbvh<usize>,
    faces: Vec<usize>,
}

impl LocalTree {
    fn new(triangles: &[[usize; 3]], vertices: &[Aabb], region: &Aabb) -> Self {
        let mut faces = Vec::new();
        let mut selected = Vec::new();
        for (i, [a, b, c]) in triangles.iter().enumerate() {
            let bounds = vertices[*a].merged(&vertices[*b]).merged(&vertices[*c]);
            if bounds.intersects(region) {
                selected.push((faces.len(), bounds));
                faces.push(i);
            }
        }
        let mut tree = Qbvh::new();
        tree.clear_and_rebuild(selected.into_iter(), 0.0);
        Self { tree, faces }
    }

    fn query(&self, bounds: &Aabb, hits: &mut Vec<usize>) {
        hits.clear();
        self.tree.intersect_aabb(bounds, hits);
        for face in hits.iter_mut() {
            *face = self.faces[*face];
        }
        hits.sort_unstable();
    }
}

impl BodyContacts {
    pub fn new(surfaces: &[TissueSurface]) -> anyhow::Result<Self> {
        let mut triangles = Vec::new();
        let mut samples = Vec::new();
        let mut offset = 0;
        for surface in surfaces {
            triangles.extend(
                surface
                    .collision_triangles
                    .iter()
                    .map(|t| t.map(|i| i + offset)),
            );
            let areas = surface.sample_areas();
            let mut unique = std::collections::BTreeSet::new();
            for binding in &surface.binding().vertices {
                anyhow::ensure!(
                    unique.insert(binding.vertex)
                        && binding.influence > 0.0
                        && areas[binding.vertex] > 0.0,
                    "body contact requires a single positive embedding per skin sample"
                );
                samples.push(SkinSample {
                    vertex: binding.vertex + offset,
                    binding: binding.clone(),
                    area: areas[binding.vertex],
                });
            }
            offset += surface.binding().vertex_count;
        }
        Ok(Self { triangles, samples })
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.triangles.len(), self.samples.len())
    }

    /// Re-solve from the original physical state for each refreshed contact set.
    /// Never integrate a step twice or project the rendered positions afterward.
    pub fn resolve(
        &self,
        original: &[Tissue],
        candidate: &mut Vec<Tissue>,
        asset: &TissueAsset,
        old: &ContactPose,
        end: &ContactPose,
        dt: f64,
    ) -> anyhow::Result<Vec<ContactMetrics>> {
        if self.samples.is_empty() || self.triangles.is_empty() {
            return Ok(vec![ContactMetrics::default(); original.len()]);
        }
        anyhow::ensure!(
            old.skin.len() == end.skin.len(),
            "contact skin vertex count changed"
        );
        let started = std::time::Instant::now();
        // Shared vertices otherwise repeat this endpoint work for every face.
        // Padding before union yields the same conservative triangle bounds.
        let bounds: Vec<_> = old
            .skin
            .iter()
            .zip(&end.skin)
            .map(|(a, b)| swept_bounds([*a, *b], CLEARANCE))
            .collect();
        tracing::debug!(
            broad_phase_ms = started.elapsed().as_secs_f64() * 1000.0,
            "tissue contact search profile"
        );
        let mut penalty_scale = vec![1.0; original.len()];
        for round in 0..MAX_CONTACT_ROUNDS {
            let mut contacts =
                self.planes(&bounds, original, candidate, asset, old, end, CLEARANCE)?;
            if contacts.iter().all(Vec::is_empty) {
                return Ok(vec![ContactMetrics::default(); original.len()]);
            }
            let (solved, metrics) =
                solve_contact_step(original, &mut contacts, &end.frames, dt, &penalty_scale)?;
            let inner = self.planes(
                &bounds,
                original,
                &solved,
                asset,
                old,
                end,
                CLEARANCE - RESIDUAL,
            )?;
            let valid = inner.iter().all(Vec::is_empty)
                && metrics.iter().all(|m| m.maximum_penetration_m <= RESIDUAL);
            tracing::debug!(
                round,
                dt,
                ?penalty_scale,
                ?metrics,
                valid,
                "tissue contact re-solve"
            );
            *candidate = solved;
            if valid {
                return Ok(metrics);
            }
            increase_penalty(&mut penalty_scale, &metrics);
        }
        Err(ContactUnresolved.into())
    }

    fn planes(
        &self,
        bounds: &[Aabb],
        old_solvers: &[Tissue],
        end_solvers: &[Tissue],
        asset: &TissueAsset,
        old: &ContactPose,
        end: &ContactPose,
        clearance: f64,
    ) -> anyhow::Result<Vec<Vec<PlaneContact>>> {
        let started = std::time::Instant::now();
        let mut candidate_faces = 0;
        let mut result = vec![Vec::new(); end_solvers.len()];
        if self.samples.is_empty() {
            return Ok(result);
        }
        let paths: Vec<_> = self
            .samples
            .iter()
            .map(|sample| {
                let (a, _) =
                    sample_position(sample, asset, old, &old_solvers[sample.binding.region]);
                let (b, point) =
                    sample_position(sample, asset, end, &end_solvers[sample.binding.region]);
                (sample, a, b, point)
            })
            .collect();
        let tree_started = std::time::Instant::now();
        // Recompute the region after every contact re-solve: a correction may
        // leave the first candidate's envelope and expose a different body face.
        let region = swept_bounds(paths.iter().flat_map(|(_, a, b, _)| [*a, *b]), clearance);
        let tree = LocalTree::new(&self.triangles, bounds, &region);
        let tree_time = tree_started.elapsed();
        tracing::debug!(
            broad_phase_ms = tree_time.as_secs_f64() * 1000.0,
            retained_faces = tree.faces.len(),
            "tissue contact search profile"
        );
        let mut hits = Vec::new();
        for (sample, a, b, end_embedded) in paths {
            tree.query(&swept_bounds([a, b], clearance), &mut hits);
            candidate_faces += hits.len();
            let mut local = Vec::new();
            for face in &hits {
                let t = self.triangles[*face];
                let from = [a, old.skin[t[0]], old.skin[t[1]], old.skin[t[2]]];
                let to = [b, end.skin[t[0]], end.skin[t[1]], end.skin[t[2]]];
                if let Some(witness) = sweep(from, to, clearance).with_context(||
                    format!("skin sample {}, face {:?}, clearance {clearance}, from {from:?}, to {to:?}",
                        sample.vertex, t))? {
                    let surface_point = (0..3)
                        .map(|i| to[i + 1] * witness.barycentric[i])
                        .sum::<DVec3>()
                        + witness.normal * clearance;
                    local.push(PlaneContact {
                        point: end_embedded,
                        surface_point,
                        normal: witness.normal,
                        stiffness_n_per_m: PRESSURE_STIFFNESS * sample.area,
                    });
                }
            }
            // Neighboring faces represent one surface sample, not duplicated area.
            let count = local.len() as f64;
            for plane in &mut local {
                plane.stiffness_n_per_m /= count;
            }
            result[sample.binding.region].extend(local);
        }
        tracing::debug!(
            queries_ms = (started.elapsed() - tree_time).as_secs_f64() * 1000.0,
            candidate_faces,
            "tissue contact search profile"
        );
        Ok(result)
    }
}

fn solve_contact_step(
    original: &[Tissue],
    contacts: &mut [Vec<PlaneContact>],
    frames: &[Frame],
    dt: f64,
    penalty_scale: &[f64],
) -> anyhow::Result<(Vec<Tissue>, Vec<ContactMetrics>)> {
    let mut solved = original.to_vec();
    let mut metrics = Vec::with_capacity(solved.len());
    for (i, solver) in solved.iter_mut().enumerate() {
        for plane in &mut contacts[i] {
            plane.stiffness_n_per_m *= penalty_scale[i];
        }
        metrics.push(
            solver
                .step_with_contacts(dt, frames[i], DVec3::NEG_Y * 9.81, &contacts[i])
                .with_context(|| format!("tissue contact solve region {i}"))?,
        );
    }
    Ok((solved, metrics))
}

fn increase_penalty(scales: &mut [f64], metrics: &[ContactMetrics]) {
    // Finite penalty compliance does not vanish by repeating the same solve or
    // reducing dt (which increases inertial resistance). Aim below the residual,
    // bounded to avoid unbounded ill-conditioning. This is numerical contact
    // enforcement, not tissue stiffness.
    for (scale, metric) in scales.iter_mut().zip(metrics) {
        if metric.maximum_penetration_m > RESIDUAL {
            let increase = (2.0 * metric.maximum_penetration_m / RESIDUAL).clamp(2.0, 8.0);
            *scale = (*scale * increase).min(MAX_PENALTY_SCALE);
        }
    }
}

fn sample_position(
    sample: &SkinSample,
    asset: &TissueAsset,
    pose: &ContactPose,
    solver: &Tissue,
) -> (DVec3, EmbeddedPoint) {
    let binding = &sample.binding;
    let region = &asset.regions[binding.region];
    let rest: DVec3 = region.tetrahedra[binding.tetrahedron]
        .iter()
        .zip(binding.barycentric)
        .map(|(i, w)| {
            pose.frames[binding.region].point(region.initial_positions[*i].into())
                * (w * binding.influence)
        })
        .sum();
    let offset = pose.skin[sample.vertex] - rest;
    (
        offset
            + region.tetrahedra[binding.tetrahedron]
                .iter()
                .zip(binding.barycentric)
                .map(|(i, w)| solver.positions()[*i] * (w * binding.influence))
                .sum::<DVec3>(),
        EmbeddedPoint {
            tetrahedron: binding.tetrahedron,
            barycentric: binding.barycentric,
            influence: binding.influence,
            offset,
        },
    )
}

pub(super) fn swept_bounds(points: impl IntoIterator<Item = DVec3>, radius: f64) -> Aabb {
    let (lo, hi) = points.into_iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    );
    Aabb::new(
        Point::from((lo - DVec3::splat(radius)).to_array()),
        Point::from((hi + DVec3::splat(radius)).to_array()),
    )
}

#[derive(Debug)]
pub(super) struct Witness {
    pub barycentric: [f64; 3],
    pub normal: DVec3,
    pub distance: f64,
    pub fraction: f64,
}

fn witness(x: [DVec3; 4]) -> anyhow::Result<Witness> {
    // Query relative to the point for translation-invariant f64 precision.
    let [a, b, c] = [x[1], x[2], x[3]].map(|v| Point::from((v - x[0]).to_array()));
    let (_, location) =
        Triangle::new(a, b, c).project_local_point_and_get_location(&Point::origin(), false);
    let barycentric = location
        .barycentric_coordinates()
        .ok_or(ContactUnresolved)?;
    let delta: DVec3 = (0..3).map(|i| (x[0] - x[i + 1]) * barycentric[i]).sum();
    let distance = delta.length();
    anyhow::ensure!(distance.is_finite() && distance > 1e-12, ContactUnresolved);
    Ok(Witness {
        fraction: 0.0,
        barycentric,
        normal: delta / distance,
        distance,
    })
}

/// Conservative advancement for a linearly moving point and triangle. The
/// maximum relative vertex speed bounds change of closest-point distance.
pub(super) fn sweep(old: [DVec3; 4], end: [DVec3; 4], radius: f64) -> anyhow::Result<Option<Witness>> {
    sweep_with_tolerance(old, end, radius, 1e-8)
}

pub(super) fn sweep_with_tolerance(old: [DVec3; 4], end: [DVec3; 4], radius: f64, tolerance: f64) -> anyhow::Result<Option<Witness>> {
    let motion = std::array::from_fn::<_, 4, _>(|i| end[i] - old[i]);
    let speed = (1..4)
        .map(|i| (motion[0] - motion[i]).length())
        .fold(0.0_f64, f64::max);
    let mut time = 0.0;
    for _ in 0..256 {
        let at = std::array::from_fn(|i| old[i] + time * motion[i]);
        let mut hit = witness(at)?;
        if hit.distance <= radius + tolerance {
            hit.fraction = time;
            return Ok(Some(hit));
        }
        // A separating plane is a lower bound over the entire interval.
        if [at, end]
            .into_iter()
            .all(|p| (1..4).all(|i| (p[0] - p[i]).dot(hit.normal) > radius + tolerance))
        {
            return Ok(None);
        }
        if speed < 1e-12 || hit.distance - radius > speed * (1.0 - time) {
            return Ok(None);
        }
        time += 0.9 * (hit.distance - radius) / speed;
        if time > 1.0 {
            return Ok(None);
        }
    }
    Err(anyhow::Error::new(ContactUnresolved).context("point/triangle advancement exhausted"))
}

#[cfg(test)]
#[path = "tissue_collision_tests.rs"]
mod tests;
