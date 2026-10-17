//! Native Rapier substep boundaries for moving-cloth timing.
//! Observations use physics seconds, never displayed animation timestamps.
//! These are endpoints, not a certificate of the path inside Rapier's CCD solve.
use bevy::{ecs::system::SystemParam, prelude::*};
use bevy_rapier3d::rapier::{dynamics::RigidBodyHandle, math::Isometry, na::Vector3};
use bevy_rapier3d::{plugin::RapierTransformPropagateSet, prelude::*};
use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct BodyBoundary {
    pub handle: RigidBodyHandle,
    pub pose: Isometry<f32>,
    pub linear_velocity: Vector3<f32>,
    pub angular_velocity: Vector3<f32>,
    pub ccd_enabled: bool,
}

fn sample(context: &RapierContext) -> Vec<BodyBoundary> {
    context
        .bodies
        .iter()
        .map(|(handle, body)| BodyBoundary {
            handle,
            pose: *body.position(),
            linear_velocity: *body.linvel(),
            angular_velocity: *body.angvel(),
            ccd_enabled: body.is_ccd_enabled(),
        })
        .collect()
}

#[derive(Debug)]
pub(crate) struct PhysicsInterval {
    pub sequence: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub before: Vec<BodyBoundary>,
    pub after: Vec<BodyBoundary>,
}

#[derive(Default)]
pub(crate) struct ContextSubsteps {
    pub sequence: u64,
    pub elapsed_seconds: f64,
    /// Only the current render update; consumers must not silently skip intervals.
    pub intervals: Vec<PhysicsInterval>,
    pub error: Option<String>,
}

impl ContextSubsteps {
    fn record(&mut self, dt: f32, before: Vec<BodyBoundary>, context: &RapierContext) {
        let start_seconds = self.elapsed_seconds;
        self.elapsed_seconds += f64::from(dt);
        self.sequence += 1;
        self.intervals.push(PhysicsInterval {
            sequence: self.sequence,
            start_seconds,
            end_seconds: self.elapsed_seconds,
            before,
            after: sample(context),
        });
    }
}

#[derive(Resource, Default)]
pub(crate) struct PhysicsSubsteps(pub HashMap<Entity, ContextSubsteps>);

struct StepPlan {
    outer_dt: f32,
    substep_dt: f32,
    substeps: usize,
    outer_steps: usize,
    remaining_diff: f32,
    interpolate: bool,
}

impl StepPlan {
    fn new(mode: TimestepMode, delta: f32, diff: f32) -> Result<Self, String> {
        let (dt, scale, substeps, interpolate) = match mode {
            TimestepMode::Fixed { dt, substeps } => (dt, 1., substeps, false),
            TimestepMode::Variable {
                max_dt,
                time_scale,
                substeps,
            } => ((delta * time_scale).min(max_dt), 1., substeps, false),
            TimestepMode::Interpolated {
                dt,
                time_scale,
                substeps,
            } => (dt, time_scale, substeps, true),
        };
        if !dt.is_finite()
            || dt < 0.
            || (interpolate && dt == 0.)
            || !scale.is_finite()
            || scale < 0.
            || substeps == 0
            || !delta.is_finite()
            || delta < 0.
            || !diff.is_finite()
        {
            return Err("invalid native physics timestep".into());
        }
        let mut remaining_diff = diff;
        let mut outer_steps = 1;
        if interpolate {
            remaining_diff += delta;
            outer_steps = 0;
            // Match Rapier 0.28's repeated f32 subtraction, including its
            // one-step lookahead. Integer division changes boundary rounding.
            while remaining_diff > 0. {
                let next = remaining_diff - dt;
                if next == remaining_diff || !remaining_diff.is_finite() {
                    return Err("native physics clock cannot advance".into());
                }
                remaining_diff = next;
                outer_steps += 1;
            }
        }
        let substep_dt = dt / substeps as f32 * scale;
        if !substep_dt.is_finite() {
            return Err("native physics substep duration overflow".into());
        }
        Ok(Self {
            outer_dt: dt,
            substep_dt,
            substeps,
            outer_steps,
            remaining_diff,
            interpolate,
        })
    }
}

