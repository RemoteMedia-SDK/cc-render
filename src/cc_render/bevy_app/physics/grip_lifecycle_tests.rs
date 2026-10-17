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
