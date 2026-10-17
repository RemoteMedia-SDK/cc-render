//! Synthetic physics fixtures, not avatar motion acceptance.
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use tokio::sync::mpsc::{self, UnboundedSender};

use super::{body_colliders::BodyBoneCollider, grab::{self, ContactGrip, GrabFsms, GrabState},
    props::{self, DespawnCmdRx, GrabCmdRx, Prop, PropCmdRx, PropRegistry}, StoredPhysicsConfig};
use crate::cc_render::renderer::{GrabCmd, GrabOrRelease, Hand, PhysicsConfig, PropCmd, PropShape};

struct Fixture {
    app: App,
    door: Entity,
    hinge: ImpulseJoint,
    grab: UnboundedSender<GrabOrRelease>,
    spawn: UnboundedSender<PropCmd>,
    despawn: UnboundedSender<String>,
}

fn fixture() -> Fixture {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin, AssetPlugin::default()));
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<SceneSpawner>();
    app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default());
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1. / 60.)));
    app.insert_resource(StoredPhysicsConfig(PhysicsConfig::default()));
    app.init_resource::<GrabFsms>();
    app.add_systems(Update, props::handle_prop_cmds);
    app.add_systems(PostUpdate, grab::handle_grab_cmds.after(PhysicsSet::Writeback));
    let (grab, rx) = mpsc::unbounded_channel();
    app.insert_resource(GrabCmdRx(std::sync::Mutex::new(rx)));
    let (spawn, rx) = mpsc::unbounded_channel();
    app.insert_resource(PropCmdRx(std::sync::Mutex::new(rx)));
    let (despawn, rx) = mpsc::unbounded_channel();
    app.insert_resource(DespawnCmdRx(std::sync::Mutex::new(rx)));
    for smpl_idx in [20, 21] {
        app.world_mut().spawn((Transform::from_xyz(0.031, 1., 0.),
            RigidBody::KinematicPositionBased, Collider::ball(0.02), BodyBoneCollider { smpl_idx },
            CollisionGroups::new(Group::GROUP_32, Group::ALL & !Group::GROUP_32)));
    }
    let base = app.world_mut().spawn((Transform::from_xyz(0., 1., 0.), RigidBody::Fixed)).id();
    let hinge = ImpulseJoint::new(base, RevoluteJointBuilder::new(Vec3::Y)
        .limits([0., std::f32::consts::FRAC_PI_2]).build());
    let door = app.world_mut().spawn((Transform::from_xyz(0., 1., 0.),
        RigidBody::Dynamic, Collider::ball(0.02), GravityScale(0.), hinge,
        CollisionGroups::new(Group::GROUP_1, Group::ALL & !Group::GROUP_32),
        Prop { id: "door".into(), loading: false, grasp_local: None })).id();
    let mut registry = PropRegistry::default();
    registry.by_id.insert("door".into(), door);
    app.insert_resource(registry);
    let mut result = Fixture { app, door, hinge, grab, spawn, despawn };
    result.tick(4);
    result
}

impl Fixture {
    fn tick(&mut self, frames: usize) {
        for _ in 0..frames { self.app.update(); }
    }

    fn acquire(&mut self, hand: Hand) -> Entity {
        self.grab.send(GrabOrRelease::Grab(GrabCmd {
            hand, target: "door".into(), palm_offset_override: None,
        })).unwrap();
        self.tick(4);
        let fsms = self.app.world().resource::<GrabFsms>();
        let state = match hand { Hand::Left => &fsms.left, Hand::Right => &fsms.right };
        let GrabState::Attached { prop_entity, grip_entity, .. } = state else {
            panic!("Contact fixture did not attach: {state:?}");
        };
        assert_eq!(*prop_entity, self.door);
        assert!(self.app.world().get::<RapierImpulseJointHandle>(*grip_entity).is_some());
        *grip_entity
    }

    fn joint_count(&mut self) -> usize {
        let world = self.app.world_mut();
        let count = world.query::<&RapierContext>().single(world).impulse_joints.len();
        if world.get_entity(self.door).is_ok() {
            let children = world.get::<Children>(self.door).map_or(&[][..], |children| &children[..]);
            assert_eq!(children.len(), count - 1, "Stale child grip references");
            assert!(children.iter().all(|child| world.get::<ContactGrip>(*child).is_some()));
        }
        count
    }

