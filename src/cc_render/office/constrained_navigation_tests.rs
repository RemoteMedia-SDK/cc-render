//! Read-only navigation fixtures over the authored fridge, not body motion tests.
use super::*;
use crate::cc_render::{scene_mechanism::SceneMechanism, scene_package::ScenePackage,
    renderer::PropSnapshot};

fn fridge() -> (OfficeController, SceneMechanism) {
    let package: ScenePackage = serde_json::from_str(include_str!("../../../scenes/toronto_condo/scene-articulated.json")).unwrap();
    let mechanism = package.mechanisms[0].clone();
    let mut app = App::new();
    app.insert_resource(package);
    super::super::systems::install(&mut app);
    let mut controller = app.world_mut().remove_resource::<OfficeController>().unwrap();
    assert_eq!(controller.constrained_objects.len(), 2);
    // Isolate the appliance for route tests; the nearby bathroom wall otherwise
    // masks the door's contribution to the inflated navigation footprint.
    controller.objects.retain(|object| object.id.starts_with("fridge_"));
    controller.held.push((Hand::Right, "fridge_main_door".into()));
    controller.observed.ready = true;
    controller.observed.pelvis = [-3.3, 0.95, -1.6];
    (controller, mechanism)
}

fn snapshot(controller: &OfficeController) -> SceneState {
    SceneState { office: Some(controller.snapshot()), ..Default::default() }
}

fn observed_door(mechanism: &SceneMechanism, degrees: f32) -> PropSnapshot {
    let part = mechanism.parts.iter().find(|p| p.id == "fridge_main_door").unwrap();
    let rotation = Quat::from_rotation_y(degrees.to_radians());
    let position = Vec3::from(mechanism.placement) + Vec3::from(part.position) + rotation * part.bounds().0;
    PropSnapshot { id: part.id.clone(), loading: false, grasp_world: None,
        transform: Mat4::from_rotation_translation(rotation, position).to_cols_array_2d(),
        aabb_min: [0.; 3], aabb_max: [0.; 3] }
}

#[test]
fn held_door_rotation_invalidates_preview_without_changing_motion() {
    let (mut controller, mechanism) = fridge();
    let before_motion = serde_json::to_value(&controller.motion).unwrap();
    let before_revision = controller.revision;
    let start = [-3.3, 0., -1.6];
    let goal = [-4.6, 0., -1.83];
    let closed = snapshot(&controller);
    let path = closed.generate_waypoint(start, goal).unwrap();
    assert!(closed.waypoint_geometry_is_current(&path).unwrap());
    assert_eq!(path.navigation_geometry_sha256.len(), 64);
    assert!(closed.office.as_ref().unwrap().navigation_object_ids.contains(&"fridge_main_door".into()));
    // Even sub-millimetre movement changes the preview identity. No per-frame
    // 3 cm threshold or avatar-clock progress is needed to observe the change.
    for angle in [0.001, 0.002, 90.] {
        assert!(!controller.update_prop_observations(&[observed_door(&mechanism, angle)]));
        let state = snapshot(&controller);
        assert!(!state.waypoint_geometry_is_current(&path).unwrap());
        assert_eq!(state.office.as_ref().unwrap().avatar.displayed_pts_ms, path.snapshot_pts_ms);
    }
    let opened = snapshot(&controller);
    let frozen = serde_json::to_value(&opened).unwrap();
    assert_eq!(opened.generate_waypoint(start, goal).unwrap_err().code(), "goal_blocked");
    let fresh = opened.generate_waypoint(start, [-3., 0., 0.]).unwrap();
    assert!(opened.waypoint_geometry_is_current(&fresh).unwrap());
    assert_eq!(serde_json::to_value(&opened).unwrap(), frozen);
    assert_eq!(serde_json::to_value(&controller.motion).unwrap(), before_motion);
    assert_eq!(controller.revision, before_revision);
}

#[test]
fn waypoint_hash_rejects_old_sessions_and_legacy_previews() {
    let (controller, _) = fridge();
    let state = snapshot(&controller);
    let mut path = state.generate_waypoint([-3.3, 0., -1.6], [-3., 0., 0.]).unwrap();
    path.session_id.push_str("-other");
    assert!(!state.waypoint_geometry_is_current(&path).unwrap());
    path.session_id = controller.motion.session_id.clone();
    path.navigation_geometry_sha256.clear();
    assert!(!state.waypoint_geometry_is_current(&path).unwrap());
}

