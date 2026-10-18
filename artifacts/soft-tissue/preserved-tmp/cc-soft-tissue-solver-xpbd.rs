use super::material::{self, Tet};
use bevy::math::{DQuat, DVec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub density_kg_m3: f64,
    pub shear_pa: f64,
    pub bulk_pa: f64,
    /// Dissipates relative edge extension, never absolute world velocity.
    /// This is discrete strain-rate damping, not a measured tissue viscosity.
    pub strain_damping_per_second: f64,
    pub iterations: usize,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

impl Material {
    pub(super) fn lambda(self) -> f64 {
        self.bulk_pa - 2.0 * self.shear_pa / 3.0
    }

    fn validate(self) -> Result<(), TissueError> {
        if [self.density_kg_m3, self.shear_pa, self.bulk_pa]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.lambda() <= 0.0
            || !self.strain_damping_per_second.is_finite()
            || self.strain_damping_per_second < 0.0
            || !(1..=64).contains(&self.iterations)
        {
            return Err(TissueError::InvalidInput("material"));
        }
        Ok(())
    }
}

/// Fixed attachments are the actual chest footprint. Compliant attachments
/// model a graded attachment region or an explicit garment support approximation.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Attachment {
    Free,
    Fixed,
    Compliant { compliance_m_per_n: f64 },
}

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub translation: DVec3,
    pub rotation: DQuat,
}

