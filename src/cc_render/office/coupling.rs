//! Declared-interaction coupling: attached objects follow the anchor hands.
//!
//! Pure pose maths (`coupled_pose`) is separate from the Bevy system so it can be
//! tested. Objects are posed kinematically from observed wrists; nothing here
//! moves the avatar or edits generated motion.
use super::controller::{CouplingLatch, OfficeController};
use super::super::bevy_app::physics::props::{Prop, PropRegistry};
use super::super::bevy_app::pose::CC5BindRotations;
use super::super::renderer::Hand;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

/// Twist angle of `q` about the unit axis `axis` (swing-twist decomposition).
fn twist_angle(q: Quat, axis: Vec3) -> f32 {
    let v = Vec3::new(q.x, q.y, q.z);
    2. * v.dot(axis).atan2(q.w)
}

/// Least-squares rotation taking `local` points onto `world` points (Horn's
/// quaternion method, dominant eigenvector by shifted power iteration).
fn fit_rotation(local: &[Vec3], world: &[Vec3]) -> Quat {
    let (cl, cw) = (local.iter().sum::<Vec3>() / local.len() as f32, world.iter().sum::<Vec3>() / world.len() as f32);
    let mut s = [[0f32; 3]; 3];
    for (a, b) in local.iter().zip(world) {
        let (a, b) = (*a - cl, *b - cw);
        for i in 0..3 { for j in 0..3 { s[i][j] += a[i] * b[j]; } }
    }
    let (sxx, sxy, sxz, syx, syy, syz, szx, szy, szz) = (s[0][0], s[0][1], s[0][2], s[1][0], s[1][1], s[1][2], s[2][0], s[2][1], s[2][2]);
    let n = [[sxx + syy + szz, syz - szy, szx - sxz, sxy - syx],
             [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
             [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
             [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz]];
    let shift = n.iter().flatten().map(|v| v.abs()).sum::<f32>() + 1.;
    let mut q = [1f32, 0., 0., 0.];
    for _ in 0..64 {
        let mut next = [0f32; 4];
        for i in 0..4 { next[i] = shift * q[i] + (0..4).map(|j| n[i][j] * q[j]).sum::<f32>(); }
        let norm = next.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
        q = next.map(|v| v / norm);
    }
    Quat::from_xyzw(q[1], q[2], q[3], q[0]).normalize()
}

/// Grips closer than this on the object cannot steer it; the primary hand holds it rigidly.
pub const MIN_TWO_HAND_SPAN_M: f32 = 0.2;

/// Object pose from anchor wrists `(position, rotation, object-local grip point)`,
/// primary first. One anchor (or two grips closer than [`MIN_TWO_HAND_SPAN_M`]):
/// rigid to the primary hand from the latch. Two: the grip line follows the line
/// between the hands as an unoriented axis (minimal swing from the current
/// rotation, never flipping the object end over end) and roll follows the
/// primary hand's twist; the secondary hand may slide along its grip. Three or
/// more: least-squares rigid fit. `slide` keeps the object at its latch height
/// with yaw-only rotation.
pub fn coupled_pose(anchors: &[(Vec3, Quat, Vec3)], current: Quat, latch: &mut CouplingLatch,
                    slide: bool) -> Transform {
    let (p0, q0, g0) = anchors[0];
    let rigid = anchors.len() == 1 || (anchors.len() == 2 && anchors[1].2.distance(g0) < MIN_TWO_HAND_SPAN_M);
    let mut rotation = match anchors.len() {
        _ if rigid => q0 * latch.primary_rotation.inverse() * latch.object_rotation,
        2 => {
            let (p1, _, g1) = anchors[1];
            let (local, world) = (current * (g1 - g0), p1 - p0);
            let mut r = current;
            if local.length() > 1e-4 && world.length() > 1e-4 {
                let axis = world.normalize() * world.dot(local).signum();
                r = Quat::from_rotation_arc(local.normalize(), axis) * r;
                let twist = twist_angle(q0 * latch.previous_primary.inverse(), axis);
                r = Quat::from_axis_angle(axis, twist) * r;
            }
            r.normalize()
        }
        _ => fit_rotation(&anchors.iter().map(|a| a.2).collect::<Vec<_>>(),
                          &anchors.iter().map(|a| a.0).collect::<Vec<_>>()),
    };
    latch.previous_primary = q0;
    if slide {
        let (yaw, _, _) = rotation.to_euler(EulerRot::YXZ);
        rotation = Quat::from_rotation_y(yaw);
    }
    let mut translation = if anchors.len() > 2 {
        let cw = anchors.iter().map(|a| a.0).sum::<Vec3>() / anchors.len() as f32;
        let cl = anchors.iter().map(|a| a.2).sum::<Vec3>() / anchors.len() as f32;
        cw - rotation * cl
    } else {
        p0 - rotation * g0
    };
    if slide { translation.y = latch.object_height; }
    Transform::from_translation(translation).with_rotation(rotation)
}

fn hand_transform(bind: &CC5BindRotations, globals: &Query<&GlobalTransform>, hand: Hand) -> Option<Transform> {
    bind.entities[if hand == Hand::Left { 20 } else { 21 }]
        .and_then(|e| globals.get(e).ok()).map(|g| g.compute_transform())
}

/// Lowest object-local y of the object's solids (its resting surface).
fn lowest_local_y(object: &super::OfficeObject) -> f32 {
    if object.parts.is_empty() { return -object.half_extents[1]; }
    object.parts.iter().map(|p| p.position[1] - p.half_extents[1]).fold(f32::INFINITY, f32::min)
}

pub(crate) fn apply_couplings(
    mut commands: Commands,
    mut office: ResMut<OfficeController>,
    bind: Res<CC5BindRotations>,
    globals: Query<&GlobalTransform>,
    registry: Res<PropRegistry>,
    mut props: Query<(&mut Transform, &mut Velocity), With<Prop>>,
) {
    office.remember_homes();
    if office.couplings.is_empty() { return; }
    let (objects, homes) = (office.objects.clone(), office.homes.clone());
    let held: Vec<String> = office.couplings.iter().map(|c| c.object.clone()).collect();
    let solids = super::support::boxes(&objects.iter().filter(|o| !held.contains(&o.id)).cloned().collect::<Vec<_>>());
    let mut released = Vec::new();
    let mut error = None;
    for coupling in office.couplings.iter_mut() {
        let Some(&entity) = registry.by_id.get(&coupling.object) else { continue };
        let Ok((mut transform, mut velocity)) = props.get_mut(entity) else { continue };
        let Some(object) = objects.iter().find(|o| o.id == coupling.object) else { continue };
        if let Some(onto) = coupling.release.clone() {
            let onto_home = onto.is_none();
            let pose = match onto {
                None => homes.get(&coupling.object).map(|h| Transform::from_translation(Vec3::new(h[0], h[1], h[2]))
                    .with_rotation(Quat::from_xyzw(h[3], h[4], h[5], h[6]))),
                Some(reference) => super::interaction::resolve_zone(object, &reference, &objects, &homes).map(|(zone, t)| {
                    let surface = t * Vec3::from(zone.center.unwrap_or_default());
                    let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
                    Transform::from_translation(surface - Vec3::Y * lowest_local_y(object) + Vec3::Y * 0.002)
                        .with_rotation(Quat::from_rotation_y(yaw))
                }),
            };
            if let Some(pose) = pose { *transform = pose; }
            *velocity = Velocity::zero();
            // Put back where it belongs, it stays put; set down onto a surface, physics takes over.
            commands.entity(entity).insert(if onto_home { RigidBody::Fixed } else { RigidBody::Dynamic });
            released.push(coupling.object.clone());
            continue;
        }
        let anchors: Vec<(Vec3, Quat, Vec3)> = coupling.anchors.iter()
            .filter_map(|(hand, grip, _)| hand_transform(&bind, &globals, *hand).map(|w| (w.translation, w.rotation, Vec3::from(*grip))))
            .collect();
        if anchors.len() != coupling.anchors.len() { continue; }
        let latch = coupling.latch.get_or_insert(CouplingLatch {
            object_rotation: transform.rotation, primary_rotation: anchors[0].1,
            previous_primary: anchors[0].1, object_height: transform.translation.y });
        let previous = *transform;
        let desired = coupled_pose(&anchors, transform.rotation, latch, coupling.slide_height.is_some());
        // Held objects do not pass through furniture or walls: when the hands would push
        // one through, it keeps its rotation and follows the primary hand, else it stops.
        // Only a deep penetration that cannot be avoided is an error.
        let kept = Transform::from_translation(anchors[0].0 - previous.rotation * anchors[0].2).with_rotation(previous.rotation);
        *transform = match [desired, kept].into_iter().find(|pose| held_hit(object, pose, &solids, HELD_CONTACT_M).is_none()) {
            Some(pose) => pose,
            None => {
                if let Some(owner) = held_hit(object, &previous, &solids, HELD_PENETRATION_M) {
                    error.get_or_insert(format!("extreme_deformation: held {} intersects {owner}", coupling.object));
                }
                previous
            }
        };
        *velocity = Velocity::zero();
        commands.entity(entity).insert(RigidBody::KinematicPositionBased);
    }
    office.couplings.retain(|c| !released.contains(&c.object));
    if office.motion_error.is_none() && error.is_some() { office.motion_error = error; }
}

/// Contact a held object may not make (m into another solid), and the depth that is an error.
pub const HELD_CONTACT_M: f32 = 0.005;
pub const HELD_PENETRATION_M: f32 = 0.05;

/// The other object a held object's solids pass more than `depth` into at `pose`.
fn held_hit<'a>(object: &super::OfficeObject, pose: &Transform, solids: &'a [super::support::ColliderBox],
                depth: f32) -> Option<&'a str> {
    let mut object = object.clone();
    object.position = pose.translation.to_array();
    object.rotation_xyzw = Some(pose.rotation.to_array());
    let edges = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    super::support::boxes(std::slice::from_ref(&object)).iter().find_map(|part| {
        let corner = |i: usize| part.center + part.rotation * Vec3::new(
            if i & 1 == 0 { -part.half.x } else { part.half.x },
            if i & 2 == 0 { -part.half.y } else { part.half.y },
            if i & 4 == 0 { -part.half.z } else { part.half.z });
        solids.iter().find(|s| edges.iter().any(|(a, b)| s.intersects(corner(*a), corner(*b), -depth)))
            .map(|s| s.owner.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latch(object: Quat, hand: Quat) -> CouplingLatch {
        CouplingLatch { object_rotation: object, primary_rotation: hand, previous_primary: hand, object_height: 0.1 }
    }

    #[test]
    fn two_hand_pose_follows_the_hand_line_and_ignores_a_wrist_flip() {
        // A broom-like grip line along local +X between two grips 0.5 m apart.
        let (g0, g1) = (Vec3::new(0.3, 0., 0.), Vec3::new(-0.2, 0., 0.));
        let mut l = latch(Quat::IDENTITY, Quat::IDENTITY);
        let hands = |p0: Vec3, p1: Vec3, q0: Quat| [(p0, q0, g0), (p1, Quat::IDENTITY, g1)];
        let pose = coupled_pose(&hands(Vec3::new(0., 1., 0.), Vec3::new(0., 0.5, 0.), Quat::IDENTITY), Quat::IDENTITY, &mut l, false);
        // Primary grip sits in the primary hand; the line g0->g1 points from hand 0 to hand 1.
        assert!((pose * g0).distance(Vec3::new(0., 1., 0.)) < 1e-4);
        assert!(((pose.rotation * (g1 - g0)).normalize() - Vec3::new(0., -1., 0.)).length() < 1e-4);
        // A 169-degree flip of the primary wrist about an axis across the line barely changes the pose:
        // only its twist component about the grip line is followed.
        let flip = Quat::from_rotation_z(169f32.to_radians());
        let flipped = coupled_pose(&hands(Vec3::new(0., 1., 0.), Vec3::new(0., 0.5, 0.), flip), pose.rotation, &mut l, false);
        assert!(((flipped.rotation * (g1 - g0)).normalize() - Vec3::new(0., -1., 0.)).length() < 1e-4);
        // The hands swapping height tilts the line through horizontal but never turns it end over end.
        let swapped = coupled_pose(&hands(Vec3::new(0., 1., 0.), Vec3::new(0.05, 1.5, 0.), flip), flipped.rotation, &mut l, false);
        assert!((swapped.rotation * (g1 - g0)).normalize().y < -0.99);
    }

    #[test]
    fn grips_too_close_together_hold_the_object_rigidly_in_the_primary_hand() {
        let mut l = latch(Quat::IDENTITY, Quat::IDENTITY);
        let (g0, g1) = (Vec3::new(0., 0.19, 0.), Vec3::new(0., 0.135, 0.));
        let pose = coupled_pose(&[(Vec3::new(1., 1., 1.), Quat::IDENTITY, g0), (Vec3::new(1.5, 1., 1.), Quat::IDENTITY, g1)],
                                Quat::IDENTITY, &mut l, false);
        assert!(pose.rotation.dot(Quat::IDENTITY).abs() > 0.9999, "the far-off second wrist does not swing it");
        assert!((pose * g0).distance(Vec3::new(1., 1., 1.)) < 1e-4);
    }

    #[test]
    fn one_hand_is_rigid_and_three_hands_fit_least_squares() {
        let mut l = latch(Quat::IDENTITY, Quat::IDENTITY);
        let turn = Quat::from_rotation_y(0.7);
        let pose = coupled_pose(&[(Vec3::new(1., 1., 1.), turn, Vec3::new(0.1, 0., 0.))], Quat::IDENTITY, &mut l, false);
        assert!((pose.rotation.dot(turn).abs() - 1.).abs() < 1e-5);
        assert!((pose * Vec3::new(0.1, 0., 0.)).distance(Vec3::new(1., 1., 1.)) < 1e-4);
        let local = [Vec3::new(0.2, 0., 0.), Vec3::new(-0.2, 0., 0.), Vec3::new(0., 0.3, 0.)];
        let r = Quat::from_rotation_x(0.4) * Quat::from_rotation_y(-0.9);
        let world: Vec<_> = local.iter().map(|g| r * *g + Vec3::new(2., 0.5, -1.)).collect();
        let anchors: Vec<_> = local.iter().zip(&world).map(|(g, w)| (*w, Quat::IDENTITY, *g)).collect();
        let fit = coupled_pose(&anchors, Quat::IDENTITY, &mut l, false);
        assert!(fit.rotation.dot(r).abs() > 0.9999, "{:?} vs {:?}", fit.rotation, r);
        assert!(fit.translation.distance(Vec3::new(2., 0.5, -1.)) < 1e-3);
    }

    #[test]
    fn held_contact_is_detected_and_only_deep_penetration_is_an_error_depth() {
        let objects = super::super::starter_objects();
        let broom = objects.iter().find(|o| o.id == "broom").unwrap();
        let walls: Vec<_> = objects.iter().filter(|o| o.id == "wall_front").cloned().collect();
        let solids = super::super::support::boxes(&walls);
        let at = |z: f32| Transform::from_translation(Vec3::new(-1.6, 0.66, z));
        assert!(held_hit(broom, &at(2.45), &solids, HELD_CONTACT_M).is_none());
        // Lying towards the (low) front wall: the handle tip reaches 0.19 m into it.
        let tipped = Transform::from_translation(Vec3::new(-1.6, 0.2, 2.45)).with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2));
        assert_eq!(held_hit(broom, &tipped, &solids, HELD_CONTACT_M), Some("wall_front"));
        let grazing = Transform::from_translation(Vec3::new(-1.6, 0.25, 2.92 - 0.66 + 0.02))
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2));
        assert!(held_hit(broom, &grazing, &solids, HELD_CONTACT_M).is_some());
        assert!(held_hit(broom, &grazing, &solids, HELD_PENETRATION_M).is_none(), "2 cm of contact is tolerated");
    }

    #[test]
    fn sliding_keeps_height_and_yaw_only() {
        let mut l = latch(Quat::IDENTITY, Quat::IDENTITY);
        let tilt = Quat::from_rotation_y(0.5) * Quat::from_rotation_x(0.6);
        let pose = coupled_pose(&[(Vec3::new(0.4, 0.9, 0.2), tilt, Vec3::ZERO)], Quat::IDENTITY, &mut l, true);
        assert_eq!(pose.translation.y, 0.1);
        let up = pose.rotation * Vec3::Y;
        assert!((up - Vec3::Y).length() < 1e-5, "yaw only");
    }
}
