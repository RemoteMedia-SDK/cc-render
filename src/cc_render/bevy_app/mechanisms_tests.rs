//! Exact exported fridge shapes under diagnostic loads. No avatar or task actuator.
use super::*;
use super::super::physics::{grab::GrabFsms, props::{publish_scene_state, SceneStateTx}};

#[derive(Resource)]
struct SceneReceiver {
    _receiver: tokio::sync::watch::Receiver<crate::cc_render::renderer::SceneState>,
}

fn fixture() -> (App, SceneMechanism) {
    let package: ScenePackage = serde_json::from_str(include_str!("../../../scenes/toronto_condo/scene-articulated.json")).unwrap();
    fixture_for(package, 0)
}

fn fixture_for(package: ScenePackage, mechanism_index: usize) -> (App, SceneMechanism) {
    let (app, mut mechanisms) = fixture_for_indices(package, &[mechanism_index]);
    (app, mechanisms.remove(0))
}

fn fixture_for_indices(package: ScenePackage, indices: &[usize]) -> (App, Vec<SceneMechanism>) {
    let mechanisms: Vec<_> = indices.iter().map(|i| package.mechanisms[*i].clone()).collect();
    let selected = indices.to_vec();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin));
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<SceneSpawner>();
    app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default());
    // Match the live renderer's default integration settings. Rapier's plugin
    // default uses one substep, which differs from our four-substep runtime.
    let physics = crate::cc_render::renderer::PhysicsConfig::default();
    app.insert_resource(TimestepMode::Interpolated {
        dt: 1. / physics.timestep_hz.max(1) as f32, time_scale: 1.,
        substeps: physics.max_substeps_per_frame.max(1) as usize,
    });
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1. / 60.)));
    app.insert_resource(package);
    app.init_resource::<PropRegistry>();
    app.init_resource::<GrabFsms>();
    let (tx, rx) = tokio::sync::watch::channel(Default::default());
    app.insert_resource(SceneStateTx(tx));
    app.insert_resource(SceneReceiver { _receiver: rx });
    app.add_systems(Startup, (super::super::environment::spawn_solids,
        move |mut commands: Commands, package: Res<ScenePackage>| {
            for index in &selected { spawn_bodies(&mut commands, &package.mechanisms[*index]); }
        }));
    app.add_systems(PostUpdate, publish_scene_state.after(bevy::transform::TransformSystem::TransformPropagate));
    for _ in 0..4 { app.update(); }
    (app, mechanisms)
}

#[path = "drawer_tests.rs"]
mod drawers;

#[path = "vanity_tests.rs"]
mod vanity;

#[path = "kitchen_drawer_tests.rs"]
mod kitchen_drawers;

#[path = "cabinet_tests.rs"]
mod cabinets;

#[path = "room_door_tests.rs"]
mod room_doors;

#[path = "balcony_slider_tests.rs"]
mod balcony_slider;

fn door_entities(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    world.query_filtered::<Entity, (With<MechanismBody>, With<ImpulseJoint>)>().iter(world).collect()
}

fn assert_current_snapshots(app: &App, mechanism: &SceneMechanism, doors: &[Entity]) {
    let state = app.world().resource::<SceneStateTx>().0.borrow();
    for entity in doors {
        let prop = app.world().get::<Prop>(*entity).unwrap();
        let pose = app.world().get::<Transform>(*entity).unwrap();
        let published = state.props.iter().find(|p| p.id == prop.id).unwrap();
        let actual = Transform::from_matrix(Mat4::from_cols_array_2d(&published.transform));
        assert!(actual.translation.distance(pose.translation) < 1e-5);
        assert!(actual.rotation.angle_between(pose.rotation) < 1e-3);
        let part = mechanism.parts.iter().find(|p| p.id == prop.id).unwrap();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for shape in &part.colliders {
            for corner in 0..8 {
                let sign = Vec3::new(if corner & 1 == 0 { -1. } else { 1. },
                    if corner & 2 == 0 { -1. } else { 1. }, if corner & 4 == 0 { -1. } else { 1. });
                let point = pose.transform_point(Vec3::from(shape.position) - part.bounds().0
                    + sign * Vec3::from(shape.half_extents));
                min = min.min(point); max = max.max(point);
            }
        }
        assert!(min.distance(Vec3::from(published.aabb_min)) < 1e-4);
        assert!(max.distance(Vec3::from(published.aabb_max)) < 1e-4);
        assert_joint_snapshot(app, part, *entity, published);
    }
    assert!(state.props.iter().filter(|p| mechanism.parts.iter()
        .any(|part| part.id == p.id && part.joint.is_none())).all(|p| p.articulation.is_none()));
}

