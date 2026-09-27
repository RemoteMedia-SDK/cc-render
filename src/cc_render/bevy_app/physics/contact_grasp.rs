//! Generic ideal rigid grip, admitted only on intersecting physics shapes.
//! Captures existing relative frames: no object repositioning, canonical grip
//! rotation, body target, or distance-based object follower. This models neither
//! finger closure nor human grip-force limits.
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

pub(super) fn capture_joint(
    wrist: &GlobalTransform,
    object: &GlobalTransform,
    active_contact: bool,
) -> Option<FixedJoint> {
    if !active_contact {
        return None;
    }
    let wrist = wrist.compute_transform();
    let object = object.compute_transform();
    if !wrist.translation.is_finite() || !object.translation.is_finite()
        || !wrist.rotation.is_finite() || !object.rotation.is_finite()
    {
        return None;
    }
    let inverse = wrist.rotation.inverse();
    let mut joint = FixedJointBuilder::new()
        .local_anchor1(inverse * (object.translation - wrist.translation))
        .local_anchor2(Vec3::ZERO)
        .local_basis1(inverse * object.rotation)
        .local_basis2(Quat::IDENTITY)
        .build();
    // The captured pair is one rigid grip; other collisions remain active.
    joint.data.set_contacts_enabled(false);
    Some(joint)
}

pub(super) fn has_object_contact(context: &RapierContext, wrist: Entity, object: Entity) -> bool {
    let Some(hand) = context.entity2collider().get(&wrist)
        .and_then(|handle| context.colliders.get(*handle)) else { return false; };
    let Some(body) = context.entity2body().get(&object)
        .and_then(|handle| context.bodies.get(*handle)) else { return false; };
    if hand.is_sensor() || !body.is_dynamic() { return false; }
    // Office props suppress animation-driven impulses before a verified grip.
    // Test the actual collision shapes, not filtered solver-contact reports.
    // Zero prediction distance: a nearby but noncontacting hand cannot capture.
    body.colliders().iter().filter_map(|handle| context.colliders.get(*handle))
        .filter(|collider| !collider.is_sensor()).any(|collider| {
        bevy_rapier3d::rapier::parry::query::contact(
            hand.position(), hand.shape(), collider.position(), collider.shape(), 0.0)
            .ok().flatten().is_some_and(|contact| contact.dist <= 0.0)
    })
}

pub(super) fn target_position(
    context: Option<&RapierContext>,
    wrist: Option<Entity>,
    target: Option<(Entity, &super::props::Prop, &GlobalTransform)>,
) -> Option<Vec3> {
    let context = context?;
    let wrist = wrist?;
    let (object, prop, transform) = target?;
    log_pair(context, wrist, object);
    has_object_contact(context, wrist, object).then(|| prop.grasp_world(transform))
}

