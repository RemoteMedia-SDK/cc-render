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
// Conservative-advancement hit/separation tolerance, 1 % of RESIDUAL. Grazing
// motion converges geometrically, so a tighter value can exhaust the search.
const SWEEP_TOLERANCE: f64 = 1e-6;
// Numerical penalty, including each sample's represented rest area. This is
// not an anatomical material measurement or a support-garment parameter.
const PRESSURE_STIFFNESS: f64 = 1e10;
const MAX_CONTACT_ROUNDS: usize = 8;
const MAX_PENALTY_SCALE: f64 = 4096.0;
// Engineering bound on hand/body-to-tissue contact pressure, far below the
// 500 kPa bulk modulus so contact cannot crush local volume. Not calibrated.
const DEFAULT_CONTACT_PRESSURE_PA: f64 = 10_000.0;

/// Bounded contact pressure. Overlap a capped sample cannot remove is permitted
/// and reported (warn by default, like seat overlap); `strict` rejects it.
#[derive(Clone, Copy, Debug)]
pub(super) struct ContactPolicy {
    pub max_pressure_pa: f64,
    pub strict: bool,
}

impl Default for ContactPolicy {
    fn default() -> Self {
        Self { max_pressure_pa: DEFAULT_CONTACT_PRESSURE_PA, strict: false }
    }
}

impl ContactPolicy {
    pub fn from_env() -> anyhow::Result<Self> {
        let max_pressure_pa = std::env::var("CC_AVATAR_TISSUE_CONTACT_PRESSURE_PA")
            .map_or(Ok(DEFAULT_CONTACT_PRESSURE_PA), |s| s.parse())?;
        anyhow::ensure!(max_pressure_pa > 0.0, "tissue contact pressure must be positive (inf disables the cap)");
        let strict = std::env::var("CC_AVATAR_TISSUE_STRICT_CONTACT").as_deref() == Ok("1");
        Ok(Self { max_pressure_pa, strict })
    }
}

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
    policy: ContactPolicy,
}

/// Planes per region, with the skin vertex each plane samples. `inherited`
/// marks a plane whose sample started the interval inside the clearance and
/// ends outside it: overlap carried over from a capped step that is receding.
struct Planes {
    contacts: Vec<Vec<PlaneContact>>,
    vertices: Vec<Vec<usize>>,
    inherited: Vec<Vec<bool>>,
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
    pub fn new(surfaces: &[TissueSurface], policy: ContactPolicy) -> anyhow::Result<Self> {
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
        Ok(Self { triangles, samples, policy })
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
            let Planes { mut contacts, vertices, .. } =
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
            // A sample at the pressure cap keeps its permitted overlap; every
            // other sample, including newly exposed faces, must meet RESIDUAL.
            let limited = force_limited_vertices(&contacts, &vertices, &solved);
            let accepted_overlap = !self.policy.strict
                || metrics.iter().all(|m| m.maximum_force_limited_penetration_m <= RESIDUAL);
            let valid = accepted_overlap
                && inner.vertices.iter().flatten().zip(inner.inherited.iter().flatten())
                    .all(|(v, inherited)| *inherited || limited.contains(v))
                && metrics.iter().all(|m| m.maximum_unlimited_penetration_m <= RESIDUAL);
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
            if round + 1 == MAX_CONTACT_ROUNDS && tracing::enabled!(tracing::Level::DEBUG) {
                log_unresolved_planes(&contacts, &inner.contacts, candidate, asset);
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
    ) -> anyhow::Result<Planes> {
        let started = std::time::Instant::now();
        let mut candidate_faces = 0;
        let mut result = vec![Vec::new(); end_solvers.len()];
        let mut vertices = vec![Vec::new(); end_solvers.len()];
        let mut inherited = vec![Vec::new(); end_solvers.len()];
        if self.samples.is_empty() {
            return Ok(Planes { contacts: result, vertices, inherited });
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
            let mut receding = Vec::new();
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
                    let inside = |x: [DVec3; 4]| -> anyhow::Result<bool> {
                        Ok(self::witness(x)?.distance <= clearance + SWEEP_TOLERANCE)
                    };
                    receding.push(inside(from)? && !inside(to)?);
                    local.push(PlaneContact {
                        point: end_embedded,
                        surface_point,
                        normal: witness.normal,
                        stiffness_n_per_m: PRESSURE_STIFFNESS * sample.area,
                        max_force_n: self.policy.max_pressure_pa * sample.area,
                    });
                }
            }
            // Neighboring faces represent one surface sample, not duplicated area.
            let count = local.len() as f64;
            for plane in &mut local {
                plane.stiffness_n_per_m /= count;
                plane.max_force_n /= count;
            }
            vertices[sample.binding.region].extend(std::iter::repeat_n(sample.vertex, local.len()));
            inherited[sample.binding.region].extend(receding);
            result[sample.binding.region].extend(local);
        }
        tracing::debug!(
            queries_ms = (started.elapsed() - tree_time).as_secs_f64() * 1000.0,
            candidate_faces,
            "tissue contact search profile"
        );
        Ok(Planes { contacts: result, vertices, inherited })
    }
}

