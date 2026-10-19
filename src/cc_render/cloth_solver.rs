//! CPU cloth prototype: area-lumped particles, compliant distance constraints,
//! and swept/discrete particle/triangle contacts with fixed colliders. No body-motion writes.
//!
//! Small-step XPBD with accumulated multipliers across material passes
//! (Macklin et al., https://mmacklin.com/xpbd.pdf). Opposite-vertex springs
//! approximate bending;
//! they are not a calibrated shell material. Swept self-contact is frictionless;
//! dynamic-body coupling and runtime hand admission are not implemented here.
//! The crate-private contact clamp is an explicitly limited coupling prototype.
use std::collections::BTreeMap;

const CONSTRAINT_PASSES: usize = 4;

use bevy::math::{DQuat, DVec3, Vec2, Vec3};
use bevy_rapier3d::rapier::parry::{math::Isometry, shape::SharedShape};

use super::cloth_grip::{ClothGrip, GripParticles};
use super::deformable_surface::{DeformedSurface, SurfaceError};

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClothParameters {
    pub total_mass_kg: f64,
    pub thickness_m: f32,
    /// Inverse spring stiffness in metres/newton; topology dependent.
    pub stretch_compliance: f64,
    pub bend_compliance: f64,
    pub air_drag_per_second: f64,
    pub spring_damping_ratio: f64,
    pub substeps: usize,
}