    fn assert_hinge(&self, handle: RapierImpulseJointHandle) {
        assert_eq!(self.app.world().get::<ImpulseJoint>(self.door), Some(&self.hinge));
        assert_eq!(self.app.world().get::<RapierImpulseJointHandle>(self.door).unwrap().0, handle.0);
    }
}

#[test]
fn two_hands_swap_and_release_preserve_the_live_hinge() {
    let mut f = fixture();
    let handle = *f.app.world().get::<RapierImpulseJointHandle>(f.door).unwrap();
    let left = f.acquire(Hand::Left);
    let right = f.acquire(Hand::Right);
    assert_ne!(left, right);
    assert_eq!(f.joint_count(), 3);
    f.assert_hinge(handle);
    let replacement = f.acquire(Hand::Right);
    assert_ne!(right, replacement);
    assert!(f.app.world().get_entity(right).is_err());
    assert!(f.app.world().get_entity(left).is_ok());
    assert_eq!(f.joint_count(), 3);
    f.grab.send(GrabOrRelease::Release(Hand::Left)).unwrap();
    f.tick(4);
    assert!(f.app.world().get_entity(left).is_err());
    assert!(f.app.world().get_entity(replacement).is_ok());
    assert_eq!(f.joint_count(), 2);
    f.grab.send(GrabOrRelease::Release(Hand::Right)).unwrap();
    f.tick(4);
    assert_eq!(f.joint_count(), 1);
    f.assert_hinge(handle);
    // Diagnostic initial angular velocity: the released body must still rotate
    // about its hinge, stay anchored, and stop at the permanent angular limit.
    f.app.world_mut().entity_mut(f.door).insert(Velocity { angvel: Vec3::Y * 2., ..default() });
    f.tick(90);
    let pose = f.app.world().get::<Transform>(f.door).unwrap();
    assert!(pose.translation.distance(Vec3::Y) < 0.001);
    let angle = pose.rotation.to_euler(EulerRot::YXZ).0;
    assert!(angle > 1.4 && angle < 1.59, "released hinge angle: {angle}");
    f.assert_hinge(handle);
}

#[test]
fn removing_or_replacing_a_two_hand_prop_cleans_up_all_grips() {
    for replace in [false, true] {
        let mut f = fixture();
        let grips = [f.acquire(Hand::Left), f.acquire(Hand::Right)];
        if replace {
            f.spawn.send(PropCmd {
                id: "door".into(), shape: PropShape::Sphere { radius: 0.02 },
                initial_transform: Mat4::from_translation(Vec3::new(2., 1., 0.)).to_cols_array_2d(),
                mass_kg: 1., friction: 0.5, restitution: 0., grasp: None,
            }).unwrap();
        } else {
            f.despawn.send("door".into()).unwrap();
        }
        f.tick(4);
        assert!(f.app.world().get_entity(f.door).is_err());
        for grip in grips { assert!(f.app.world().get_entity(grip).is_err()); }
        assert_eq!(f.joint_count(), 0);
        let fsms = f.app.world().resource::<GrabFsms>();
        assert!(matches!(fsms.left, GrabState::Idle));
        assert!(matches!(fsms.right, GrabState::Idle));
        let registry = f.app.world().resource::<PropRegistry>();
        assert_eq!(registry.pending_failures.len(), 2);
        let reason = if replace { "prop_replaced" } else { "prop_removed" };
        assert!(registry.pending_failures.iter().all(|failure| failure.reason == reason));
        assert_eq!(registry.by_id.contains_key("door"), replace);
        let world = f.app.world_mut();
        assert_eq!(world.query_filtered::<Entity, With<ContactGrip>>().iter(world).count(), 0);
    }
}