/// Skin vertices with at least one plane at the force cap in the solved state.
fn force_limited_vertices(
    contacts: &[Vec<PlaneContact>],
    vertices: &[Vec<usize>],
    solved: &[Tissue],
) -> std::collections::BTreeSet<usize> {
    contacts.iter().zip(vertices).zip(solved)
        .flat_map(|((planes, vertices), tissue)| planes.iter().zip(vertices)
            .filter(|(plane, _)| plane.is_force_limited(tissue.contact_depth(plane)))
            .map(|(_, v)| *v))
        .collect()
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

/// Diagnostic only: the final planes of an unresolved step and how much of each
/// sample's embedding the solver can move (barycentric weight on free nodes).
fn log_unresolved_planes(
    solved: &[Vec<PlaneContact>],
    remaining: &[Vec<PlaneContact>],
    tissue: &[Tissue],
    asset: &TissueAsset,
) {
    for (region, planes) in solved.iter().enumerate() {
        let attachments = &asset.regions[region].attachments;
        let tetrahedra = &asset.regions[region].tetrahedra;
        for plane in planes {
            let ids = tetrahedra[plane.point.tetrahedron];
            let free_weight: f64 = ids.iter().zip(plane.point.barycentric)
                .filter(|(i, _)| !matches!(attachments[**i], crate::cc_render::soft_tissue::Attachment::Fixed))
                .map(|(_, w)| w * plane.point.influence)
                .sum();
            let point = plane.point.offset + ids.iter().zip(plane.point.barycentric)
                .map(|(i, w)| tissue[region].positions()[*i] * (w * plane.point.influence))
                .sum::<DVec3>();
            tracing::debug!(
                region,
                tetrahedron = plane.point.tetrahedron,
                nodes = ?ids,
                attachments = ?ids.map(|i| attachments[i]),
                barycentric = ?plane.point.barycentric,
                free_weight,
                normal = ?plane.normal,
                depth_m = -plane.normal.dot(point - plane.surface_point),
                stiffness_n_per_m = plane.stiffness_n_per_m,
                "unresolved tissue contact plane"
            );
        }
        for plane in &remaining[region] {
            tracing::debug!(region, tetrahedron = plane.point.tetrahedron, normal = ?plane.normal,
                surface_point = ?plane.surface_point, "remaining inner tissue contact plane");
        }
    }
}

fn increase_penalty(scales: &mut [f64], metrics: &[ContactMetrics]) {
    // Finite penalty compliance does not vanish by repeating the same solve or
    // reducing dt (which increases inertial resistance). Aim below the residual,
    // bounded to avoid unbounded ill-conditioning. This is numerical contact
    // enforcement, not tissue stiffness.
    for (scale, metric) in scales.iter_mut().zip(metrics) {
        // Raising the penalty cannot move a sample already at the force cap.
        if metric.maximum_unlimited_penetration_m > RESIDUAL {
            let increase = (2.0 * metric.maximum_unlimited_penetration_m / RESIDUAL).clamp(2.0, 8.0);
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

fn swept_bounds(points: impl IntoIterator<Item = DVec3>, radius: f64) -> Aabb {
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
struct Witness {
    barycentric: [f64; 3],
    normal: DVec3,
    distance: f64,
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
        barycentric,
        normal: delta / distance,
        distance,
    })
}

/// Conservative advancement for a linearly moving point and triangle. The
/// maximum relative vertex speed bounds change of closest-point distance.
fn sweep(old: [DVec3; 4], end: [DVec3; 4], radius: f64) -> anyhow::Result<Option<Witness>> {
    let motion = std::array::from_fn::<_, 4, _>(|i| end[i] - old[i]);
    let speed = (1..4)
        .map(|i| (motion[0] - motion[i]).length())
        .fold(0.0_f64, f64::max);
    let mut time = 0.0;
    for _ in 0..256 {
        let at = std::array::from_fn(|i| old[i] + time * motion[i]);
        let hit = witness(at)?;
        if hit.distance <= radius + SWEEP_TOLERANCE {
            return Ok(Some(hit));
        }
        // A separating plane is a lower bound over the entire interval.
        if [at, end]
            .into_iter()
            .all(|p| (1..4).all(|i| (p[0] - p[i]).dot(hit.normal) > radius + SWEEP_TOLERANCE))
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
    Err(ContactUnresolved.into())
}

#[cfg(test)]
#[path = "tissue_collision_tests.rs"]
mod tests;
