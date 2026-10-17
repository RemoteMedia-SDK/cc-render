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

/// Hands closer together than this cannot steer the object; the primary hand holds it rigidly
/// until they are [`TWO_HAND_RESUME_M`] apart again (no toggling around one threshold).
pub const MIN_TWO_HAND_SPAN_M: f32 = 0.15;
pub const TWO_HAND_RESUME_M: f32 = 0.25;
/// Frames over which the object blends back to the hand line when the hands spread again.
pub const TWO_HAND_SETTLE_FRAMES: u8 = 8;

/// Object pose from anchor wrists `(position, rotation, object-local grip point)`,
/// primary first. One anchor (or two hands closer than [`MIN_TWO_HAND_SPAN_M`]):
/// rigid to the primary hand from the latch. Two: the grip line follows the line
/// between the hands as an unoriented axis (minimal swing from the current
/// rotation, never flipping the object end over end) and roll follows the
/// primary hand's twist; the secondary hand may slide along its grip. Three or
/// more: least-squares rigid fit. `slide` keeps the object at its latch height
/// with yaw-only rotation.
pub fn coupled_pose(anchors: &[(Vec3, Quat, Vec3)], current: Quat, latch: &mut CouplingLatch,
                    slide: bool) -> Transform {
    let (p0, q0, g0) = anchors[0];
    // The hands' own spacing decides: the secondary hand may slide along its grip, so grips
    // close together on a stick still steer it when the hands are apart. Entering rigid keeps
    // the object where it is relative to the primary hand; leaving it blends back to the line.
    if anchors.len() == 2 {
        let span = anchors[1].0.distance(p0);
        if !latch.rigid && span < MIN_TWO_HAND_SPAN_M {
            latch.rigid = true;
            (latch.primary_rotation, latch.object_rotation) = (q0, current);
        } else if latch.rigid && span > TWO_HAND_RESUME_M {
            latch.rigid = false;
            latch.settle_frames = TWO_HAND_SETTLE_FRAMES;
        }
    }
    let rigid = anchors.len() == 1 || (anchors.len() == 2 && latch.rigid);
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
    if latch.settle_frames > 0 && !rigid {
        latch.settle_frames -= 1;
        rotation = current.slerp(rotation, 0.3).normalize();
    }
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

/// Where on a placement zone (zone-local) an object set down from `local` comes to rest:
/// the nearest point on the zone's surface that keeps an object of footprint radius
/// `radius` inside its edges (the centre line when the zone is smaller than that).
pub fn placement_point(zone: &super::interaction::Zone, local: Vec3, radius: f32) -> Vec3 {
    let center = Vec3::from(zone.center.unwrap_or_default());
    let half = Vec3::from(zone.half_extents.unwrap_or([0.; 3]));
    let room = |h: f32| (h - radius).max(0.);
    Vec3::new(local.x.clamp(center.x - room(half.x), center.x + room(half.x)), center.y,
              local.z.clamp(center.z - room(half.z), center.z + room(half.z)))
}

use super::hand::lowest_local_y;

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_couplings(
    mut commands: Commands,
    mut office: ResMut<OfficeController>,
    bind: Res<CC5BindRotations>,
    globals: Query<&GlobalTransform>,
    registry: Res<PropRegistry>,
    mut props: Query<(&mut Transform, &mut Velocity), (With<Prop>, Without<ImpulseJoint>)>,
    rigs: Option<Res<super::hand::HandRigs>>,
    mut held_in_hand: ResMut<super::hand::HeldInHand>,
    time: Res<Time>,
) {
    office.remember_homes();
    *held_in_hand = super::hand::HeldInHand::default();
    if office.couplings.is_empty() { return; }
    let dt = time.delta_secs();
    let palm = |hand: Hand| rigs.as_ref().and_then(|r| r.get(hand)).map(|c| c.rig.palm);
    let (objects, homes) = (office.objects.clone(), office.homes.clone());
    let held: Vec<String> = office.couplings.iter().map(|c| c.object.clone()).collect();
    let solids = super::support::boxes(&objects.iter().filter(|o| !held.contains(&o.id)).cloned().collect::<Vec<_>>());
    let mut released = Vec::new();
    let mut placed = Vec::new();
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
                    let surface = t * placement_point(&zone, t.rotation.inverse() * (transform.translation - t.translation),
                                                      object.half_extents[0].max(object.half_extents[2]));
                    let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
                    Transform::from_translation(surface - Vec3::Y * lowest_local_y(object) + Vec3::Y * 0.002)
                        .with_rotation(Quat::from_rotation_y(yaw))
                }),
            };
            if let Some(pose) = pose { *transform = pose; }
            *velocity = Velocity::zero();
            placed.push((coupling.object.clone(), *transform));
            // Put back where it belongs, it stays put; set down onto a surface, physics takes over.
            commands.entity(entity).insert(if onto_home { RigidBody::Fixed } else { RigidBody::Dynamic });
            released.push(coupling.object.clone());
            continue;
        }
        let hands: Vec<Transform> = coupling.anchors.iter()
            .filter_map(|(hand, ..)| hand_transform(&bind, &globals, *hand)).collect();
        if hands.len() != coupling.anchors.len() { continue; }
        let zones: Vec<Option<super::interaction::Zone>> = coupling.anchors.iter()
            .map(|(_, point, grip)| grip.and_then(|g| super::hand::grip_zone(object, *point, g))).collect();
        // Two or more hands hold a power grip at their palm grip points, not their wrists.
        let anchors: Vec<(Vec3, Quat, Vec3)> = coupling.anchors.iter().zip(&hands).zip(&zones)
            .map(|(((hand, point, grip), w), zone)| {
                let at = match (coupling.anchors.len() > 1, grip, zone, palm(*hand)) {
                    (true, Some(super::interaction::Grip::Power), Some(zone), Some(palm)) =>
                        *w * palm.grip(super::hand::grip_radius(object, zone)),
                    _ => w.translation,
                };
                // A power grip holds the handle's axis, not wherever the wrist latched inside the
                // zone's reach tolerance.
                let point = match (grip, zone) {
                    (Some(super::interaction::Grip::Power), Some(z)) => z.axis_point(Vec3::from(*point)),
                    _ => Vec3::from(*point),
                };
                (at, w.rotation, point)
            }).collect();
        let latch = coupling.latch.get_or_insert_with(|| {
            // A one-hand grip with a hint seats the object in the palm (openspec add-conforming-hand-grasps).
            let seat = match (coupling.anchors.as_slice(), zones.first(), palm(coupling.anchors[0].0)) {
                ([(_, _, Some(grip))], Some(Some(zone)), Some(palm)) => {
                    let latched = super::hand::in_hand(&hands[0], &*transform);
                    super::hand::seat(*grip, zone, object, &palm, &latched, coupling.anchors[0].0)
                        .map(|seat| (latched, seat, *grip, 0.))
                }
                _ => None,
            };
            CouplingLatch { object_rotation: transform.rotation, primary_rotation: anchors[0].1,
                            previous_primary: anchors[0].1, object_height: transform.translation.y, seat,
                            rigid: false, settle_frames: 0 }
        });
        let previous = *transform;
        if let Some((_, seat, grip, progress)) = latch.seat.as_mut() {
            // A plate goes onto the palm only once the palm faces up and the seated plate is clear
            // of the counter; until then it stays as gripped.
            let allowed = *grip != super::interaction::Grip::PalmTop
                || (palm(coupling.anchors[0].0).is_some_and(|p| (hands[0].rotation * p.normal).y >= SEAT_PALM_UP_COS)
                    && held_hit(object, &(hands[0] * *seat), &solids, HELD_CONTACT_M).is_none());
            let step = dt / super::hand::SEAT_BLEND_S;
            *progress = (*progress + if allowed { step } else { -step }).clamp(0., 1.);
        }
        let desired = match latch.seat {
            Some((latched, seat, _, progress)) if coupling.slide_height.is_none() =>
                hands[0] * super::hand::seat_blend(&latched, &seat, progress * super::hand::SEAT_BLEND_S),
            _ => coupled_pose(&anchors, transform.rotation, latch, coupling.slide_height.is_some()),
        };
        // Held objects do not pass through furniture or walls: when the hands would push
        // one through, it keeps its rotation and follows the primary hand, else it stops.
        // Only a deep penetration that cannot be avoided is an error.
        // Pressed into a surface (a mug lowered onto the desk) it rides up onto it instead.
        let kept = Transform::from_translation(anchors[0].0 - previous.rotation * anchors[0].2).with_rotation(previous.rotation);
        let lifted = (1..=HELD_LIFT_STEPS).map(|i| desired.with_translation(desired.translation + Vec3::Y * 0.01 * i as f32));
        *transform = match std::iter::once(desired).chain(lifted).chain([kept])
            .find(|pose| held_hit(object, pose, &solids, HELD_CONTACT_M).is_none()) {
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
        for ((hand, ..), w) in coupling.anchors.iter().zip(&hands) {
            held_in_hand.set(*hand, coupling.object.clone(), super::hand::in_hand(w, &*transform));
        }
    }
    office.couplings.retain(|c| !released.contains(&c.object));
    // A released object is an obstacle again from this frame on: record where it was put, not
    // where it was held, before physics publishes its new pose.
    for (id, pose) in placed {
        if let Some(object) = office.objects.iter_mut().find(|o| o.id == id) {
            object.position = pose.translation.to_array();
            object.rotation_xyzw = Some(pose.rotation.to_array());
            object.yaw_deg = pose.rotation.to_euler(EulerRot::YXZ).0.to_degrees();
        }
    }
    if office.motion_error.is_none() && error.is_some() { office.motion_error = error; }
}