fn assert_joint_snapshot(app: &App, part: &MechanismPart, entity: Entity,
    published: &crate::cc_render::renderer::PropSnapshot) {
    use crate::cc_render::renderer::ArticulationCoordinate;
    let observed = published.articulation.as_ref().expect("permanent joint facts");
    let authored = part.joint.as_ref().unwrap();
    let joint = app.world().get::<ImpulseJoint>(entity).unwrap();
    let parent = app.world().get::<Transform>(joint.parent).unwrap();
    let child = app.world().get::<Transform>(entity).unwrap();
    assert_eq!(observed.parent_id, authored.parent);
    assert!(observed.registered);
    assert!(!observed.motor_enabled);
    let a = parent.transform_point(joint.data.as_ref().local_anchor1());
    let b = child.transform_point(joint.data.as_ref().local_anchor2());
    let axis = parent.rotation * Vec3::from(authored.axis);
    assert!(a.distance(Vec3::from(observed.parent_anchor_world)) < 1e-5);
    assert!(b.distance(Vec3::from(observed.child_anchor_world)) < 1e-5);
    assert!(axis.distance(Vec3::from(observed.axis_world)) < 1e-5);
    match observed.state {
        ArticulationCoordinate::Revolute { angle_rad, limits_rad } => {
            assert_eq!(limits_rad, authored.limits_rad);
            let yaw = (parent.rotation.inverse() * child.rotation).to_euler(EulerRot::YXZ).0;
            assert!((angle_rad-yaw).abs() < 1e-4);
        }
        ArticulationCoordinate::Prismatic { travel_m, limits_m } => {
            assert_eq!(limits_m, authored.limits_m);
            assert!((travel_m-(b-a).dot(axis)).abs() < 1e-5);
        }
    }
}

#[test]
fn exact_fridge_shapes_preserve_cavity_mass_and_rebased_joint_frames() {
    let (mut app, mechanism) = fixture();
    let world = app.world_mut();
    assert!(!world.query::<&Name>().iter(world).any(|n| n.as_str() == "collision:kitchen_fridge"));
    let bodies: Vec<_> = world.query_filtered::<(Entity, &Prop, &Transform), With<MechanismBody>>()
        .iter(world).map(|(e, p, t)| (e, p.id.clone(), *t)).collect();
    assert_eq!(bodies.len(), 3);
    for (entity, id, transform) in &bodies {
        let part = mechanism.parts.iter().find(|p| &p.id == id).unwrap();
        let collider = world.get::<Collider>(*entity).unwrap();
        let cavity = Vec3::from(mechanism.placement) + Vec3::new(0., 1.25, 0.);
        assert!(!collider.raw.contains_point(&bevy_rapier3d::rapier::na::Isometry3::identity(),
            &bevy_rapier3d::rapier::na::Point3::from((cavity - transform.translation).to_array())), "cavity blocked by {id}");
        if let Some(joint) = &part.joint {
            let hinge = world.get::<ImpulseJoint>(*entity).unwrap();
            let parent = world.get::<Transform>(hinge.parent).unwrap();
            let a = parent.transform_point(hinge.data.as_ref().local_anchor1());
            let b = transform.transform_point(hinge.data.as_ref().local_anchor2());
            assert!(a.distance(b) < 1e-5);
            assert!(a.distance(Vec3::from(mechanism.placement) + Vec3::from(joint.parent_anchor)) < 1e-5);
            let context = world.query::<&RapierContext>().single(world);
            let body = &context.bodies[context.entity2body()[entity]];
            assert!((body.mass() - part.mass_kg.unwrap()).abs() < 1e-3);
        }
    }
}

