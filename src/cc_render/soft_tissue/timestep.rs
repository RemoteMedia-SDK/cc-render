//! Variable-step BDF2 of both position and velocity. Startup uses backward
//! Euler; a step growth above 2 also restarts derivative history, not state.
//! Fractional render solves and rejected trials own cloned history.
use super::*;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeIntegration {
    #[default]
    ImplicitEuler,
    Bdf2,
}

#[derive(Clone)]
pub(super) struct History {
    pub positions: Vec<DVec3>,
    pub velocities: Vec<DVec3>,
    pub dt: f64,
}

pub(super) struct EdgeDamping {
    pub nodes: [usize; 2],
    pub target_length: f64,
    pub stiffness: f64,
}

impl EdgeDamping {
    pub fn energy(&self, x: &[DVec3]) -> f64 {
        0.5 * self.stiffness
            * (x[self.nodes[0]].distance(x[self.nodes[1]]) - self.target_length).powi(2)
    }
}

pub(super) struct IntegrationStep {
    pub dt: f64,
    pub baseline: Vec<DVec3>,
    pub predicted: Vec<DVec3>,
    pub damping: Dissipation,
}

#[derive(Default)]
pub(super) struct Dissipation {
    pub edges: Vec<EdgeDamping>,
    pub elements: Vec<viscosity::Element>,
}

impl Dissipation {
    pub fn energy(&self, x: &[DVec3], tets: &[Tet]) -> f64 {
        self.edges.iter().map(|e| e.energy(x)).sum::<f64>()
            + self
                .elements
                .iter()
                .map(|e| e.energy(&tets[e.tet], x))
                .sum::<f64>()
    }
}

impl IntegrationStep {
    pub fn new(tissue: &Tissue, interval: f64, gravity: DVec3) -> Self {
        let bdf2 = tissue.material.time_integration == TimeIntegration::Bdf2;
        let previous = tissue
            .previous
            .as_ref()
            .filter(|old| bdf2 && interval / old.dt <= 2.0 + 1e-12);
        let (dt, current, older) = match previous {
            Some(old) => {
                let r = interval / old.dt;
                let a0 = (1.0 + 2.0 * r) / (1.0 + r);
                (interval / a0, (1.0 + r) / a0, -r * r / ((1.0 + r) * a0))
            }
            None => (interval, 1.0, 0.0),
        };
        let baseline: Vec<_> = tissue
            .positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                *p * current + previous.map_or(DVec3::ZERO, |old| old.positions[i] * older)
            })
            .collect();
        let predicted = baseline
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let velocity = tissue.velocities[i] * current
                    + previous.map_or(DVec3::ZERO, |old| old.velocities[i] * older);
                *p + dt * velocity + dt * dt * gravity
            })
            .collect();
        let edges = if bdf2 && tissue.material.strain_damping_per_second > 0.0 {
            tissue
                .edges
                .iter()
                .filter_map(|&[a, b]| {
                    let inverse_mass = tissue.inverse_mass[a] + tissue.inverse_mass[b];
                    if inverse_mass == 0.0 {
                        return None;
                    }
                    let length = current * tissue.positions[a].distance(tissue.positions[b])
                        + previous.map_or(0.0, |old| {
                            older * old.positions[a].distance(old.positions[b])
                        });
                    Some(EdgeDamping {
                        nodes: [a, b],
                        target_length: length,
                        stiffness: tissue.material.strain_damping_per_second
                            / (tissue.damping_degree * inverse_mass * dt),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let elements = tissue
            .material
            .viscosity
            .map_or_else(Vec::new, |viscosity| {
                tissue
                    .tets
                    .iter()
                    .enumerate()
                    .map(|(i, tet)| viscosity::Element {
                        tet: i,
                        history_strain: current
                            * viscosity::isochoric_green(tet.deformation(&tissue.positions))
                            + previous.map_or(bevy::math::DMat3::ZERO, |old| {
                                older * viscosity::isochoric_green(tet.deformation(&old.positions))
                            }),
                        history_log_volume: current
                            * tet.deformation(&tissue.positions).determinant().ln()
                            + previous.map_or(0., |old| {
                                older * tet.deformation(&old.positions).determinant().ln()
                            }),
                        shear: viscosity.shear_pa_s / dt,
                        bulk: viscosity.bulk_pa_s / dt,
                    })
                    .collect()
            });
        let damping = Dissipation { edges, elements };
        Self {
            dt,
            baseline,
            predicted,
            damping,
        }
    }
}