/// A `palm_top` object is seated only while the palm faces up within 60 degrees.
pub const SEAT_PALM_UP_COS: f32 = 0.5;
/// Contact a held object may not make (m into another solid), and the depth that is an error.
pub const HELD_CONTACT_M: f32 = 0.005;
pub const HELD_PENETRATION_M: f32 = 0.05;
/// A held object pressed into a surface may ride up on it by up to this many centimetres.
pub const HELD_LIFT_STEPS: u32 = 8;

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
        CouplingLatch { object_rotation: object, primary_rotation: hand, previous_primary: hand, object_height: 0.1,
                        seat: None, rigid: false, settle_frames: 0 }
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
    fn hands_too_close_together_hold_the_object_rigidly_in_the_primary_hand() {
        let mut l = latch(Quat::IDENTITY, Quat::IDENTITY);
        let (g0, g1) = (Vec3::new(0., 0.4, 0.), Vec3::new(0., 0., 0.));
        let pose = coupled_pose(&[(Vec3::new(1., 1., 1.), Quat::IDENTITY, g0), (Vec3::new(1.05, 1., 1.), Quat::IDENTITY, g1)],
                                Quat::IDENTITY, &mut l, false);
        assert!(pose.rotation.dot(Quat::IDENTITY).abs() > 0.9999, "two hands 5 cm apart do not swing it");
        assert!((pose * g0).distance(Vec3::new(1., 1., 1.)) < 1e-4);
    }

    #[test]
    fn hands_meeting_and_parting_never_snap_the_stick() {
        let mut l = latch(Quat::IDENTITY, Quat::IDENTITY);
        let (g0, g1) = (Vec3::new(0., 0.5, 0.), Vec3::new(0., 0.1, 0.));
        let pose = |p1: Vec3, current: Quat, l: &mut CouplingLatch|
            coupled_pose(&[(Vec3::new(0., 1.2, 0.), Quat::IDENTITY, g0), (p1, Quat::IDENTITY, g1)], current, l, false);
        let mut r = pose(Vec3::new(0., 0.8, 0.), Quat::IDENTITY, &mut l).rotation;
        // The lower hand slides up beside the upper one (12 cm apart, level): no snap to horizontal.
        let close = pose(Vec3::new(0.12, 1.2, 0.), r, &mut l).rotation;
        assert!(close.angle_between(r) < 1e-4, "entering rigid keeps the stick where it is");
        assert!(l.rigid);
        // 20 cm apart: still rigid (hysteresis), still no change.
        r = close;
        assert!(pose(Vec3::new(0., 1.0, 0.), r, &mut l).rotation.angle_between(r) < 1e-4);
        // Apart again (40 cm below): it eases back to the hand line rather than jumping.
        let first = pose(Vec3::new(0.1, 0.8, 0.), r, &mut l).rotation;
        assert!(!l.rigid);
        let target = Quat::from_rotation_arc(Vec3::Y, (Vec3::new(0., 1.2, 0.) - Vec3::new(0.1, 0.8, 0.)).normalize());
        assert!(first.angle_between(r) < target.angle_between(r) * 0.5, "blended, not snapped");
        let mut q = first;
        for _ in 0..TWO_HAND_SETTLE_FRAMES { q = pose(Vec3::new(0.1, 0.8, 0.), q, &mut l).rotation; }
        assert!((q * Vec3::Y).dot(target * Vec3::Y) > 0.99, "settled on the hand line");
    }

    #[test]
    fn grips_close_on_a_stick_still_run_it_through_both_hands() {
        let mut l = latch(Quat::IDENTITY, Quat::IDENTITY);
        // Grips 5.5 cm apart on the stick's axis, hands 0.5 m apart: the stick runs through both.
        let (g0, g1) = (Vec3::new(0., 0.19, 0.), Vec3::new(0., 0.135, 0.));
        let (p0, p1) = (Vec3::new(1., 1., 1.), Vec3::new(1.5, 1., 1.));
        let pose = coupled_pose(&[(p0, Quat::IDENTITY, g0), (p1, Quat::IDENTITY, g1)], Quat::IDENTITY, &mut l, false);
        assert!((pose * g0).distance(p0) < 1e-4);
        let axis = pose.rotation * Vec3::Y;
        let off = (p1 - p0) - axis * (p1 - p0).dot(axis);
        assert!(off.length() < 1e-4, "the second hand is {off:?} off the stick");
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
        // A mug pressed 3.5 cm into the desk top is clear of it once raised by the lift allowance.
        let mug = objects.iter().find(|o| o.id == "mug").unwrap();
        let desk = super::super::support::boxes(&objects.iter().filter(|o| o.id == "desk").cloned().collect::<Vec<_>>());
        let pressed = Transform::from_translation(Vec3::new(0.74, 0.80, -1.9));
        assert_eq!(held_hit(mug, &pressed, &desk, HELD_CONTACT_M), Some("desk"));
        let raised = pressed.with_translation(pressed.translation + Vec3::Y * 0.01 * HELD_LIFT_STEPS as f32);
        assert!(held_hit(mug, &raised, &desk, HELD_CONTACT_M).is_none());
    }

    #[test]
    fn set_down_lands_inside_the_placement_zone_not_where_the_hand_let_go() {
        let zone: super::super::interaction::Zone = serde_json::from_value(serde_json::json!({
            "id":"place_area","type":"patch","center":[0.35,0.42,0.0],"half_extents":[0.25,0.1,0.3]})).unwrap();
        // Released 0.36 m off-centre towards the table edge (live run): pulled in to fit a 0.17 m plate.
        let p = placement_point(&zone, Vec3::new(0.36, 0.53, -0.36), 0.17);
        assert!(p.distance(Vec3::new(0.36, 0.42, -0.13)) < 1e-5, "{p:?}");
        let q = placement_point(&zone, Vec3::new(0.0, 0.5, 0.0), 0.17);
        assert!(q.distance(Vec3::new(0.27, 0.42, 0.0)) < 1e-5, "{q:?}");
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
