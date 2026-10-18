use super::material::{self, Tet};
use super::{contact, ContactMetrics, PlaneContact};
use super::{pressure, PressureDiscretization};
use bevy::math::{DQuat, DVec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[path = "implicit.rs"]
mod implicit;
#[path = "elasticity.rs"]
mod elasticity;
#[path = "preconditioner.rs"]
mod preconditioner;
#[path = "timestep.rs"]
mod timestep;
pub use timestep::TimeIntegration;
use timestep::{History, IntegrationStep};
#[path = "viscosity.rs"]
mod viscosity;
pub use viscosity::Viscosity;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    #[serde(default)]
    pub pressure_discretization: PressureDiscretization,
    /// In boundary-nodal mode, use local pressure below this authored-shape
    /// mean-ratio quality. Zero disables geometry selection. The partition is
    /// fixed at construction, independent of motion and reference-shape fitting.
    #[serde(default)]
    pub element_pressure_quality_cutoff: f64,
    /// Optional element-volume share of averaged pressure, in Pa. This
    /// penalizes within-patch volume variation while preserving affine bulk
    /// energy. Zero retains pure averaging; only valid for nodal pressure.
    #[serde(default)]
    pub pressure_stabilization_pa: f64,
    #[serde(default)]
    pub time_integration: TimeIntegration,
    /// Objective material strain-rate damping, in Pa s. Mutually exclusive
    /// with the legacy topology-dependent edge damping rate.
    #[serde(default)]
    pub viscosity: Option<Viscosity>,
    pub density_kg_m3: f64,
    pub shear_pa: f64,
    /// Dimensionless quartic corotated-strain coefficient. Zero retains the
    /// original material; positive values increase resistance at large strain.
    /// This is a phenomenological material parameter, not a displacement cap.
    #[serde(default)]
    pub strain_stiffening: f64,
    pub bulk_pa: f64,
    /// Dissipates relative edge extension, never absolute world velocity.
    /// This is discrete strain-rate damping, not a measured tissue viscosity.
    pub strain_damping_per_second: f64,
    pub iterations: usize,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "contact_tests.rs"]
mod contact_tests;

#[cfg(test)]
#[path = "integration_tests.rs"]
mod integration_tests;

impl Material {
    pub(super) fn nodal_bulk(self) -> f64 {
        self.bulk_pa - self.pressure_stabilization_pa
    }

    pub(super) fn lambda(self) -> f64 {
        self.bulk_pa - 2.0 * self.shear_pa / 3.0
    }