impl ClothParameters {
    fn validate(&self) -> Result<(), ClothError> {
        if !self.total_mass_kg.is_finite()
            || self.total_mass_kg <= 0.0
            || !self.thickness_m.is_finite()
            || self.thickness_m <= 0.0
            || [
                self.stretch_compliance,
                self.bend_compliance,
                self.air_drag_per_second,
                self.spring_damping_ratio,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
            || !(1..=256).contains(&self.substeps)
        {
            return Err(ClothError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClothError {
    #[error(transparent)]
    Surface(#[from] SurfaceError),
    #[error("invalid cloth parameters, topology, timestep or collider")]
    InvalidInput,
    #[error("unsupported fixed collider contact")]
    UnsupportedContact,
    #[error("cloth step produced nonfinite state")]
    NonfiniteState,
    #[error("swept cloth contact could not be resolved conservatively")]
    SweptContactUnresolved,
    #[error("cloth self contact could not be resolved conservatively")]
    SelfContactUnresolved,
    #[error("cloth rigid impact has no movable contact degrees of freedom")]
    RigidContactUnresolved,
}

/// An immutable collider snapshot. Never used to move its owning rigid body.
/// Only fixed colliders belong in this prototype; dynamic response is absent.
pub struct FixedClothCollider {
    shape: SharedShape,
    // Double-precision polyhedral core; swept queries add the rounded border analytically.
    sweep_shape: parry_ccd::shape::SharedShape,
    center: DVec3,
    rotation: DQuat,
    half_extents: DVec3,
    border: f64,
    friction: f64,
    bounds_min: DVec3,
    bounds_max: DVec3,
}

impl FixedClothCollider {
    /// Only cuboids and rounded cuboids are supported. Their exact signed
    /// distance avoids support-map penetration noise at sub-millimetre scales.
    pub fn new(shape: SharedShape, pose: Isometry<f32>, friction: f64) -> Result<Self, ClothError> {
        let (half_extents, border) = if let Some(cuboid) = shape.as_cuboid() {
            (cuboid.half_extents, 0.0)
        } else if let Some(rounded) = shape.as_round_cuboid() {
            (rounded.inner_shape.half_extents, rounded.border_radius)
        } else {
            return Err(ClothError::UnsupportedContact);
        };
        if !friction.is_finite()
            || friction < 0.0
            || !pose
                .translation
                .vector
                .iter()
                .chain(pose.rotation.coords.iter())
                .all(|x| x.is_finite())
            || (pose.rotation.norm_squared() - 1.0).abs() > 1e-4
            || half_extents.iter().any(|v| !v.is_finite() || *v <= 0.0)
            || !border.is_finite()
            || border < 0.0
        {
            return Err(ClothError::InvalidInput);
        }
        let t = pose.translation.vector;
        let q = pose.rotation.quaternion();
        let center = DVec3::new(t.x as f64, t.y as f64, t.z as f64);
        let rotation = DQuat::from_xyzw(q.i as f64, q.j as f64, q.k as f64, q.w as f64).normalize();
        let half_extents = DVec3::new(
            half_extents.x as f64,
            half_extents.y as f64,
            half_extents.z as f64,
        );
        let extent = (rotation * DVec3::X).abs() * half_extents.x
            + (rotation * DVec3::Y).abs() * half_extents.y
            + (rotation * DVec3::Z).abs() * half_extents.z
            + DVec3::splat(border as f64);
        Ok(Self {
            sweep_shape: parry_ccd::shape::SharedShape::cuboid(
                half_extents.x,
                half_extents.y,
                half_extents.z,
            ),
            shape,
            center,
            rotation,
            half_extents,
            border: border as f64,
            friction,
            bounds_min: center - extent,
            bounds_max: center + extent,
        })
    }

    fn intersects_bounds(&self, min: DVec3, max: DVec3, radius: f64) -> bool {
        self.bounds_min.cmple(max + DVec3::splat(radius)).all()
            && self.bounds_max.cmpge(min - DVec3::splat(radius)).all()
    }

    fn distance_and_normal(&self, position: DVec3) -> (f64, DVec3) {
        let local = self.rotation.conjugate() * (position - self.center);
        let q = local.abs() - self.half_extents;
        let outside = q.max(DVec3::ZERO);
        let length = outside.length();
        let signs = DVec3::new(
            if local.x < 0.0 { -1.0 } else { 1.0 },
            if local.y < 0.0 { -1.0 } else { 1.0 },
            if local.z < 0.0 { -1.0 } else { 1.0 },
        );
        let normal = if length > 0.0 {
            outside / length * signs
        } else if q.x >= q.y && q.x >= q.z {
            DVec3::X * signs.x
        } else if q.y >= q.z {
            DVec3::Y * signs.y
        } else {
            DVec3::Z * signs.z
        };
        (
            length + q.max_element().min(0.0) - self.border,
            self.rotation * normal,
        )
    }

    fn project_particles(
        &self,
        positions: &mut [DVec3],
        previous: &[DVec3],
        radius: f64,
    ) -> (DVec3, DVec3) {
        let mut bounds = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for (position, old) in positions.iter_mut().zip(previous) {
            self.project(position, *old, radius);
            bounds.0 = bounds.0.min(*position);
            bounds.1 = bounds.1.max(*position);
        }
        bounds
    }

    fn project(&self, position: &mut DVec3, previous: DVec3, radius: f64) {
        if !self.intersects_bounds(*position, *position, radius) {
            return;
        }
        let (distance, normal) = self.distance_and_normal(*position);
        let depth = radius - distance;
        if depth <= 0.0 {
            return;
        }
        *position += normal * depth;
        let displacement = *position - previous;
        let tangent = displacement - normal * displacement.dot(normal);
        let length = tangent.length();
        if length > 0.0 {
            // Coulomb bound from the normal position correction, no sticky pin.
            *position -= tangent * (self.friction * depth / length).min(1.0);
        }
    }
}

struct DistanceSpring {
    vertices: [usize; 2],
    rest: f64,
    compliance: f64,
    damping_ratio: f64,
    stretch: bool,
}

impl DistanceSpring {
    fn project(&self, particles: &mut GripParticles<'_>, accumulated: &mut f64) {
        let positions = &mut *particles.positions;
        let previous = particles.previous;
        let inverse_mass = particles.inverse_mass;
        let h = particles.h;
        let [a, b] = self.vertices;
        let delta = positions[a] - positions[b];
        let length = delta.length();
        if length <= f64::EPSILON {
            return;
        }
        // XPBD Eq. 26 (https://mmacklin.com/xpbd.pdf): accumulated elastic
        // and Rayleigh damping multiplier; reset once per physical substep.
        // beta = 2*zeta*sqrt(k*m_eff), gamma = compliance*beta/h.
        let inverse_effective_mass = inverse_mass[a] + inverse_mass[b];
        let gamma =
            2.0 * self.damping_ratio * (self.compliance / inverse_effective_mass).sqrt() / h;
        let normal = delta / length;
        let relative_displacement =
            normal.dot((positions[a] - previous[a]) - (positions[b] - previous[b]));
        let alpha = self.compliance / (h * h);
        let lambda = -(length - self.rest + alpha * *accumulated + gamma * relative_displacement)
            / ((1.0 + gamma) * inverse_effective_mass + alpha);
        *accumulated += lambda;
        let correction = normal * lambda;
        positions[a] += correction * inverse_mass[a];
        positions[b] -= correction * inverse_mass[b];
    }
}

/// Working constraints commit only after the whole cloth tick succeeds.
struct PendingConstraints {
    multipliers: Vec<f64>,
    grips: Vec<ClothGrip>,
}

impl PendingConstraints {
    fn new(solver: &ClothSolver, grips: &[ClothGrip]) -> Result<Self, ClothError> {
        let mut grips = grips.to_vec();
        for grip in &mut grips {
            grip.prepare(&solver.surface)?;
        }
        Ok(Self {
            multipliers: vec![0.; solver.springs.len()],
            grips,
        })
    }

    fn project(
        &mut self,
        solver: &ClothSolver,
        particles: &mut GripParticles<'_>,
        interval: [f64; 2],
        pass: usize,
    ) -> Result<(), ClothError> {
        // Grip capacity/force is integrated once per physical substep.
        let grips = if pass + 1 == CONSTRAINT_PASSES {
            &mut self.grips[..]
        } else {
            &mut []
        };
        solver.project_constraints(particles, interval, grips, &mut self.multipliers)
    }

    fn commit(&mut self, grips: &mut [ClothGrip]) {
        for grip in &mut self.grips {
            grip.finish();
        }
        grips.clone_from_slice(&self.grips);
    }
}

pub struct ClothSolver {
    surface: DeformedSurface,
    positions: Vec<DVec3>,
    velocities: Vec<DVec3>,
    inverse_mass: Vec<f64>,
    triangles: Vec<[usize; 3]>,
    self_collision: self_collision::SelfCollisionTopology,
    springs: Vec<DistanceSpring>,
    parameters: ClothParameters,
}

impl ClothSolver {
    /// Rest positions set the material lengths and mass distribution. Initial
    /// positions may be a previously observed drape, with zero starting velocity.
    pub fn new(
        rest: Vec<Vec3>,
        initial: Vec<Vec3>,
        triangles: Vec<[u32; 3]>,
        uv: Vec<Vec2>,
        parameters: ClothParameters,
    ) -> Result<Self, ClothError> {
        parameters.validate()?;
        DeformedSurface::new(
            rest.clone(),
            triangles.clone(),
            uv.clone(),
            parameters.thickness_m,
        )?;
        if initial.len() != rest.len() {
            return Err(ClothError::InvalidInput);
        }
        let surface = DeformedSurface::new(
            initial.clone(),
            triangles.clone(),
            uv,
            parameters.thickness_m,
        )?;
        let rest: Vec<_> = rest.into_iter().map(|v| v.as_dvec3()).collect();
        let inverse_mass = area_masses(&rest, &triangles, parameters.total_mass_kg)?
            .into_iter()
            .map(|mass| 1.0 / mass)
            .collect();
        let springs = material_springs(&rest, &triangles, parameters)?;
        let velocities = vec![DVec3::ZERO; initial.len()];
        let triangles: Vec<_> = triangles.iter().map(|t| t.map(|i| i as usize)).collect();
        let self_collision = self_collision::SelfCollisionTopology::new(&triangles);
        Ok(Self {
            surface,
            positions: initial.into_iter().map(|v| v.as_dvec3()).collect(),
            velocities,
            inverse_mass,
            triangles,
            self_collision,
            springs,
            parameters,
        })
    }

    pub fn surface(&self) -> &DeformedSurface {
        &self.surface
    }

    pub fn max_speed(&self) -> f64 {
        self.velocities
            .iter()
            .map(|v| v.length())
            .fold(0.0, f64::max)
    }

    pub fn max_edge_stretch_ratio(&self) -> f64 {
        self.springs
            .iter()
            .filter(|s| s.stretch)
            .map(|s| self.positions[s.vertices[0]].distance(self.positions[s.vertices[1]]) / s.rest)
            .fold(0.0, f64::max)
    }

    pub fn max_particle_penetration(&self, colliders: &[FixedClothCollider]) -> f64 {
        let radius = self.parameters.thickness_m as f64 * 0.5;
        self.positions
            .iter()
            .flat_map(|p| {
                colliders
                    .iter()
                    .filter(move |c| c.intersects_bounds(*p, *p, radius))
                    .map(move |c| (radius - c.distance_and_normal(*p).0).max(0.0))
            })
            .fold(0.0, f64::max)
    }

    /// Advance only by explicit simulation time, never wall-clock time. Invalid
    /// or failed steps leave all state and the published surface revision intact.
    /// A caller must stop/report an error, not silently replay a frozen success.
    pub fn step(
        &mut self,
        dt: f64,
        gravity: DVec3,
        colliders: &[FixedClothCollider],
    ) -> Result<(), ClothError> {
        self.step_with_grips(dt, gravity, colliders, &mut [])
    }

    fn project_constraints(
        &self,
        particles: &mut GripParticles<'_>,
        interval: [f64; 2],
        grips: &mut [ClothGrip],
        multipliers: &mut [f64],
    ) -> Result<(), ClothError> {
        for (spring, multiplier) in self.springs.iter().zip(multipliers) {
            spring.project(particles, multiplier);
        }
        for grip in grips {
            grip.project(particles, interval)?;
        }
        Ok(())
    }

    /// Coupling accepts acquired material contacts only, never task targets.
    /// Both cloth and clamp state commit together after a successful step.
    pub(crate) fn step_with_grips(
        &mut self,
        dt: f64,
        gravity: DVec3,
        colliders: &[FixedClothCollider],
        grips: &mut [ClothGrip],
    ) -> Result<(), ClothError> {
        if !dt.is_finite() || !(0.0..=1.0 / 30.0).contains(&dt) || dt == 0.0 || !gravity.is_finite()
        {
            return Err(ClothError::InvalidInput);
        }
        let h = dt / self.parameters.substeps as f64;
        let drag = (-self.parameters.air_drag_per_second * h).exp();
        let radius = self.parameters.thickness_m as f64 * 0.5;
        let mut positions = self.positions.clone();
        let mut velocities = self.velocities.clone();
        let mut previous = positions.clone();
        let mut constraints = PendingConstraints::new(self, grips)?;
        for substep in 0..self.parameters.substeps {
            previous.copy_from_slice(&positions);
            constraints.multipliers.fill(0.);
            for (p, v) in positions.iter_mut().zip(&mut velocities) {
                *v = *v * drag + gravity * h;
                *p += *v * h;
            }
            let interval =
                [substep as f64, (substep + 1) as f64].map(|s| s / self.parameters.substeps as f64);
            let mut particles = GripParticles {
                positions: &mut positions,
                previous: &previous,
                inverse_mass: &self.inverse_mass,
                h,
            };
            for pass in 0..CONSTRAINT_PASSES {
                constraints.project(self, &mut particles, interval, pass)?;
            }
            self.self_collision
                .resolve(&mut particles, &self.triangles, colliders, radius * 2.)?;
            reconstruct_velocities(&positions, &previous, &mut velocities, h)?;
        }
        self.surface
            .update_positions(positions.iter().map(|v| v.as_vec3()).collect())?;
        self.positions = positions;
        self.velocities = velocities;
        constraints.commit(grips);
        Ok(())
    }
}

fn reconstruct_velocities(
    positions: &[DVec3],
    previous: &[DVec3],
    velocities: &mut [DVec3],
    h: f64,
) -> Result<(), ClothError> {
    for ((p, old), v) in positions.iter().zip(previous).zip(velocities) {
        *v = (*p - *old) / h;
        if !p.is_finite() || !v.is_finite() {
            return Err(ClothError::NonfiniteState);
        }
    }
    Ok(())
}

fn collide_particles(
    positions: &mut [DVec3],
    previous: &[DVec3],
    colliders: &[FixedClothCollider],
    radius: f64,
) {
    let (mut min, mut max) = positions.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    for collider in colliders {
        if collider.intersects_bounds(min, max, radius) {
            (min, max) = collider.project_particles(positions, previous, radius);
        }
    }
}

#[path = "cloth_collision.rs"]
mod collision;

#[path = "cloth_sweep.rs"]
mod sweep;

#[path = "cloth_rigid_impact.rs"]
mod rigid_impact;

#[path = "cloth_pair.rs"]
mod pair;

#[path = "cloth_self_collision.rs"]
mod self_collision;

fn area_masses(rest: &[DVec3], triangles: &[[u32; 3]], mass: f64) -> Result<Vec<f64>, ClothError> {
    let mut weights = vec![0.0; rest.len()];
    for &[a, b, c] in triangles {
        let area = (rest[b as usize] - rest[a as usize])
            .cross(rest[c as usize] - rest[a as usize])
            .length()
            * 0.5;
        for i in [a, b, c] {
            weights[i as usize] += area / 3.0;
        }
    }
    let total: f64 = weights.iter().sum();
    if !total.is_finite() || weights.iter().any(|w| *w <= 0.0) {
        return Err(ClothError::InvalidInput);
    }
    let masses: Vec<_> = weights.into_iter().map(|w| mass * w / total).collect();
    if masses
        .iter()
        .any(|m| !m.is_finite() || *m <= 0.0 || !(1.0 / m).is_finite())
    {
        return Err(ClothError::InvalidInput);
    }
    Ok(masses)
}

fn material_springs(
    rest: &[DVec3],
    triangles: &[[u32; 3]],
    p: ClothParameters,
) -> Result<Vec<DistanceSpring>, ClothError> {
    let mut edges: BTreeMap<[usize; 2], Vec<usize>> = BTreeMap::new();
    for t in triangles {
        for k in 0..3 {
            let mut edge = [t[k] as usize, t[(k + 1) % 3] as usize];
            edge.sort();
            edges.entry(edge).or_default().push(t[(k + 2) % 3] as usize);
        }
    }
    let mut springs = Vec::new();
    for (edge, opposite) in edges {
        if opposite.len() > 2 || (opposite.len() == 2 && opposite[0] == opposite[1]) {
            return Err(ClothError::InvalidInput);
        }
        springs.push(DistanceSpring {
            vertices: edge,
            rest: rest[edge[0]].distance(rest[edge[1]]),
            compliance: p.stretch_compliance,
            stretch: true,
            damping_ratio: p.spring_damping_ratio,
        });
        if opposite.len() == 2 {
            springs.push(DistanceSpring {
                vertices: [opposite[0], opposite[1]],
                rest: rest[opposite[0]].distance(rest[opposite[1]]),
                compliance: p.bend_compliance,
                stretch: false,
                damping_ratio: p.spring_damping_ratio,
            });
        }
    }
    Ok(springs)
}

#[cfg(test)]
#[path = "cloth_solver_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cloth_self_tests.rs"]
mod self_tests;

#[path = "cloth_batch.rs"]
mod batch;
pub(crate) use batch::{ClothBatch, ClothLayerStep};