/// Two drawers on X rails (0-0.40 m), the upper gripped by a kinematic wrist touching its front.
/// Returns the app and the drawers' entities.
fn drawers() -> (App, UnboundedSender<GrabOrRelease>, Entity, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin, AssetPlugin::default()));
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<SceneSpawner>();
    app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default());
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1. / 60.)));
    app.insert_resource(StoredPhysicsConfig(PhysicsConfig::default()));
    app.init_resource::<GrabFsms>();
    app.add_systems(PostUpdate, grab::handle_grab_cmds.after(PhysicsSet::Writeback));
    let (grab, rx) = mpsc::unbounded_channel();
    app.insert_resource(GrabCmdRx(std::sync::Mutex::new(rx)));
    let shell = app.world_mut().spawn((Transform::default(), RigidBody::Fixed)).id();
    let mut registry = PropRegistry::default();
    let mut drawer = |app: &mut App, id: &str, y: f32| {
        let rail = PrismaticJointBuilder::new(Vec3::X).limits([0., 0.4])
            .local_anchor1(Vec3::Y * y).build();
        let entity = app.world_mut().spawn((Transform::from_xyz(0., y, 0.),
            RigidBody::Dynamic, Collider::cuboid(0.2, 0.08, 0.3), ColliderMassProperties::Mass(5.),
            CollisionGroups::new(Group::GROUP_1, Group::ALL & !Group::GROUP_32),
            ImpulseJoint::new(shell, rail), props::HandleExtent(Vec3::new(0.005, 0.005, 0.2)),
            Prop { id: id.into(), loading: false,
                grasp_local: Some(Mat4::from_translation(Vec3::X * 0.2).to_cols_array_2d()) })).id();
        registry.by_id.insert(id.into(), entity);
        entity
    };
    let upper = drawer(&mut app, "upper", 0.8);
    let lower = drawer(&mut app, "lower", 0.63);
    app.insert_resource(registry);
    let wrist = app.world_mut().spawn((Transform::from_xyz(0.219, 0.8, 0.),
        RigidBody::KinematicPositionBased, Collider::ball(0.02), BodyBoneCollider { smpl_idx: 21 },
        CollisionGroups::new(Group::GROUP_32, Group::ALL & !Group::GROUP_32))).id();
    for _ in 0..4 { app.update(); }
    grab.send(GrabOrRelease::Grab(GrabCmd { hand: Hand::Right, target: "upper".into(),
        palm_offset_override: None })).unwrap();
    for _ in 0..4 { app.update(); }
    assert!(matches!(app.world().resource::<GrabFsms>().right, GrabState::Attached { .. }));
    (app, grab, wrist, upper, lower)
}

#[test]
fn a_gripped_drawer_follows_the_hand_on_its_rail_and_the_hand_slips_off_pulling_past_the_stop() {
    let (mut app, _grab, wrist, upper, lower) = drawers();
    let travel = |app: &App, drawer: Entity| app.world().get::<Transform>(drawer).unwrap().translation.x;
    // Pull out along the rail, the wrist turning, bobbing and sliding 8 cm along the bar as a hand does.
    for frame in 1..=60 {
        let mut pose = app.world_mut().get_mut::<Transform>(wrist).unwrap();
        pose.translation = Vec3::new(0.219 + 0.005 * frame as f32, 0.8 + 0.01 * (frame as f32 * 0.3).sin(),
                                     0.08 * frame as f32 / 60.);
        pose.rotation = Quat::from_rotation_y(0.3 * (frame as f32 * 0.2).sin());
        app.update();
    }
    for _ in 0..10 { app.update(); }
    let pulled = travel(&app, upper);
    assert!((pulled - 0.3).abs() < 0.02, "drawer at {pulled} m after a 0.30 m pull");
    let pose = app.world().get::<Transform>(upper).unwrap();
    assert!(pose.rotation.angle_between(Quat::IDENTITY) < 0.01 && (pose.translation.y - 0.8).abs() < 0.005);
    assert!(matches!(app.world().resource::<GrabFsms>().right, GrabState::Attached { .. }));
    // Pushing it closed and on 15 cm into it: pressing at its stop, the hand keeps hold.
    for frame in 1..=100 {
        app.world_mut().get_mut::<Transform>(wrist).unwrap().translation.x = 0.519 - 0.0045 * frame as f32;
        app.update();
    }
    // Resting at its stop (long enough to fall asleep), it still comes out with the hand.
    for _ in 0..180 { app.update(); }
    assert!(travel(&app, upper).abs() < 0.01, "drawer not closed: {}", travel(&app, upper));
    assert!(matches!(app.world().resource::<GrabFsms>().right, GrabState::Attached { .. }));
    // Out again to 0.30 m.
    for frame in 1..=100 {
        app.world_mut().get_mut::<Transform>(wrist).unwrap().translation.x = 0.069 + 0.0045 * frame as f32;
        app.update();
    }
    for _ in 0..10 { app.update(); }
    assert!((travel(&app, upper) - 0.3).abs() < 0.02, "drawer at {} m", travel(&app, upper));
    // Pull on past the 0.40 m stop: the drawer stays at it and the hand lets go out of reach of the bar.
    for frame in 61..=120 {
        app.world_mut().get_mut::<Transform>(wrist).unwrap().translation.x = 0.219 + 0.005 * frame as f32;
        app.update();
        assert!(travel(&app, upper) < 0.41, "drawer pulled past its stop: {}", travel(&app, upper));
    }
    assert!(travel(&app, upper) > 0.39, "drawer short of its stop: {}", travel(&app, upper));
    assert!(matches!(app.world().resource::<GrabFsms>().right, GrabState::Idle));
    let failures = &app.world().resource::<PropRegistry>().pending_failures;
    assert!(failures.iter().any(|f| f.reason == "grip_slipped" && f.target == "upper"), "{failures:?}");
    let world = app.world_mut();
    assert_eq!(world.query_filtered::<Entity, With<ContactGrip>>().iter(world).count(), 0);
    assert!(travel(&app, lower).abs() < 0.005, "lower drawer moved to {}", travel(&app, lower));
    // Let go, it may sleep again.
    app.update();
    assert_eq!(app.world().get::<Sleeping>(upper).map(|s| s.normalized_linear_threshold), Some(Sleeping::default().normalized_linear_threshold));
}