#[test]
fn passive_fridge_doors_open_close_and_publish_current_bounds() {
    let (mut app, mechanism) = fixture();
    let doors = door_entities(&mut app);
    let mut worst_anchor = 0_f32;
    let mut minimum = 0_f32;
    let mut maximum = 0_f32;
    for frame in 0..480 {
        let force = if frame < 240 { Vec3::X * 12. } else { Vec3::new(-12., 0., 12.) };
        for entity in &doors { app.world_mut().entity_mut(*entity).insert(ExternalForce { force, ..default() }); }
        app.update();
        for entity in &doors {
            let pose = app.world().get::<Transform>(*entity).unwrap();
            let hinge = app.world().get::<ImpulseJoint>(*entity).unwrap();
            let parent = app.world().get::<Transform>(hinge.parent).unwrap();
            worst_anchor = worst_anchor.max(parent.transform_point(hinge.data.as_ref().local_anchor1())
                .distance(pose.transform_point(hinge.data.as_ref().local_anchor2())));
            let angle = pose.rotation.to_euler(EulerRot::YXZ).0;
            minimum = minimum.min(angle); maximum = maximum.max(angle);
            if frame == 239 { assert!((angle.to_degrees() - 90.).abs() < 1., "open angle {}", angle.to_degrees()); }
            if frame == 479 { assert!(angle.abs() < 0.02, "closed angle {}", angle.to_degrees()); }
        }
        if frame == 239 || frame == 479 { assert_current_snapshots(&app, &mechanism, &doors); }
    }
    assert!(worst_anchor < 0.005, "anchor error {worst_anchor}");
    assert!(minimum > -0.02 && maximum < 1.60, "angular envelope {minimum}..{maximum}");
    let published = app.world().resource::<SceneStateTx>().0.borrow();
    assert_eq!(published.props.len(), mechanism.parts.len());
    assert!(published.props.iter().all(|p| p.aabb_max[1] > p.aabb_min[1]));
    println!("FRIDGE_RAPIER_FIXTURE {}", serde_json::json!({"maximum_anchor_error_m":worst_anchor,
        "minimum_degrees":minimum.to_degrees(),"maximum_degrees":maximum.to_degrees(),
        "scope":"Diagnostic external forces; not avatar manipulation","published_parts":published.props.len()}));
}

#[test]
fn replacement_visibility_is_environment_scoped_and_readiness_requires_every_node() {
    let package: ScenePackage = serde_json::from_str(include_str!("../../../scenes/toronto_condo/scene-articulated.json")).unwrap();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, HierarchyPlugin));
    app.insert_resource(package.clone());
    app.init_resource::<MechanismReadiness>();
    app.add_systems(Update, hide_replaced_nodes);
    let root = app.world_mut().spawn(super::super::camera::EnvironmentSceneRoot).id();
    let nodes: Vec<_> = package.mechanisms[0].replaces_nodes.iter().map(|name|
        app.world_mut().spawn((Name::new(name.clone()), Visibility::Inherited)).set_parent(root).id()).collect();
    let unrelated = app.world_mut().spawn((Name::new(package.mechanisms[0].replaces_nodes[0].clone()), Visibility::Inherited)).id();
    app.update();
    for entity in nodes { assert_eq!(app.world().get::<Visibility>(entity), Some(&Visibility::Hidden)); }
    assert_eq!(app.world().get::<Visibility>(unrelated), Some(&Visibility::Inherited));
    let mut state = app.world_mut().resource_mut::<MechanismReadiness>();
    assert!(!state.ready(&package), "physics registration is required");
    state.bodies = 3; state.joints = 2;
    assert!(state.ready(&package));
    state.hidden_nodes.remove(&package.mechanisms[0].replaces_nodes[0]);
    assert!(!state.ready(&package), "missing replacement must prevent readiness");
}