#[derive(SystemParam)]
struct StepIo<'w, 's> {
    time: Res<'w, Time>,
    collision_events: EventWriter<'w, CollisionEvent>,
    force_events: EventWriter<'w, ContactForceEvent>,
    interpolation: Query<
        'w,
        's,
        (
            &'static RapierRigidBodyHandle,
            &'static mut TransformInterpolation,
        ),
    >,
}

impl StepIo<'_, '_> {
    fn start_interpolation(&mut self, context: &RapierContext) {
        for (handle, mut interpolation) in &mut self.interpolation {
            if let Some(body) = context.bodies.get(handle.0) {
                interpolation.start = Some(*body.position());
                interpolation.end = None;
            }
        }
    }

    fn advance(&mut self, context: &mut RapierContext, gravity: Vec3, dt: f32) {
        // Delegate the actual body/joint/CCD solve and custom event handling.
        // Flush after each call because step_simulation replaces its event queue.
        context.step_simulation(
            gravity,
            TimestepMode::Fixed { dt, substeps: 1 },
            Some((&self.collision_events, &self.force_events)),
            &(),
            &self.time,
            &mut SimulationToRenderTime::default(),
            None,
        );
        context.send_bevy_events(&mut self.collision_events, &mut self.force_events);
    }
}

fn advance_context(
    context: &mut RapierContext,
    config: &RapierConfiguration,
    record: &mut ContextSubsteps,
    plan: &StepPlan,
    io: &mut StepIo<'_, '_>,
) {
    for outer in 0..plan.outer_steps {
        if plan.interpolate && outer + 1 == plan.outer_steps {
            io.start_interpolation(context);
        }
        for _ in 0..plan.substeps {
            let before = sample(context);
            io.advance(context, config.gravity, plan.substep_dt);
            record.record(plan.substep_dt, before, context);
        }
    }
    // Writeback uses the original outer timestep for render interpolation.
    context.integration_parameters.dt = plan.outer_dt;
}

fn step(
    mut contexts: Query<(
        Entity,
        &mut RapierContext,
        &RapierConfiguration,
        &mut SimulationToRenderTime,
    )>,
    mode: Res<TimestepMode>,
    mut io: StepIo,
    mut history: ResMut<PhysicsSubsteps>,
) {
    history.0.retain(|entity, _| contexts.contains(*entity));
    for (entity, mut context, config, mut timing) in &mut contexts {
        let record = history.0.entry(entity).or_default();
        record.intervals.clear();
        record.error = None;
        if config.physics_pipeline_active {
            match StepPlan::new(*mode, io.time.delta_secs(), timing.diff) {
                Ok(plan) => {
                    advance_context(&mut context, config, record, &plan, &mut io);
                    timing.diff = plan.remaining_diff;
                }
                Err(error) => {
                    record.error = Some(error);
                }
            }
        } else {
            context.propagate_modified_body_positions_to_colliders();
        }
        if config.query_pipeline_active {
            context.update_query_pipeline();
        }
        context.send_bevy_events(&mut io.collision_events, &mut io.force_events);
    }
}

pub(super) fn install(app: &mut App) {
    app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default().with_default_system_setup(false));
    app.init_resource::<PhysicsSubsteps>()
        .init_resource::<TimestepMode>();
    app.configure_sets(
        PostUpdate,
        (
            PhysicsSet::SyncBackend,
            PhysicsSet::StepSimulation,
            PhysicsSet::Writeback,
        )
            .chain()
            .before(TransformSystem::TransformPropagate),
    );
    app.configure_sets(
        PostUpdate,
        RapierTransformPropagateSet.in_set(PhysicsSet::SyncBackend),
    );
    app.add_systems(
        PostUpdate,
        (
            RapierPhysicsPlugin::<NoUserData>::get_systems(PhysicsSet::SyncBackend),
            step.in_set(PhysicsSet::StepSimulation),
            RapierPhysicsPlugin::<NoUserData>::get_systems(PhysicsSet::Writeback),
        ),
    );
}

#[cfg(test)]
#[path = "substeps_tests.rs"]
mod tests;
