//! One transactional physics tick across independently authored cloth layers.
use super::self_collision::{ContactLayer, SelfCollisionTopology};
use super::*;
use std::ops::Range;

pub(crate) struct ClothLayerStep<'a> {
    pub solver: &'a mut ClothSolver,
    pub colliders: &'a [FixedClothCollider],
    pub grips: &'a mut [ClothGrip],
}

struct Layout {
    range: Range<usize>,
    triangles: Vec<[usize; 3]>,
}

/// A settled batch costs nothing until something can disturb it.
///
/// Sleep: every particle stays within `REST_DRIFT_M` of where a quiet window
/// began, and every end-of-tick speed stays below `REST_SPEED_M_S`, for
/// `REST_SECONDS` of simulated time. The coupled layers rest together.
/// Wake: any grip, a different gravity or fixed-collider snapshot, or a layer
/// surface published outside this batch. Resting zeroes the (already
/// sub-threshold) velocities; it never moves a particle or skips a failure.
struct Rest {
    asleep: bool,
    quiet_seconds: f64,
    anchor: Vec<DVec3>,
    inputs: u64,
    revisions: Vec<u64>,
}

const REST_SECONDS: f64 = 0.5;
const REST_DRIFT_M: f64 = 5e-3;
const REST_SPEED_M_S: f64 = 0.02;

impl Rest {
    fn new() -> Self {
        Self {
            asleep: false,
            quiet_seconds: 0.,
            anchor: Vec::new(),
            inputs: 0,
            revisions: Vec::new(),
        }
    }

