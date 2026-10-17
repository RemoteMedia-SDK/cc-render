//! Compare the observed driver against Rapier's unmodified Bevy plugin.
use super::*;
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

fn fixture(observed: bool, mode: TimestepMode) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin));
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<SceneSpawner>();
    if observed {
        install(&mut app);
    } else {
        app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default());
    }
    app.insert_resource(mode);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    app.world_mut().spawn((
        Name::new("floor"),
        RigidBody::Fixed,
        Collider::cuboid(5., 0.1, 5.),
        Transform::from_xyz(0., -0.1, 0.),
    ));
    app.world_mut().spawn((
        Name::new("falling"),
        RigidBody::Dynamic,
        Collider::ball(0.2),
        Transform::from_xyz(0., 0.8, 0.),
        Restitution::coefficient(0.4),
        TransformInterpolation::default(),
        ActiveEvents::COLLISION_EVENTS | ActiveEvents::CONTACT_FORCE_EVENTS,
        ContactForceEventThreshold(0.),
    ));
    app.world_mut().spawn((
        Name::new("wall"),
        RigidBody::Fixed,
        Collider::cuboid(0.02, 3., 3.),
        Transform::from_xyz(1.5, 2., 0.),
    ));
    app.world_mut().spawn((
        Name::new("fast"),
        RigidBody::Dynamic,
        Ccd::enabled(),
        Collider::ball(0.05),
        Transform::from_xyz(0., 2., 0.),
        Velocity::linear(Vec3::X * 100.),
        ActiveEvents::COLLISION_EVENTS,
        TransformInterpolation::default(),
    ));
    let anchor = app
        .world_mut()
        .spawn((
            Name::new("anchor"),
            RigidBody::Fixed,
            Transform::from_xyz(-2., 0.8, 0.),
        ))
        .id();
    app.world_mut().spawn((
        Name::new("hinged"),
        RigidBody::Dynamic,
        Collider::cuboid(0.5, 0.4, 0.04),
        Transform::from_xyz(-1.5, 0.8, 0.),
        Velocity {
            linvel: Vec3::NEG_Z,
            angvel: Vec3::Y * 2.,
        },
        ImpulseJoint::new(
            anchor,
            RevoluteJointBuilder::new(Vec3::Y)
                .local_anchor2(Vec3::new(-0.5, 0., 0.))
                .limits([-0.6, 0.6]),
        ),
        TransformInterpolation::default(),
    ));
    app.world_mut().spawn((
        Name::new("kinematic"),
        RigidBody::KinematicVelocityBased,
        Collider::cuboid(0.1, 0.1, 0.1),
        Transform::from_xyz(-4., 1., 2.),
        Velocity {
            linvel: Vec3::Z * 0.1,
            angvel: Vec3::Y * 1.5,
        },
        TransformInterpolation::default(),
    ));
    app
}

#[derive(Debug, PartialEq)]
struct State {
    name: String,
    pose: Isometry<f32>,
    velocity: (Vector3<f32>, Vector3<f32>),
    render: Transform,
    interpolation: Option<TransformInterpolation>,
}

