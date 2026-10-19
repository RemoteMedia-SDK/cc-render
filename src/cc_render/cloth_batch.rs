//! One transactional physics tick across independently authored cloth layers.
use crate::cc_render::cloth_grip::ClampState;
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
/// Sleep: for `REST_SECONDS` of simulated time, every particle stays within
/// `REST_DRIFT_M` of where the quiet window began, and moves less than
/// `REST_SPEED_M_S` times the tick between consecutive published ticks. The
/// coupled layers rest together.
///
/// Motion is measured between published tick positions, not with the last
/// substep's velocity: where stacked layers press on the mattress's rounded
/// corners, material and contact projections exchange ~0.1-0.6 mm every
/// substep, which shows up as 0.1-0.8 m/s of substep velocity while the
/// drape itself moves well under a millimetre per tick. Resting zeroes those
/// velocities; it never moves a particle or skips a failure.
///
/// Settle: once something has moved the cloth from outside (see `disturbed`),
/// the batch also sleeps `SETTLE_SECONDS` after the last such movement, as
/// soon as nothing moves faster than `SETTLE_SPEED_M_S`, even if it has not
/// met the quiet window. Stacked covers kept awake keep creeping slowly
/// (millimetres to centimetres per second where they hang over sharp edges);
/// this ends the simulation of a disturbance instead of the creep. A cloth
/// still visibly falling or sliding is never frozen. The authored drape,
/// never disturbed, rests only by the quiet window.
///
/// Wake: anything that disturbs it, or a layer surface published outside
/// this batch. A grip held still, or a body collider lying still, does not
/// keep it awake.
pub(super) struct Rest {
    pub(super) asleep: bool,
    /// Adopt the first tick's inputs as a rest state instead of simulating.
    pub(super) adopt_on_first_tick: bool,
    quiet_seconds: f64,
    anchor: Vec<DVec3>,
    pub(super) inputs: u64,
    pub(super) revisions: Vec<u64>,
    /// Simulated seconds since the last disturbance (None: never disturbed).
    since_disturbed: Option<f64>,
    last_inputs: Option<u64>,
    last_grips: Vec<ClampState>,
    last_bodies: u64,
}

const REST_SECONDS: f64 = 0.5;
const REST_DRIFT_M: f64 = 5e-3;
const REST_SPEED_M_S: f64 = 0.05;
const SETTLE_SECONDS: f64 = 1.5;
const SETTLE_SPEED_M_S: f64 = 0.3;
/// A held grip's hand moving less than this over a tick holds still.
const GRIP_STILL_M: f64 = 1e-5;

impl Rest {
    pub(super) fn new() -> Self {
        Self {
            asleep: false,
            adopt_on_first_tick: false,
            quiet_seconds: 0.,
            anchor: Vec::new(),
            inputs: 0,
            revisions: Vec::new(),
            since_disturbed: None,
            last_inputs: None,
            last_grips: Vec::new(),
            last_bodies: 0,
        }
    }

    /// Whether something outside the cloth moves it this tick: a held grip's
    /// hand moving, a grip taken or let go, the body colliders (`bodies`, a key
    /// of their shapes and poses; 0 for none) moving or changing, or a
    /// different gravity or fixed-collider snapshot. Call once per tick.
    pub(super) fn disturbed(&mut self, layers: &[ClothLayerStep<'_>], inputs: u64, bodies: u64) -> bool {
        let grips: Vec<ClampState> = layers.iter().flat_map(|l| l.grips.iter().map(|g| g.state())).collect();
        let hand_moved = layers.iter().flat_map(|l| l.grips.iter()).any(|g| {
            g.state() == ClampState::Held && g.anchor(0.).distance(g.anchor(1.)) > GRIP_STILL_M
        });
        let changed = grips != self.last_grips
            || bodies != self.last_bodies
            || self.last_inputs.is_some_and(|last| last != inputs);
        self.last_grips = grips;
        self.last_bodies = bodies;
        self.last_inputs = Some(inputs);
        hand_moved || changed
    }

    pub(super) fn inputs(layers: &[ClothLayerStep<'_>], gravity: DVec3) -> u64 {
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
    pub(super) fn wake(&mut self) {
        if self.asleep {
            self.asleep = false;
            self.quiet_seconds = 0.;
            self.anchor.clear();
        }
    }

    pub(super) fn holds(&self, layers: &[ClothLayerStep<'_>], inputs: u64, disturbed: bool) -> bool {
        self.asleep
            && !disturbed
            && inputs == self.inputs
            && layers
                .iter()
                .map(|l| l.solver.surface.revision())
                .eq(self.revisions.iter().copied())
    }

    pub(super) fn observe(
        &mut self,
        layers: &mut [ClothLayerStep<'_>],
        tick_start: &[DVec3],
        dt: f64,
        inputs: u64,
        disturbed: bool,
    ) {
        self.asleep = false;
        self.since_disturbed = if disturbed { Some(0.) } else { self.since_disturbed.map(|t| t + dt) };
        let positions = layers.iter().flat_map(|l| l.solver.positions.iter());
        let settled = self.since_disturbed.is_some_and(|t| t + 1e-9 >= SETTLE_SECONDS)
            && positions
                .clone()
                .zip(tick_start)
                .all(|(p, a)| p.distance_squared(*a) <= (SETTLE_SPEED_M_S * dt).powi(2));
        let quiet = self.anchor.len()
            == layers
                .iter()
                .map(|l| l.solver.positions.len())
                .sum::<usize>()
            && !disturbed
            && positions
                .clone()
                .zip(&self.anchor)
                .all(|(p, a)| p.distance_squared(*a) <= REST_DRIFT_M * REST_DRIFT_M)
            && positions
                .clone()
                .zip(tick_start)
                .all(|(p, a)| p.distance_squared(*a) <= (REST_SPEED_M_S * dt).powi(2));
        if !quiet && !settled {
            self.anchor = positions.copied().collect();
            self.quiet_seconds = 0.;
            return;
        }
        self.quiet_seconds += dt;
        if settled || self.quiet_seconds + 1e-9 >= REST_SECONDS {
            self.since_disturbed = None;
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

    /// Start asleep: the layers' initial positions are a drape this solver
    /// already brought to rest (`SceneCloth::starts_at_rest`).
    pub(crate) fn start_at_rest(&mut self) {
        self.rest.adopt_on_first_tick = true;
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
        let disturbed = self.rest.disturbed(layers, inputs, 0);
        if std::mem::take(&mut self.rest.adopt_on_first_tick) {
            // The baked drape already rested under these inputs: record them
            // and sleep. Any later change to them, a grip or an external
            // surface update wakes the batch as usual.
            for layer in layers.iter_mut() {
                layer.solver.velocities.fill(DVec3::ZERO);
            }
            self.rest.asleep = true;
            self.rest.inputs = inputs;
            self.rest.revisions = layers.iter().map(|l| l.solver.surface.revision()).collect();
        }
        if self.rest.holds(layers, inputs, disturbed) {
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
        let tick_start = positions.clone();
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
                friction: layer.solver.parameters.friction(),
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
            for (layer, layout) in layers.iter().zip(&self.layout) {
                let r = layout.range.clone();
                damp_edge_velocities(
                    &layer.solver.springs,
                    &inverse_mass[r.clone()],
                    &mut velocities[r],
                    h,
                );
            }
        }
        profiled!(
            "commit",
            self.commit_layers(layers, &positions, &velocities, &mut constraints)
        )?;
        self.rest.observe(layers, &tick_start, dt, inputs, disturbed);
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