#[test]
fn observed_drawer_translation_keeps_held_obstacle_and_invalidates_preview() {
    // Geometry observation fixture only; actual sliding is tested in Rapier.
    let package = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-drawers.json")).unwrap();
    let mechanism = package.mechanisms[1].clone();
    let mut app = App::new();
    app.insert_resource(package);
    super::super::systems::install(&mut app);
    let mut controller = app.world_mut().remove_resource::<OfficeController>().unwrap();
    controller.objects.retain(|o| o.id.starts_with("bedside_"));
    controller.held.push((Hand::Right, "bedside_upper_drawer".into()));
    let motion = serde_json::to_value(&controller.motion).unwrap();
    let revision = controller.revision;
    let start = [5., 0., -3.];
    let goal = [5.52, 0., -3.43];
    let closed = snapshot(&controller);
    let path = closed.generate_waypoint(start, goal).unwrap();
    let part = &mechanism.parts[2];
    assert_eq!(part.id, "bedside_upper_drawer");
    let position = Vec3::from(mechanism.placement) + Vec3::from(part.position)
        + part.bounds().0 + Vec3::Z * 0.4;
    let prop = PropSnapshot { id:part.id.clone(), loading:false, grasp_world:None,
        transform:Mat4::from_translation(position).to_cols_array_2d(),
        aabb_min:[0.; 3], aabb_max:[0.; 3] };
    assert!(!controller.update_prop_observations(&[prop]));
    let opened = snapshot(&controller);
    assert!(opened.office.as_ref().unwrap().navigation_object_ids.contains(&part.id));
    assert!(!opened.waypoint_geometry_is_current(&path).unwrap());
    assert_eq!(opened.generate_waypoint(start, goal).unwrap_err().code(), "goal_blocked");
    assert_eq!(controller.revision, revision);
    assert_eq!(serde_json::to_value(&controller.motion).unwrap(), motion);
}

#[test]
fn declared_kinematic_attachment_is_rejected_for_a_hinged_part() {
    let (mut controller, _) = fridge();
    let objects = serde_json::to_value(&controller.objects).unwrap();
    controller.apply_coupling(super::super::interaction::CouplingCommand::Attach {
        object: "fridge_main_door".into(), anchors: vec![(Hand::Right, [0.; 3], None)], slide_on: None,
    });
    assert!(controller.couplings.is_empty());
    assert!(controller.motion_error.as_ref().unwrap().contains("physical contact grip"));
    assert_eq!(serde_json::to_value(&controller.objects).unwrap(), objects);
}

#[test]
fn legacy_coupling_cannot_reposition_or_freeze_a_body_with_a_permanent_joint() {
    use bevy_rapier3d::prelude::*;
    use crate::cc_render::bevy_app::{physics::props::{Prop, PropRegistry}, pose::CC5BindRotations};
    let (mut controller, _) = fridge();
    controller.couplings.push(Coupling { object: "fridge_main_door".into(), anchors: vec![],
        slide_height: None, release: Some(None), latch: None });
    controller.homes.insert("fridge_main_door".into(), [100., 1., 0., 0., 0., 0., 1.]);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin));
    app.insert_resource(controller);
    app.init_resource::<CC5BindRotations>();
    app.init_resource::<super::super::hand::HeldInHand>();
    let parent = app.world_mut().spawn_empty().id();
    let pose = Transform::from_xyz(-5., 1.5, -1.5);
    let hinge = ImpulseJoint::new(parent, RevoluteJointBuilder::new(Vec3::Y));
    let entity = app.world_mut().spawn((pose, Velocity::zero(), RigidBody::Dynamic, hinge,
        Prop { id: "fridge_main_door".into(), loading: false, grasp_local: None })).id();
    let mut registry = PropRegistry::default();
    registry.by_id.insert("fridge_main_door".into(), entity);
    app.insert_resource(registry);
    app.add_systems(Update, super::super::coupling::apply_couplings);
    app.update();
    assert_eq!(app.world().get::<Transform>(entity), Some(&pose));
    assert_eq!(app.world().get::<RigidBody>(entity), Some(&RigidBody::Dynamic));
    assert_eq!(app.world().get::<ImpulseJoint>(entity), Some(&hinge));
}