fn states(app: &mut App) -> Vec<State> {
    let world = app.world_mut();
    let data: Vec<_> = world
        .query::<(Entity, &Name, &Transform, Option<&TransformInterpolation>)>()
        .iter(world)
        .map(|(e, n, t, i)| (e, n.to_string(), *t, i.copied()))
        .collect();
    let context = world.query::<&RapierContext>().single(world);
    let mut result: Vec<_> = data
        .into_iter()
        .filter_map(|(entity, name, render, interpolation)| {
            let handle = context.entity2body().get(&entity)?;
            let body = context.bodies.get(*handle)?;
            Some(State {
                name,
                pose: *body.position(),
                velocity: (*body.linvel(), *body.angvel()),
                render,
                interpolation,
            })
        })
        .collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

fn events(app: &App) -> (Vec<String>, Vec<String>) {
    let world = app.world();
    let name = |e| {
        world
            .get::<Name>(e)
            .map(|n| n.as_str())
            .unwrap_or("removed")
    };
    let collisions = world
        .resource::<Events<CollisionEvent>>()
        .iter_current_update_events()
        .map(|event| match event {
            CollisionEvent::Started(a, b, flags) => {
                format!("start:{}:{}:{flags:?}", name(*a), name(*b))
            }
            CollisionEvent::Stopped(a, b, flags) => {
                format!("stop:{}:{}:{flags:?}", name(*a), name(*b))
            }
        })
        .collect();
    let forces = world
        .resource::<Events<ContactForceEvent>>()
        .iter_current_update_events()
        .map(|e| {
            format!(
                "{}:{}:{:?}:{}:{:?}:{}",
                name(e.collider1),
                name(e.collider2),
                e.total_force,
                e.total_force_magnitude,
                e.max_force_direction,
                e.max_force_magnitude
            )
        })
        .collect();
    (collisions, forces)
}

fn check_intervals(app: &mut App, previous: &mut (u64, f64), expect_ccd: bool) -> usize {
    let world = app.world_mut();
    let history = world.resource::<PhysicsSubsteps>();
    let (entity, record) = history.0.iter().next().unwrap();
    assert!(record.error.is_none(), "{:?}", record.error);
    let mut last_after: Option<&Vec<BodyBoundary>> = None;
    for step in &record.intervals {
        assert_eq!(step.sequence, previous.0 + 1);
        assert_eq!(step.start_seconds, previous.1);
        assert!(step.end_seconds >= step.start_seconds);
        if let Some(before) = last_after {
            for (a, b) in before.iter().zip(&step.before) {
                assert_eq!(a.handle, b.handle);
                assert_eq!(a.pose, b.pose);
                assert_eq!(a.linear_velocity, b.linear_velocity);
                assert_eq!(a.angular_velocity, b.angular_velocity);
            }
        }
        assert_eq!(step.before.len(), step.after.len());
        assert_eq!(
            step.before.iter().filter(|b| b.ccd_enabled).count(),
            usize::from(expect_ccd)
        );
        last_after = Some(&step.after);
        *previous = (step.sequence, step.end_seconds);
    }
    if let Some(after) = last_after {
        let context = world.get::<RapierContext>(*entity).unwrap();
        assert_eq!(after.len(), context.bodies.len());
        for snapshot in after {
            let body = context.bodies.get(snapshot.handle).unwrap();
            assert_eq!(snapshot.pose, *body.position());
            assert_eq!(snapshot.linear_velocity, *body.linvel());
            assert_eq!(snapshot.angular_velocity, *body.angvel());
            assert_eq!(snapshot.ccd_enabled, body.is_ccd_enabled());
        }
    }
    assert_eq!(*previous, (record.sequence, record.elapsed_seconds));
    record.intervals.len()
}

#[test]
fn observed_substeps_preserve_native_bodies_joints_events_and_render_interpolation() {
    let modes = [
        TimestepMode::Fixed {
            dt: 1. / 60.,
            substeps: 4,
        },
        TimestepMode::Variable {
            max_dt: 1. / 30.,
            time_scale: 0.7,
            substeps: 3,
        },
        TimestepMode::Interpolated {
            dt: 1. / 60.,
            time_scale: 0.5,
            substeps: 4,
        },
        TimestepMode::Interpolated {
            dt: 1. / 60.,
            time_scale: 2.,
            substeps: 3,
        },
    ];
    for mode in modes {
        let mut native = fixture(false, mode);
        let mut observed = fixture(true, mode);
        let mut clock = (0, 0.);
        let mut skipped = 0;
        let mut multiple = 0;
        let mut event_count = 0;
        for frame in 0..100 {
            let duration = Duration::from_micros([0, 1000, 70000, 4000, 23000, 16667][frame % 6]);
            for app in [&mut native, &mut observed] {
                app.insert_resource(TimeUpdateStrategy::ManualDuration(duration));
                if frame == 55 {
                    let world = app.world_mut();
                    let entity = world
                        .query::<(Entity, &Name)>()
                        .iter(world)
                        .find(|(_, name)| name.as_str() == "fast")
                        .unwrap()
                        .0;
                    world.despawn(entity);
                }
                app.update();
            }
            assert_eq!(
                states(&mut native),
                states(&mut observed),
                "{mode:?}, frame {frame}"
            );
            let a = events(&native);
            let b = events(&observed);
            event_count += a.0.len();
            assert_eq!(a, b);
            let time_state = |app: &mut App| {
                let w = app.world_mut();
                let (c, t) = w
                    .query::<(&RapierContext, &SimulationToRenderTime)>()
                    .single(w);
                (c.integration_parameters.dt, t.diff)
            };
            assert_eq!(time_state(&mut native), time_state(&mut observed));
            let count = check_intervals(&mut observed, &mut clock, frame < 55);
            skipped += usize::from(count == 0);
            multiple += usize::from(count > 4);
            if frame < 18 {
                eprintln!(
                    "NATIVE_STEP_FRAME {}",
                    serde_json::json!({
                        "mode":format!("{mode:?}"),"frame":frame,"substeps":count,
                        "physical_seconds":clock.1,"sequence":clock.0,
                        "render_delta_seconds":observed.world().resource::<Time>().delta_secs(),
                        "native_state_event_and_interpolation_match":true,
                    })
                );
            }
        }
        assert!(
            event_count > 0,
            "collision event comparison must be exercised"
        );
        if matches!(mode, TimestepMode::Interpolated { .. }) {
            assert!(skipped > 0 && multiple > 0);
        }
        eprintln!("NATIVE_SUBSTEP_PARITY {mode:?} steps={} physical_seconds={} collision_events={event_count}",clock.0,clock.1);
    }
}

#[test]
fn paused_native_physics_records_no_fictitious_time_or_motion() {
    let mut app = fixture(
        true,
        TimestepMode::Fixed {
            dt: 1. / 60.,
            substeps: 4,
        },
    );
    app.update();
    let mut clock = (0, 0.);
    check_intervals(&mut app, &mut clock, true);
    let before = states(&mut app);
    for mut config in app
        .world_mut()
        .query::<&mut RapierConfiguration>()
        .iter_mut(app.world_mut())
    {
        config.physics_pipeline_active = false;
    }
    for _ in 0..3 {
        app.update();
        assert_eq!(check_intervals(&mut app, &mut clock, true), 0);
    }
    assert_eq!(states(&mut app), before);
}

#[test]
fn invalid_native_timing_is_rejected_without_an_unbounded_clock_loop() {
    assert!(StepPlan::new(
        TimestepMode::Interpolated {
            dt: 0.,
            time_scale: 1.,
            substeps: 1
        },
        1.,
        0.
    )
    .is_err());
    assert!(StepPlan::new(
        TimestepMode::Fixed {
            dt: 1.,
            substeps: 0
        },
        1.,
        0.
    )
    .is_err());
    assert!(StepPlan::new(
        TimestepMode::Fixed {
            dt: f32::NAN,
            substeps: 1
        },
        1.,
        0.
    )
    .is_err());
    assert!(StepPlan::new(
        TimestepMode::Interpolated {
            dt: f32::MIN_POSITIVE,
            time_scale: 1.,
            substeps: 1
        },
        1.,
        0.
    )
    .is_err());
}

#[test]
fn removing_a_physics_context_removes_its_observation_clock() {
    let mut app = fixture(
        true,
        TimestepMode::Fixed {
            dt: 1. / 60.,
            substeps: 2,
        },
    );
    app.update();
    let extra = app
        .world_mut()
        .spawn((
            RapierContext::default(),
            RapierConfiguration::new(1.),
            SimulationToRenderTime::default(),
        ))
        .id();
    app.update();
    let history = app.world().resource::<PhysicsSubsteps>();
    assert_eq!(history.0.len(), 2);
    assert_eq!(history.0[&extra].sequence, 2);
    assert!(history.0[&extra]
        .intervals
        .iter()
        .all(|s| s.before.is_empty() && s.after.is_empty()));
    app.world_mut().despawn(extra);
    app.update();
    let history = app.world().resource::<PhysicsSubsteps>();
    assert_eq!(history.0.len(), 1);
    assert!(!history.0.contains_key(&extra));
}