    fn validate(self) -> Result<(), TissueError> {
        if [self.density_kg_m3, self.shear_pa, self.bulk_pa]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.lambda() <= 0.0
            || !self.strain_stiffening.is_finite()
            || self.strain_stiffening < 0.0
            || !self.strain_damping_per_second.is_finite()
            || self.strain_damping_per_second < 0.0
            || !(1..=64).contains(&self.iterations)
        {
            return Err(TissueError::InvalidInput("material"));
        }
        if !self.pressure_stabilization_pa.is_finite()
            || !(0.0..=self.bulk_pa).contains(&self.pressure_stabilization_pa)
            || (self.pressure_discretization == PressureDiscretization::Element
                && self.pressure_stabilization_pa != 0.0)
        {
            return Err(TissueError::InvalidInput("pressure stabilization"));
        }
        if !self.element_pressure_quality_cutoff.is_finite()
            || !(0.0..=1.0).contains(&self.element_pressure_quality_cutoff)
            || (self.pressure_discretization != PressureDiscretization::AverageNodalBoundary
                && self.element_pressure_quality_cutoff != 0.0)
        {
            return Err(TissueError::InvalidInput("element pressure quality cutoff"));
        }
        if let Some(viscosity) = self.viscosity {
            viscosity.validate()?;
            if self.strain_damping_per_second != 0.0 {
                return Err(TissueError::InvalidInput("both continuum and edge damping requested"));
            }
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
    #[error("tissue linear solve did not converge (relative residual {0}); state was not committed")]
    LinearSolveFailed(f64),
    #[error("tissue nonlinear solve did not converge (force {force_norm_n} N, correction {correction_m} m); state was not committed")]
    NonlinearSolveFailed { force_norm_n: f64, correction_m: f64 },
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
    damping_edges: Vec<[usize; 2]>,
    pressure_patches: Vec<pressure::Patch>,
    element_pressure: Vec<bool>,
    factor_pattern: std::sync::Arc<preconditioner::Pattern>,
    damping_degree: f64,
    masses: Vec<f64>,
    inverse_mass: Vec<f64>,
    positions: Vec<DVec3>,
    velocities: Vec<DVec3>,
    frame: Frame,
    previous: Option<History>,
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
                edge_indices: [usize::MAX; 6],
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
        let mut tets = tetrahedra(&rest, &topology)?;
        let mut masses = vec![0.0; rest.len()];
        for tet in &tets {
            for i in tet.ids {
                masses[i] += material.density_kg_m3 * tet.volume / 4.0;
            }
        }
        if masses.iter().any(|m| *m <= 0.0) {
            return Err(TissueError::InvalidInput("unreferenced particle"));
        }
        let inverse_mass: Vec<_> = masses
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
        let (damping_edges, damping_degree) = material_edges(&tets, rest.len());
        let element_pressure = pressure::element_partition(&tets, &initial, &attachments, material);
        let mut pressure_patches = match material.pressure_discretization {
            PressureDiscretization::Element => Vec::new(),
            _ => pressure::Patch::build(&tets, rest.len(), &element_pressure),
        };
        let mut edges: BTreeSet<_> = damping_edges.iter().copied().collect();
        for patch in &pressure_patches { patch.extend_edges(&mut edges); }
        let edges: Vec<_> = edges.into_iter().collect();
        for patch in &mut pressure_patches { patch.index_edges(&edges)?; }
        let factor_pattern = std::sync::Arc::new(preconditioner::Pattern::new(
            rest.len(),
            edges.iter().copied().filter(|edge| edge.iter().all(|i| inverse_mass[*i] > 0.0)),
        ));
        for tet in &mut tets {
            tet.edge_indices = material::EDGE_PAIRS.map(|[a, b]| {
                let mut nodes = [tet.ids[a], tet.ids[b]];
                nodes.sort_unstable();
                edges.binary_search(&nodes).expect("tet edge is indexed")
            });
        }
        let positions = initial.iter().map(|p| frame.point(*p)).collect();
        let result = Self {
            velocities: vec![DVec3::ZERO; rest.len()],
            rest,
            attachment_positions: initial,
            attachments,
            material,
            tets,
            edges,
            damping_edges,
            pressure_patches,
            element_pressure,
            factor_pattern,
            damping_degree,
            masses,
            inverse_mass,
            positions,
            frame,
            previous: None,
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

    /// Initial rigid velocity at an explicit playback/session start. This is
    /// not a per-frame correction and is never used for ordinary acceleration.
    pub fn initialize_motion(&mut self, next_frame: Frame, dt: f64) -> Result<(), TissueError> {
        next_frame.validate()?;
        if !dt.is_finite() || dt <= 0.0 {
            return Err(TissueError::InvalidInput("initial motion interval"));
        }
        for (velocity, point) in self.velocities.iter_mut().zip(&self.attachment_positions) {
            *velocity = (next_frame.point(*point) - self.frame.point(*point)) / dt;
        }
        self.previous = None;
        Ok(())
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
            elastic_energy_j: self.elastic_energy(&self.positions),
            maximum_speed_m_s: self
                .velocities
                .iter()
                .map(|v| v.length())
                .fold(0.0, f64::max),
        }
    }

    fn elastic_energy(&self, x: &[DVec3]) -> f64 {
        self.tets.iter().enumerate().map(|(i, t)| match self.material.pressure_discretization {
            PressureDiscretization::Element => t.energy(x, self.material.shear_pa,
                self.material.lambda(), self.material.strain_stiffening),
            _ => t.isochoric_energy(x, self.material.shear_pa, self.material.strain_stiffening)
                + if self.element_bulk(i) > 0. { 0.5 * self.element_bulk(i) * t.volume
                    * (t.deformation(x).determinant() - 1.).powi(2) } else { 0. },
        }).sum::<f64>()
            + self.pressure_patches.iter().map(|p| p.energy(&self.tets, x,
                self.material.nodal_bulk(), pressure::Damping::default())).sum::<f64>()
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

    fn place_fixed_attachments(&self, positions: &mut [DVec3], frame: Frame) {
        for (i, attachment) in self.attachments.iter().enumerate() {
            if matches!(attachment, Attachment::Fixed) {
                positions[i] = frame.point(self.attachment_positions[i]);
            }
        }
    }

    fn damp_strain(&self, positions: &[DVec3], velocities: &mut [DVec3], dt: f64) {
        let fraction =
            1.0 - (-self.material.strain_damping_per_second * dt / self.damping_degree).exp();
        for [a, b] in &self.damping_edges {
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
        self.step_with_contacts(dt, frame, gravity, &[]).map(|_| ())
    }

    /// Integrate unilateral normal contact and elasticity in one solve.
    /// Contact planes must come from calibrated, substep-sampled geometry.
    /// This does not generate collision witnesses or apply body-pose corrections.
    pub fn step_with_contacts(
        &mut self,
        dt: f64,
        frame: Frame,
        gravity: DVec3,
        contacts: &[PlaneContact],
    ) -> Result<ContactMetrics, TissueError> {
        frame.validate()?;
        if !dt.is_finite() || dt <= 0.0 || dt > 1.0 / 60.0 || !gravity.is_finite() {
            return Err(TissueError::InvalidInput("substep or gravity"));
        }
        if contacts.len() > 16384 {
            return Err(TissueError::InvalidInput("contact capacity"));
        }
        for contact in contacts {
            contact.validate(self.tets.len())?;
        }
        let mut next: Vec<_> = self
            .positions
            .iter()
            .zip(&self.velocities)
            .map(|(p, v)| *p + dt * *v)
            .collect();
        // Warm-start from velocity, without introducing an unbalanced gravity
        // displacement into already-supported tissue. Gravity remains in the
        // inertial target, so the implicit Euler equations are unchanged.
        let integration = IntegrationStep::new(self, dt, gravity);
        self.place_fixed_attachments(&mut next, frame);
        self.validate_positions(&next)?;
        self.solve_implicit(&mut next, &integration.predicted, frame, integration.dt, contacts, &integration.damping)?;
        self.validate_positions(&next)?;
        let mut velocities: Vec<_> = next
            .iter()
            .zip(&integration.baseline)
            .map(|(a, b)| (*a - *b) / integration.dt)
            .collect();
        if self.material.time_integration == TimeIntegration::ImplicitEuler {
            self.damp_strain(&next, &mut velocities, dt);
        }
        if velocities.iter().any(|v| !v.is_finite()) {
            return Err(TissueError::Nonfinite);
        }
        let contact_metrics = contact::metrics(contacts, &next, &self.tets)?;
        if self.material.time_integration == TimeIntegration::Bdf2 {
            self.previous = Some(History { positions: self.positions.clone(), velocities: self.velocities.clone(), dt });
        }
        self.positions = next;
        self.velocities = velocities;
        self.frame = frame;
        Ok(contact_metrics)
    }
}