impl Frame {
    pub const IDENTITY: Self = Self {
        translation: DVec3::ZERO,
        rotation: DQuat::IDENTITY,
    };
    pub fn point(self, p: DVec3) -> DVec3 {
        self.rotation * p + self.translation
    }
    pub fn interpolate(self, to: Self, alpha: f64) -> Self {
        Self {
            translation: self.translation.lerp(to.translation, alpha),
            rotation: self.rotation.slerp(to.rotation, alpha),
        }
    }
    fn validate(self) -> Result<(), TissueError> {
        if !self.translation.is_finite()
            || !self.rotation.is_finite()
            || (self.rotation.length_squared() - 1.0).abs() > 1e-8
        {
            return Err(TissueError::InvalidInput("attachment frame"));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TissueError {
    #[error("invalid tissue {0}")]
    InvalidInput(&'static str),
    #[error("tissue step inverted or collapsed tetrahedron {0}; state was not committed")]
    InvertedTet(usize),
    #[error("tissue step produced nonfinite state; state was not committed")]
    Nonfinite,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct TissueMetrics {
    pub mass_kg: f64,
    pub minimum_volume_ratio: f64,
    pub maximum_volume_ratio: f64,
    pub kinetic_energy_j: f64,
    pub elastic_energy_j: f64,
    pub maximum_speed_m_s: f64,
}

#[derive(Clone)]
pub struct Tissue {
    rest: Vec<DVec3>,
    attachment_positions: Vec<DVec3>,
    attachments: Vec<Attachment>,
    material: Material,
    pub(super) tets: Vec<Tet>,
    edges: Vec<[usize; 2]>,
    damping_degree: f64,
    masses: Vec<f64>,
    inverse_mass: Vec<f64>,
    positions: Vec<DVec3>,
    velocities: Vec<DVec3>,
    frame: Frame,
}

fn tetrahedra(rest: &[DVec3], topology: &[[usize; 4]]) -> Result<Vec<Tet>, TissueError> {
    let mut unique = BTreeSet::new();
    topology
        .iter()
        .map(|ids| {
            if ids.iter().any(|i| *i >= rest.len()) {
                return Err(TissueError::InvalidInput("tet index"));
            }
            let mut key = *ids;
            key.sort_unstable();
            if key.windows(2).any(|p| p[0] == p[1]) || !unique.insert(key) {
                return Err(TissueError::InvalidInput("duplicate tet or vertex"));
            }
            let matrix = material::edges(ids.map(|i| rest[i]));
            let volume = matrix.determinant() / 6.0;
            if !volume.is_finite() || volume <= 1e-12 {
                return Err(TissueError::InvalidInput(
                    "nonpositive or degenerate rest tet",
                ));
            }
            Ok(Tet {
                ids: *ids,
                inverse_rest: matrix.inverse(),
                volume,
            })
        })
        .collect()
}

fn material_edges(tets: &[Tet], count: usize) -> (Vec<[usize; 2]>, f64) {
    let mut edges = BTreeSet::new();
    for t in tets {
        for a in 0..4 {
            for b in a + 1..4 {
                let mut edge = [t.ids[a], t.ids[b]];
                edge.sort_unstable();
                edges.insert(edge);
            }
        }
    }
    let mut degree = vec![0; count];
    for [a, b] in &edges {
        degree[*a] += 1;
        degree[*b] += 1;
    }
    (
        edges.into_iter().collect(),
        degree.into_iter().max().unwrap_or(1) as f64,
    )
}

impl Tissue {
    /// `rest` is the constitutive reference; `initial` is the authored loaded
    /// shape and attachment location. They may differ after offline rest fitting.
    pub fn new(
        rest: Vec<DVec3>,
        initial: Vec<DVec3>,
        topology: Vec<[usize; 4]>,
        attachments: Vec<Attachment>,
        material: Material,
        frame: Frame,
    ) -> Result<Self, TissueError> {
        material.validate()?;
        frame.validate()?;
        if !(4..=4096).contains(&rest.len())
            || topology.is_empty()
            || topology.len() > 32768
            || initial.len() != rest.len()
            || attachments.len() != rest.len()
            || rest.iter().chain(&initial).any(|p| !p.is_finite())
        {
            return Err(TissueError::InvalidInput(
                "geometry dimensions or coordinates",
            ));
        }
        for a in &attachments {
            if let Attachment::Compliant { compliance_m_per_n } = a {
                if !compliance_m_per_n.is_finite() || *compliance_m_per_n <= 0.0 {
                    return Err(TissueError::InvalidInput("attachment compliance"));
                }
            }
        }
        let tets = tetrahedra(&rest, &topology)?;
        let mut masses = vec![0.0; rest.len()];
        for tet in &tets {
            for i in tet.ids {
                masses[i] += material.density_kg_m3 * tet.volume / 4.0;
            }
        }
        if masses.iter().any(|m| *m <= 0.0) {
            return Err(TissueError::InvalidInput("unreferenced particle"));
        }
        let inverse_mass = masses
            .iter()
            .zip(&attachments)
            .map(|(m, a)| {
                if matches!(a, Attachment::Fixed) {
                    0.0
                } else {
                    1.0 / m
                }
            })
            .collect();
        let (edges, damping_degree) = material_edges(&tets, rest.len());
        let positions = initial.iter().map(|p| frame.point(*p)).collect();
        let result = Self {
            velocities: vec![DVec3::ZERO; rest.len()],
            rest,
            attachment_positions: initial,
            attachments,
            material,
            tets,
            edges,
            damping_degree,
            masses,
            inverse_mass,
            positions,
            frame,
        };
        result.validate_positions(&result.positions)?;
        Ok(result)
    }

    pub fn positions(&self) -> &[DVec3] {
        &self.positions
    }
    pub fn velocities(&self) -> &[DVec3] {
        &self.velocities
    }
    pub fn rest_positions(&self) -> &[DVec3] {
        &self.rest
    }
    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn metrics(&self) -> TissueMetrics {
        let ratios = self
            .tets
            .iter()
            .map(|t| t.deformation(&self.positions).determinant());
        TissueMetrics {
            mass_kg: self.masses.iter().sum(),
            minimum_volume_ratio: ratios.clone().fold(f64::INFINITY, f64::min),
            maximum_volume_ratio: ratios.fold(f64::NEG_INFINITY, f64::max),
            kinetic_energy_j: self
                .masses
                .iter()
                .zip(&self.velocities)
                .map(|(m, v)| 0.5 * m * v.length_squared())
                .sum(),
            elastic_energy_j: self
                .tets
                .iter()
                .map(|t| {
                    t.energy(
                        &self.positions,
                        self.material.shear_pa,
                        self.material.lambda(),
                    )
                })
                .sum(),
            maximum_speed_m_s: self
                .velocities
                .iter()
                .map(|v| v.length())
                .fold(0.0, f64::max),
        }
    }

    fn validate_positions(&self, positions: &[DVec3]) -> Result<(), TissueError> {
        if positions.iter().any(|p| !p.is_finite()) {
            return Err(TissueError::Nonfinite);
        }
        for (i, tet) in self.tets.iter().enumerate() {
            if tet.deformation(positions).determinant() <= 1e-4 {
                return Err(TissueError::InvertedTet(i));
            }
        }
        Ok(())
    }

    fn attach(&self, positions: &mut [DVec3], frame: Frame, dt: f64, multipliers: &mut [DVec3]) {
        for (i, attachment) in self.attachments.iter().enumerate() {
            let target = frame.point(self.attachment_positions[i]);
            match attachment {
                Attachment::Free => {}
                Attachment::Fixed => positions[i] = target,
                Attachment::Compliant { compliance_m_per_n } => {
                    let alpha = compliance_m_per_n / (dt * dt);
                    let dl = -(positions[i] - target + alpha * multipliers[i])
                        / (self.inverse_mass[i] + alpha);
                    positions[i] += self.inverse_mass[i] * dl;
                    multipliers[i] += dl;
                }
            }
        }
    }

    fn solve_material(
        &self,
        positions: &mut [DVec3],
        dt: f64,
        multipliers: &mut [[f64; 10]],
        reverse: bool,
    ) {
        let mu = self.material.shear_pa;
        let lambda = self.material.lambda();
        for step in 0..self.tets.len() {
            let i = if reverse {
                self.tets.len() - 1 - step
            } else {
                step
            };
            let tet = &self.tets[i];
            let factor = 1.0 / (tet.volume * dt * dt);
            material::project(
                tet,
                positions,
                &self.inverse_mass,
                [factor / (2.0 * mu), factor / lambda],
                &mut multipliers[i],
            );
        }
    }

    fn damp_strain(&self, positions: &[DVec3], velocities: &mut [DVec3], dt: f64) {
        let fraction =
            1.0 - (-self.material.strain_damping_per_second * dt / self.damping_degree).exp();
        for [a, b] in &self.edges {
            let w = self.inverse_mass[*a] + self.inverse_mass[*b];
            if w == 0.0 {
                continue;
            }
            let n = (positions[*b] - positions[*a]).normalize_or_zero();
            let impulse = fraction * (velocities[*b] - velocities[*a]).dot(n) / w;
            velocities[*a] += self.inverse_mass[*a] * impulse * n;
            velocities[*b] -= self.inverse_mass[*b] * impulse * n;
        }
    }

    /// Advance one physical substep. On failure neither state nor frame changes.
    /// The adapter must resample `frame` at this substep's endpoint and supply
    /// all contact constraints in the same interval before production use.
    pub fn step(&mut self, dt: f64, frame: Frame, gravity: DVec3) -> Result<(), TissueError> {
        frame.validate()?;
        if !dt.is_finite() || dt <= 0.0 || dt > 1.0 / 60.0 || !gravity.is_finite() {
            return Err(TissueError::InvalidInput("substep or gravity"));
        }
        let mut next: Vec<_> = self
            .positions
            .iter()
            .zip(&self.velocities)
            .map(|(p, v)| *p + dt * *v + dt * dt * gravity)
            .collect();
        let mut material_multipliers = vec![[0.0; 10]; self.tets.len()];
        let mut attachment_multipliers = vec![DVec3::ZERO; self.positions.len()];
        self.attach(&mut next, frame, dt, &mut attachment_multipliers);
        self.validate_positions(&next)?;
        for pass in 0..self.material.iterations {
            self.solve_material(&mut next, dt, &mut material_multipliers, pass % 2 != 0);
            self.attach(&mut next, frame, dt, &mut attachment_multipliers);
        }
        self.validate_positions(&next)?;
        let mut velocities: Vec<_> = next
            .iter()
            .zip(&self.positions)
            .map(|(a, b)| (*a - *b) / dt)
            .collect();
        self.damp_strain(&next, &mut velocities, dt);
        if velocities.iter().any(|v| !v.is_finite()) {
            return Err(TissueError::Nonfinite);
        }
        self.positions = next;
        self.velocities = velocities;
        self.frame = frame;
        Ok(())
    }
}
