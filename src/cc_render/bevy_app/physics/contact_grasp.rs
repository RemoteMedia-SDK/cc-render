//! Generic ideal rigid grip, admitted only on intersecting physics shapes.
//! Captures existing relative frames: no object repositioning, canonical grip
//! rotation, body target, or distance-based object follower. It does not model
//! finger closure; only a grip on a jointed part is force limited.
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

/// A hand on a jointed part (a drawer on its rail, a door on its hinge) pulls with at most this
/// force per axis: the part's own joint keeps its limits and orientation.
pub(super) const ARTICULATED_GRIP_MAX_FORCE_N: f32 = 150.;
/// A hand that drifts further than this from where it gripped a part, out from it or off to the
/// side (pulling on past its stop, say), has slipped off and lets go. Sliding along a handle bar
/// or pressing into the part is not drift. Generated hands sway several centimetres about a
/// held bar, and nothing repositions them.
pub(super) const ARTICULATED_GRIP_SLIP_M: f32 = 0.12;
/// A hand moving faster than this is sweeping past a jointed part's handle, not closing on it.
pub(super) const ARTICULATED_GRIP_MAX_SPEED_M_S: f32 = 0.25;

/// The grip on a jointed part, captured where the wrist touches it: a force-limited spring at the
/// wrist, free to turn. A kinematic hand with a rigid grip would overpower the part's joint and
/// drag it past its limits. Returns the joint and the grip point in the part's frame.
pub(super) fn capture_articulated_joint(
    wrist: &GlobalTransform,
    object: &GlobalTransform,
    active_contact: bool,
) -> Option<(GenericJoint, Vec3)> {
    if !active_contact { return None; }
    let point = object.affine().inverse().transform_point3(wrist.translation());
    if !point.is_finite() { return None; }
    let mut joint = GenericJointBuilder::new(JointAxesMask::empty())
        .local_anchor1(Vec3::ZERO)
        .local_anchor2(point)
        .build();
    for axis in [JointAxis::LinX, JointAxis::LinY, JointAxis::LinZ] {
        // Stiff and lightly damped: it closes nearly all of the gap each physics step, as tight
        // as a rigid grip until the force limit.
        joint.set_motor_position(axis, 0., 1.0e4, 10.)
            .set_motor_max_force(axis, ARTICULATED_GRIP_MAX_FORCE_N);
    }
    joint.set_contacts_enabled(false);
    Some((joint, point))
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

/// A hand is on a handle bar when its collider comes this close to the declared bar, which is
/// not itself a collision shape: the wrist collider is a 5 cm stub, and the palm and fingers
/// that close around the bar reach on beyond it.
pub(super) const HANDLE_WRAP_M: f32 = 0.04;

/// The hand's collider within HANDLE_WRAP_M of a part's handle box (half extents `extent`,
/// centred at `centre` in the part's frame).
pub(super) fn has_handle_contact(context: &RapierContext, wrist: Entity, object: &GlobalTransform,
                                 centre: Vec3, extent: Vec3) -> bool {
    let Some(hand) = context.entity2collider().get(&wrist)
        .and_then(|handle| context.colliders.get(*handle)) else { return false; };
    if hand.is_sensor() { return false; }
    let object = object.compute_transform();
    let pose = bevy_rapier3d::rapier::math::Isometry::from_parts(
        object.transform_point(centre).into(), object.rotation.into());
    let bar = bevy_rapier3d::rapier::parry::shape::Cuboid::new(extent.into());
    bevy_rapier3d::rapier::parry::query::contact(hand.position(), hand.shape(), &pose, &bar, HANDLE_WRAP_M)
        .ok().flatten().is_some_and(|contact| contact.dist <= HANDLE_WRAP_M)
}

/// A wrist on a handle bar sits within this of it: the palm's grip point is about 6.5 cm along
/// the fingers and 2.7 cm off the palm (SDK `GRIP_REACH_M`, `GRIP_LIFT_M`), and the fingers,
/// some 18 cm from the wrist to their tips, still close around a bar a few cm further out
/// (generated wrists hold their planned grasp to within about 4 cm).
pub(super) const HANDLE_REACH_M: f32 = 0.12;

/// While the hand actually touches the target (a prop with a handle box: the handle): the point
/// its reach is judged to, and how far that may be. A prop with a handle box: the nearest point on the bar, within
/// HANDLE_REACH_M; else its grasp anchor, within the configured proximity.
pub(super) fn target_position(
    context: Option<&RapierContext>,
    wrist: Option<(Entity, Vec3)>,
    target: Option<(Entity, &super::props::Prop, &GlobalTransform, Option<&super::props::HandleExtent>)>,
    proximity_m: f32,
) -> Option<(Vec3, f32)> {
    let context = context?;
    let (wrist, wrist_position) = wrist?;
    let (object, prop, transform, handle) = target?;
    log_pair(context, wrist, object);
    let touching = match handle {
        Some(extent) => has_handle_contact(context, wrist, transform, prop.grasp_offset(), extent.0),
        None => has_object_contact(context, wrist, object),
    };
    if !touching { return None; }
    Some(match handle {
        Some(extent) => (nearest_on_handle(transform, prop.grasp_offset(), extent.0, wrist_position), HANDLE_REACH_M),
        None => (prop.grasp_world(transform), proximity_m),
    })
}

/// Nearest point to `point` on a handle box (half extents `extent`, centred at `centre` in
/// prop space).
pub(super) fn nearest_on_handle(transform: &GlobalTransform, centre: Vec3, extent: Vec3, point: Vec3) -> Vec3 {
    let matrix = transform.compute_matrix();
    let local = matrix.inverse().transform_point3(point) - centre;
    matrix.transform_point3(centre + local.clamp(-extent, extent))
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

/// Check both ECS ownership and backend registration in contact fixtures.
#[cfg(test)]
fn assert_right_grip(app: &App, object: Entity, wrist: Entity, attached: bool) -> Option<Entity> {
    use super::grab::{GrabFsms, GrabState};
    let grip = match app.world().resource::<GrabFsms>().right {
        GrabState::Attached { grip_entity, .. } => Some(grip_entity),
        _ => None,
    };
    assert_eq!(grip.is_some(), attached);
    assert!(app.world().get::<ImpulseJoint>(object).is_none());
    if let Some(entity) = grip {
        assert_eq!(app.world().get::<Parent>(entity).unwrap().get(), object);
        assert_eq!(app.world().get::<ImpulseJoint>(entity).unwrap().parent, wrist);
        assert!(app.world().get::<RapierImpulseJointHandle>(entity).is_some());
    }
    grip
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hand_on_a_long_bar_is_judged_to_the_bar_not_its_centre() {
        // A 90 cm bar along Z at the front of a drawer, the drawer turned 90 degrees.
        let drawer = GlobalTransform::from(Transform::from_xyz(1., 0.8, 0.).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)));
        let (centre, extent) = (Vec3::new(0.3, 0.1, 0.), Vec3::new(0.006, 0.006, 0.45));
        let world_centre = drawer.transform_point(centre);
        let along = drawer.transform_point(centre + Vec3::new(0., 0., 0.35));
        let wrist = along + Vec3::new(0., 0.07, 0.);  // above the bar, 35 cm from its middle
        let nearest = nearest_on_handle(&drawer, centre, extent, wrist);
        assert!(nearest.distance(along) < 0.007, "{nearest} vs {along}");
        assert!(wrist.distance(nearest) <= HANDLE_REACH_M);
        assert!(wrist.distance(world_centre) > HANDLE_REACH_M);
    }

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
            let grip = assert_right_grip(&app, mug, wrist, attached);
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
                assert!(app.world().get_entity(grip.unwrap()).is_err());
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
