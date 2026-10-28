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
    PropSnapshot { id: part.id.clone(), loading: false, grasp_world: None, articulation: None,
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
    let prop = PropSnapshot { id:part.id.clone(), loading:false, grasp_world:None, articulation:None,
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
fn bedroom_door_controls_passage_in_the_complete_condo_without_moving_avatar() {
    verify_door_passage("scene-doors.json", "bedroom_door_leaf",
        [2.85, 0., -0.55], [2.85, 0., -1.85],
        Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)));
}

#[test]
fn balcony_slider_controls_full_condo_passage_without_moving_avatar() {
    verify_door_passage("scene-balcony-slider.json", "balcony_sliding_door",
        [-1., 0., 4.2], [-1., 0., 5.65], Transform::from_xyz(2., 0., 0.));
}

#[test]
fn bathroom_door_controls_full_condo_passage_without_moving_avatar() {
    verify_door_passage("scene-bathroom-door.json", "bathroom_door_leaf",
        // Stand in the aisle between the island (ending at -1.15) and wall (-1.93).
        [-3.78, 0., -1.55], [-3.78, 0., -2.65],
        Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)));
}

fn verify_door_passage(manifest: &str, part_id: &str, start: [f32; 3], goal: [f32; 3], open: Transform) {
    // Read-only geometry observations; actual joint response is tested separately.
    let package = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo").join(manifest)).unwrap();
    let mechanism = package.mechanisms.iter().find(|m| m.parts.iter().any(|p| p.id == part_id)).unwrap().clone();
    let mut app = App::new();
    app.insert_resource(package);
    super::super::systems::install(&mut app);
    let mut controller = app.world_mut().remove_resource::<OfficeController>().unwrap();
    let motion = serde_json::to_value(&controller.motion).unwrap();
    let revision = controller.revision;
    assert_eq!(snapshot(&controller).generate_waypoint(start, goal).unwrap_err().code(), "no_path");
    let part = mechanism.parts.iter().find(|p| p.id == part_id).unwrap();
    let observed = |offset: Transform| {
        let position = Vec3::from(mechanism.placement) + Vec3::from(part.position)
            + offset.translation + offset.rotation*part.bounds().0;
        PropSnapshot { id:part.id.clone(), loading:false, grasp_world:None, articulation:None,
            transform:Mat4::from_rotation_translation(offset.rotation, position).to_cols_array_2d(),
            aabb_min:[0.; 3], aabb_max:[0.; 3] }
    };
    controller.held.push((Hand::Right, part.id.clone()));
    assert!(!controller.update_prop_observations(&[observed(open)]));
    let opened = snapshot(&controller);
    let frozen = serde_json::to_value(&opened).unwrap();
    let path = opened.generate_waypoint(start, goal).unwrap();
    assert!(opened.waypoint_geometry_is_current(&path).unwrap());
    assert!(opened.office.as_ref().unwrap().navigation_object_ids.contains(&part.id));
    assert_eq!(serde_json::to_value(&opened).unwrap(), frozen);
    assert!(!controller.update_prop_observations(&[observed(Transform::IDENTITY)]));
    let closed = snapshot(&controller);
    assert!(!closed.waypoint_geometry_is_current(&path).unwrap());
    assert_eq!(closed.generate_waypoint(start, goal).unwrap_err().code(), "no_path");
    assert_eq!(controller.revision, revision);
    assert_eq!(serde_json::to_value(&controller.motion).unwrap(), motion);
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

#[test]
fn a_gripped_drawer_pulled_towards_her_does_not_reject_her_own_root() {
    let package = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-drawers.json")).unwrap();
    let mut app = App::new();
    app.insert_resource(package);
    super::super::systems::install(&mut app);
    let mut controller = app.world_mut().remove_resource::<OfficeController>().unwrap();
    controller.objects.retain(|o| o.id == "bedside_upper_drawer");
    // She stands just outside the reserve of the drawer's front.
    let drawer = controller.objects[0].clone();
    let front = drawer.position[2] + drawer.half_extents[2];
    let stand = Vec3::new(drawer.position[0], 0.9, front + NATIVE_ROOT_RESERVE_M + 0.05);
    controller.observed.pelvis = stand.to_array();
    assert!(controller.permits_native_root(stand));
    // The drawer slides 0.15 m out towards her: ungripped, her standing root is now inside its
    // reserve, and she may not step closer.
    controller.objects[0].position[2] += 0.15;
    assert!(!controller.permits_native_root(stand - Vec3::Z * 0.05));
    // Gripped, it moves with her hand: the native check ignores it; planning still keeps it.
    let grip = crate::cc_render::renderer::GrabSnapshot { hand: Hand::Right, prop: "bedside_upper_drawer".into() };
    controller.advance(&SceneState { grabs: vec![grip], ..Default::default() });
    assert!(controller.permits_native_root(stand));
    assert!(!controller.permits_root(stand));
    // Let go, the open drawer is an obstacle again: she may stay or step back, never closer.
    controller.advance(&SceneState::default());
    assert!(controller.held.is_empty());
    assert!(controller.permits_native_root(stand));
    assert!(controller.permits_native_root(stand + Vec3::Z * 0.05));
    assert!(controller.permits_native_root(stand - Vec3::Z * 0.002), "standing sway");
    assert!(!controller.permits_native_root(stand - Vec3::Z * 0.05));
    assert!(!controller.permits_root(stand));
    // Let go clear of it, then slid on into her reserve by itself: the same rule.
    controller.leaving_seat = None;
    assert!(controller.permits_native_root(stand));
    assert!(controller.permits_native_root(stand + Vec3::Z * 0.05));
    assert!(!controller.permits_native_root(stand - Vec3::Z * 0.05));
    // Its release phase lets go of a grip the hand has already slipped off: not an error.
    controller.apply_coupling(super::super::interaction::CouplingCommand::Release {
        object: "bedside_upper_drawer".into(), onto: None });
    assert_eq!(controller.motion_error, None);
}

#[test]
fn standing_at_the_edge_of_an_open_vanity_drawers_reserve_she_may_sway() {
    // Live: let go at full travel, her pelvis 0.2001 m from the drawer swayed 0.6 mm closer.
    let package = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-vanity-drawers.json")).unwrap();
    let mut app = App::new();
    app.insert_resource(package);
    super::super::systems::install(&mut app);
    let mut controller = app.world_mut().remove_resource::<OfficeController>().unwrap();
    controller.observed.ready = true;
    controller.objects.iter_mut().find(|o| o.id == "vanity_upper_drawer").unwrap().position[0] += 0.4;
    let start = Vec3::new(-4.6158905, 0.9324477, -2.7891545);
    controller.observed.pelvis = start.to_array();
    assert!(controller.permits_native_root(Vec3::new(-4.6165109, 0.9316328, -2.7874453)));
    // Live: stood 15 cm from it after pulling, standing up straight moved her 3.2 mm closer.
    let near = Vec3::new(-4.664736, 0.9711016, -2.834644);
    controller.observed.pelvis = near.to_array();
    assert!(controller.permits_native_root(Vec3::new(-4.667926, 0.9727172, -2.8314695)));
    assert!(!controller.permits_native_root(near - Vec3::X * 0.06), "within 10 cm of it");
    // Live: let go of the lower drawer at full travel 20.3 cm from it, just outside its full
    // reserve; standing up moved her 3.2 mm closer.
    controller.objects.iter_mut().find(|o| o.id == "vanity_upper_drawer").unwrap().position[0] -= 0.4;
    controller.objects.iter_mut().find(|o| o.id == "vanity_lower_drawer").unwrap().position[0] += 0.4;
    let edge = Vec3::new(-4.612988, 0.9456605, -2.7784178);
    controller.observed.pelvis = edge.to_array();
    assert!(controller.permits_native_root(Vec3::new(-4.616138, 0.9509126, -2.7807331)));
    assert!(!controller.permits_native_root(edge - Vec3::X * 0.12), "within 10 cm of it");
}