#[test]
fn a_drawer_pressed_shut_stays_shut_when_let_go() {
    let (mut app, grab, wrist, upper, _) = drawers();
    let travel = |app: &App| app.world().get::<Transform>(upper).unwrap().translation.x;
    for frame in 1..=40 {
        app.world_mut().get_mut::<Transform>(wrist).unwrap().translation.x = 0.219 - 0.004 * frame as f32;
        app.update();
    }
    grab.send(GrabOrRelease::Release(Hand::Right)).unwrap();
    for _ in 0..60 { app.update(); }
    assert!(travel(&app).abs() < 0.01, "drawer kicked open to {}", travel(&app));
}

#[test]
fn a_hand_sweeping_past_a_drawer_handle_does_not_grip_it_until_at_rest() {
    let (mut app, grab, wrist, _, _) = drawers();
    grab.send(GrabOrRelease::Release(Hand::Right)).unwrap();
    app.update();
    grab.send(GrabOrRelease::Grab(GrabCmd { hand: Hand::Right, target: "upper".into(),
        palm_offset_override: None })).unwrap();
    // Down across the drawer front at 0.6 m/s, in contact all the way.
    for frame in 0..8 {
        app.world_mut().get_mut::<Transform>(wrist).unwrap().translation.y = 0.84 - 0.01 * frame as f32;
        app.update();
        assert!(matches!(app.world().resource::<GrabFsms>().right, GrabState::Pursuing { .. }), "frame {frame}");
    }
    for _ in 0..3 { app.update(); }
    assert!(matches!(app.world().resource::<GrabFsms>().right, GrabState::Attached { .. }));
}

#[test]
fn a_hand_resting_on_the_handle_grips_it_without_touching_the_drawer() {
    for (x, grips) in [(0.26_f32, true), (0.29, false)] {
        let (mut app, grab, wrist, _, _) = drawers();
        grab.send(GrabOrRelease::Release(Hand::Right)).unwrap();
        app.update();
        app.world_mut().get_mut::<Transform>(wrist).unwrap().translation.x = x;
        for _ in 0..3 { app.update(); }
        grab.send(GrabOrRelease::Grab(GrabCmd { hand: Hand::Right, target: "upper".into(),
            palm_offset_override: None })).unwrap();
        for _ in 0..4 { app.update(); }
        let attached = matches!(app.world().resource::<GrabFsms>().right, GrabState::Attached { .. });
        assert_eq!(attached, grips, "wrist {:.3} m from the bar", x - 0.205 - 0.02);
    }
}