    fn inputs(layers: &[ClothLayerStep<'_>], gravity: DVec3) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        gravity.to_array().map(f64::to_bits).hash(&mut hasher);
        for layer in layers {
            layer.colliders.len().hash(&mut hasher);
            for c in layer.colliders {
                c.center.to_array().map(f64::to_bits).hash(&mut hasher);
                c.rotation.to_array().map(f64::to_bits).hash(&mut hasher);
                c.half_extents
                    .to_array()
                    .map(f64::to_bits)
                    .hash(&mut hasher);
                [c.border, c.friction].map(f64::to_bits).hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// A disturbed batch needs a whole new quiet window before resting.
    fn wake(&mut self) {
        if self.asleep {
            self.asleep = false;
            self.quiet_seconds = 0.;
            self.anchor.clear();
        }
    }

    fn holds(&self, layers: &[ClothLayerStep<'_>], inputs: u64) -> bool {
        self.asleep
            && inputs == self.inputs
            && layers.iter().all(|l| l.grips.is_empty())
            && layers
                .iter()
                .map(|l| l.solver.surface.revision())
                .eq(self.revisions.iter().copied())
    }

    fn observe(&mut self, layers: &mut [ClothLayerStep<'_>], dt: f64, inputs: u64) {
        self.asleep = false;
        let positions = layers.iter().flat_map(|l| l.solver.positions.iter());
        let quiet = self.anchor.len()
            == layers
                .iter()
                .map(|l| l.solver.positions.len())
                .sum::<usize>()
            && layers.iter().all(|l| l.grips.is_empty())
            && positions
                .clone()
                .zip(&self.anchor)
                .all(|(p, a)| p.distance_squared(*a) <= REST_DRIFT_M * REST_DRIFT_M)
            && layers
                .iter()
                .flat_map(|l| l.solver.velocities.iter())
                .all(|v| v.length_squared() <= REST_SPEED_M_S * REST_SPEED_M_S);
        if !quiet {
            self.anchor = positions.copied().collect();
            self.quiet_seconds = 0.;
            return;
        }
        self.quiet_seconds += dt;
        if self.quiet_seconds + 1e-9 >= REST_SECONDS {
            for layer in layers.iter_mut() {
                layer.solver.velocities.fill(DVec3::ZERO);
            }
            self.asleep = true;
            self.inputs = inputs;
            self.revisions = layers.iter().map(|l| l.solver.surface.revision()).collect();
        }
    }
}

pub(crate) struct ClothBatch {
    layout: Vec<Layout>,
    triangles: Vec<[usize; 3]>,
    contacts: SelfCollisionTopology,
    rest: Rest,
    #[cfg(test)]
    energy_probe: energy::EnergyProbe,
}

impl ClothBatch {
    pub(crate) fn new(solvers: &[&ClothSolver]) -> Result<Self, ClothError> {
        if solvers.is_empty() {
            return Err(ClothError::InvalidInput);
        }
        let mut layout = Vec::new();
        let mut triangles = Vec::new();
        let mut offset = 0;
        for solver in solvers {
            triangles.extend(solver.triangles.iter().map(|t| t.map(|i| i + offset)));
            let end = offset + solver.positions.len();
            layout.push(Layout {
                range: offset..end,
                triangles: solver.triangles.clone(),
            });
            offset = end;
        }
        Ok(Self {
            #[cfg(test)]
            energy_probe: energy::EnergyProbe::from_environment(),
            contacts: SelfCollisionTopology::new(&triangles),
            rest: Rest::new(),
            layout,
            triangles,
        })
    }

    fn validate(
        &self,
        layers: &[ClothLayerStep<'_>],
        dt: f64,
        gravity: DVec3,
    ) -> Result<(), ClothError> {
        if !dt.is_finite()
            || !(0.0..=1. / 30.).contains(&dt)
            || dt == 0.
            || !gravity.is_finite()
            || layers.len() != self.layout.len()
        {
            return Err(ClothError::InvalidInput);
        }
        for (layer, layout) in layers.iter().zip(&self.layout) {
            if layer.solver.positions.len() != layout.range.len()
                || layer.solver.triangles != layout.triangles
            {
                return Err(ClothError::InvalidInput);
            }
        }
        Ok(())
    }

    fn project_layers(
        &self,
        layers: &[ClothLayerStep<'_>],
        particles: &mut GripParticles<'_>,
        interval: [f64; 2],
        constraints: &mut [PendingConstraints],
        pass: usize,
    ) -> Result<(), ClothError> {
        for ((layer, layout), state) in layers.iter().zip(&self.layout).zip(constraints) {
            let r = layout.range.clone();
            state.project(
                layer.solver,
                &mut GripParticles {
                    positions: &mut particles.positions[r.clone()],
                    previous: &particles.previous[r.clone()],
                    inverse_mass: &particles.inverse_mass[r],
                    h: particles.h,
                },
                interval,
                pass,
            )?;
        }
        Ok(())
    }

    fn commit_layers(
        &self,
        layers: &mut [ClothLayerStep<'_>],
        positions: &[DVec3],
        velocities: &[DVec3],
        constraints: &mut [PendingConstraints],
    ) -> Result<(), ClothError> {
        // Validate every render surface and revision before publishing any one.
        let surfaces = layers
            .iter()
            .zip(&self.layout)
            .map(|(layer, layout)| {
                layer.solver.surface.updated(
                    positions[layout.range.clone()]
                        .iter()
                        .map(|p| p.as_vec3())
                        .collect(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (((layer, layout), surface), pending) in layers
            .iter_mut()
            .zip(&self.layout)
            .zip(surfaces)
            .zip(constraints)
        {
            layer
                .solver
                .positions
                .copy_from_slice(&positions[layout.range.clone()]);
            layer
                .solver
                .velocities
                .copy_from_slice(&velocities[layout.range.clone()]);
            layer.solver.surface = surface;
            pending.commit(layer.grips);
        }
        Ok(())
    }

    /// The batch is at rest and its last published surfaces stand.
    #[cfg(test)]
    pub(crate) fn resting(&self) -> bool {
        self.rest.asleep
    }

    pub(crate) fn step(
        &mut self,
        layers: &mut [ClothLayerStep<'_>],
        dt: f64,
        gravity: DVec3,
    ) -> Result<(), ClothError> {
        self.validate(layers, dt, gravity)?;
        let inputs = Rest::inputs(layers, gravity);
        if self.rest.holds(layers, inputs) {
            #[cfg(test)]
            crate::cc_render::cloth_profile::count("resting_ticks", 1);
            return Ok(());
        }
        self.rest.wake();
        // Every material gets at least its requested integration resolution.
        let substeps = layers
            .iter()
            .map(|l| l.solver.parameters.substeps)
            .max()
            .unwrap();
        let h = dt / substeps as f64;
        #[cfg(test)]
        self.energy_probe.begin_tick(dt, substeps, gravity);
        let mut positions: Vec<_> = layers
            .iter()
            .flat_map(|l| l.solver.positions.iter().copied())
            .collect();
        let mut velocities: Vec<_> = layers
            .iter()
            .flat_map(|l| l.solver.velocities.iter().copied())
            .collect();
        let inverse_mass: Vec<_> = layers
            .iter()
            .flat_map(|l| l.solver.inverse_mass.iter().copied())
            .collect();
        let mut previous = positions.clone();
        let mut constraints = layers
            .iter()
            .map(|layer| PendingConstraints::new(layer.solver, layer.grips))
            .collect::<Result<Vec<_>, _>>()?;
        let contacts: Vec<_> = layers
            .iter()
            .zip(&self.layout)
            .map(|(layer, layout)| ContactLayer {
                range: layout.range.clone(),
                triangles: &layer.solver.triangles,
                radius: layer.solver.parameters.thickness_m as f64 * 0.5,
                fixed: layer.colliders,
            })
            .collect();
        for substep in 0..substeps {
            #[cfg(test)]
            self.energy_probe
                .record(("before", substep, None), layers, &positions, |i| {
                    velocities[i]
                });
            previous.copy_from_slice(&positions);
            #[cfg(test)]
            crate::cc_render::cloth_profile::stage("start", substep, &positions);
            for state in &mut constraints {
                state.multipliers.fill(0.);
            }
            let interval = [substep as f64, (substep + 1) as f64].map(|s| s / substeps as f64);
            for (layer, layout) in layers.iter().zip(&self.layout) {
                let r = layout.range.clone();
                let drag = (-layer.solver.parameters.air_drag_per_second * h).exp();
                for (p, v) in positions[r.clone()]
                    .iter_mut()
                    .zip(&mut velocities[r.clone()])
                {
                    *v = *v * drag + gravity * h;
                    *p += *v * h;
                }
            }
            #[cfg(test)]
            crate::cc_render::cloth_profile::stage("predicted", substep, &positions);
            #[cfg(test)]
            self.energy_probe
                .record(("predicted", substep, None), layers, &positions, |i| {
                    velocities[i]
                });
            for pass in 0..CONSTRAINT_PASSES {
                profiled!(
                    "constraints",
                    self.project_layers(
                        layers,
                        &mut GripParticles {
                            positions: &mut positions,
                            previous: &previous,
                            inverse_mass: &inverse_mass,
                            h,
                        },
                        interval,
                        &mut constraints,
                        pass,
                    )
                )?;
                #[cfg(test)]
                self.energy_probe.record(
                    ("constraints", substep, Some(pass)),
                    layers,
                    &positions,
                    |i| (positions[i] - previous[i]) / h,
                );
            }
            #[cfg(test)]
            crate::cc_render::cloth_profile::stage("constraints", substep, &positions);
            profiled!(
                "contacts",
                self.contacts.resolve_layers(
                    &mut GripParticles {
                        positions: &mut positions,
                        previous: &previous,
                        inverse_mass: &inverse_mass,
                        h,
                    },
                    &self.triangles,
                    &contacts,
                )
            )?;
            #[cfg(test)]
            self.energy_probe
                .record(("contacts", substep, None), layers, &positions, |i| {
                    (positions[i] - previous[i]) / h
                });
            #[cfg(test)]
            crate::cc_render::cloth_profile::stage("contacts", substep, &positions);
            reconstruct_velocities(&positions, &previous, &mut velocities, h)?;
        }
        profiled!(
            "commit",
            self.commit_layers(layers, &positions, &velocities, &mut constraints)
        )?;
        self.rest.observe(layers, dt, inputs);
        #[cfg(test)]
        self.energy_probe.finish_tick();
        Ok(())
    }
}

#[cfg(test)]
#[path = "cloth_batch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cloth_energy_probe.rs"]
mod energy;

#[cfg(test)]
#[path = "cloth_bench_tests.rs"]
pub(crate) mod bench;