pub(super) fn log_pair(context: &RapierContext, wrist: Entity, object: Entity) {
    if std::env::var("CC_GRASP_DIAGNOSTICS").as_deref() != Ok("1") { return; }
    let pair = context.entity2collider().get(&wrist).and_then(|hand| {
        let object_collider = context.entity2collider().iter().find(|(entity, _)|
            **entity == object || context.collider_parent(**entity) == Some(object))?.1;
        Some((context.colliders.get(*hand)?, context.colliders.get(*object_collider)?))
    });
    if pair.is_none() {
        tracing::info!(target:"cc_render", hand_mapped=context.entity2collider().contains_key(&wrist),
            object_mapped=context.entity2collider().contains_key(&object),
            colliders=context.entity2collider().len(), "physical grip missing collider mapping");
    }
    if let Some((hand, object)) = pair {
        let contact = bevy_rapier3d::rapier::parry::query::contact(
            hand.position(), hand.shape(), object.position(), object.shape(), 1.0)
            .ok().flatten().map(|contact| contact.dist);
        tracing::info!(target:"cc_render", ?contact,
            hand_position=?hand.position().translation.vector,
            hand_aabb=?hand.compute_aabb(), object_aabb=?object.compute_aabb(),
            "physical grip collider diagnostic");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecs_grasp_requires_contact_with_requested_prop() {
        use super::super::{body_colliders::BodyBoneCollider, grab::{handle_grab_cmds, GrabFsms, GrabState},
            props::{Prop, PropRegistry, GrabCmdRx}, StoredPhysicsConfig};
        use crate::cc_render::renderer::{PhysicsConfig, GrabOrRelease, Hand};
        for (distance, compound) in [(0.045_f32, false), (0.031, false), (0.045, true), (0.031, true)] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin));
            app.init_resource::<Assets<Mesh>>();
            app.init_resource::<SceneSpawner>();
            app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default());
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(1. / 60.)));
            app.insert_resource(StoredPhysicsConfig(PhysicsConfig::default()));
            app.insert_resource(GrabFsms::default());
            app.add_systems(Update, handle_grab_cmds);
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            app.insert_resource(GrabCmdRx(std::sync::Mutex::new(rx)));
            let wrist = app.world_mut().spawn((Transform::from_xyz(0., 1., 0.),
                RigidBody::KinematicPositionBased, Collider::ball(0.02), BodyBoneCollider { smpl_idx:21 },
                CollisionGroups::new(Group::GROUP_32, Group::ALL))).id();
            let mug = app.world_mut().spawn((Transform::from_xyz(distance, 1., 0.),
                RigidBody::Dynamic, GravityScale(0.),
                Prop { id:"mug".into(), loading:false, grasp_local:None })).id();
            if compound {
                app.world_mut().spawn((Transform::default(), Collider::ball(0.02),
                    CollisionGroups::new(Group::GROUP_1, Group::ALL & !Group::GROUP_32))).set_parent(mug);
            } else {
                app.world_mut().entity_mut(mug).insert((Collider::ball(0.02),
                    CollisionGroups::new(Group::GROUP_1, Group::ALL & !Group::GROUP_32)));
            }
            // A contact with this unrelated prop must not capture the requested mug.
            app.world_mut().spawn((Transform::from_xyz(-0.031, 1., 0.),
                RigidBody::Dynamic, GravityScale(0.), Collider::ball(0.02),
                Prop { id:"other".into(), loading:false, grasp_local:None }));
            let mut registry = PropRegistry::default();
            registry.by_id.insert("mug".into(), mug);
            app.insert_resource(registry);
            app.world_mut().resource_mut::<GrabFsms>().right = GrabState::Pursuing {
                target:"mug".into(), elapsed:0., palm_override:None };
            for _ in 0..10 { app.update(); }
            let attached = matches!(app.world().resource::<GrabFsms>().right, GrabState::Attached {..});
            assert_eq!(attached, distance < 0.04, "distance={distance}");
            assert_eq!(app.world().get::<ImpulseJoint>(mug).is_some(), attached);
            if attached {
                let start = app.world().get::<Transform>(mug).unwrap().translation;
                // Synthetic kinematic drive tests physics only, not model behavior.
                for _ in 0..20 {
                    app.world_mut().get_mut::<Transform>(wrist).unwrap().translation.x += 0.005;
                    app.update();
                }
                // Let the ECS transform/physics pipeline consume the last input.
                for _ in 0..3 { app.update(); }
                let displacement = app.world().get::<Transform>(mug).unwrap().translation - start;
                assert!((displacement - Vec3::X * 0.1).length() < 0.003, "{displacement:?}");
                tx.send(GrabOrRelease::Release(Hand::Right)).unwrap();
                app.update();
                assert!(app.world().get::<ImpulseJoint>(mug).is_none());
                assert!(matches!(app.world().resource::<GrabFsms>().right, GrabState::Idle));
            }
        }
    }

    #[test]
    fn proximity_without_actual_contact_cannot_capture() {
        let wrist = GlobalTransform::IDENTITY;
        let object = GlobalTransform::from_translation(Vec3::X * 0.001);
        assert!(capture_joint(&wrist, &object, false).is_none());
    }

    #[test]
    fn contact_capture_preserves_both_world_joint_frames() {
        let wrist = Transform::from_xyz(1.4, 0.9, -1.8)
            .with_rotation(Quat::from_euler(EulerRot::XYZ, 0.4, -0.8, 0.3));
        let object = Transform::from_xyz(1.36, 0.85, -1.79)
            .with_rotation(Quat::from_rotation_y(0.2));
        let joint = capture_joint(&wrist.into(), &object.into(), true).unwrap();
        assert!((wrist.translation + wrist.rotation * joint.data.local_anchor1()
            - object.translation).length() < 1e-6);
        let world_basis = wrist.rotation * joint.data.local_basis1();
        assert!(world_basis.angle_between(object.rotation).abs() < 1e-5);
        assert_eq!(joint.data.local_anchor2(), Vec3::ZERO);
        assert_eq!(joint.data.local_basis2(), Quat::IDENTITY);
    }
}
