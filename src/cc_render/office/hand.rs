//! Conforming hand grasps: the palm grip frame, seated grips and contact-conforming
//! fingers (openspec add-conforming-hand-grasps; restored by owner decision 2026-10-17).
//!
//! Everything here is in the hand bone's local space, so a grip computed once stays
//! valid while the hand moves. Fingers are posed from the rig's own rest transforms:
//! flexion is a rotation about each finger bone's local Z, with the closing sign read
//! from the rig (the side the thumb sits on), never assumed per side.
use super::interaction::{Grip, Zone, ZoneKind};
use bevy::prelude::*;
use bevy_rapier3d::rapier::parry::{self, na};

/// Finger names in CC5 bone order; the thumb closes last so it opposes the fingers.
pub const FINGERS: [&str; 5] = ["Thumb", "Index", "Mid", "Ring", "Pinky"];
/// Half thickness of the palm: the palm surface lies this far along the normal.
pub const PALM_HALF_THICKNESS_M: f32 = 0.012;
/// Handles shorter than this are gripped at their middle.
pub const SHORT_HANDLE_M: f32 = 0.12;
/// Seated grips blend in from the latch offset over this long.
pub const SEAT_BLEND_S: f32 = 0.25;
const FLEX_LIMITS: [f32; 3] = [1.5, 1.7, 1.2];
const THUMB_LIMITS: [f32; 3] = [0.9, 0.9, 0.9];
const COARSE_STEP: f32 = 0.15;
/// A finger may press this far into what it holds (skin compressing on a handle).
pub const SKIN_CONTACT_M: f32 = 0.004;
const GAP_RANGE_M: f32 = 0.05;
const REFINE_STEPS: usize = 4;

#[derive(Clone, Debug)]
pub struct FingerRig {
    /// Rest local transforms of bones 1..3 (bone 1 relative to the hand).
    pub rest: [Transform; 3],
    /// Fingertip in bone 3's frame.
    pub tip: Vec3,
    pub radius: f32,
    pub limits: [f32; 3],
    /// Local axis (signed) whose positive rotation closes this finger.
    pub axis: Vec3,
}

impl FingerRig {
    pub fn new(rest: [Transform; 3], thumb: bool) -> Self {
        // The last bone has no child: its tip continues 0.75 of its own length along the bone axis.
        let tip = Vec3::Y * rest[2].translation.length() * 0.75;
        Self { rest, tip, radius: if thumb { 0.009 } else { 0.008 },
               limits: if thumb { THUMB_LIMITS } else { FLEX_LIMITS }, axis: Vec3::Z }
    }

    /// Joint positions (bones 1..3) and fingertip in hand space at the given flexion.
    pub fn chain(&self, flex: f32, angles: [f32; 3]) -> [Vec3; 4] {
        let mut t = Transform::IDENTITY;
        let mut points = [Vec3::ZERO; 4];
        for k in 0..3 {
            t = t * self.posed(flex, k, angles[k]);
            points[k] = t.translation;
        }
        points[3] = t * self.tip;
        points
    }

    /// Bone `k`'s local transform closed by `angle` (`flex` scales the finger's axis: the
    /// rig's closing sign while the axes are being read, 1 afterwards).
    pub fn posed(&self, flex: f32, k: usize, angle: f32) -> Transform {
        let mut local = self.rest[k];
        local.rotation = local.rotation * Quat::from_axis_angle(self.axis, flex * angle);
        local
    }
}

/// The palm grip frame in hand space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PalmFrame {
    pub wrist: Vec3,
    pub knuckles: Vec3,
    /// From the wrist towards the knuckles.
    pub forward: Vec3,
    /// Across the knuckles, towards the index finger: the axis the fingers wrap around.
    pub lateral: Vec3,
    /// The side the palm faces (where the fingers close to).
    pub normal: Vec3,
}

impl PalmFrame {
    /// A point on the palm's surface, 60% of the way from the wrist to the knuckles.
    pub fn palm(&self) -> Vec3 {
        self.wrist + (self.knuckles - self.wrist) * 0.6 + self.normal * PALM_HALF_THICKNESS_M
    }
    /// Where the axis of a handle of radius `r` lies in a power grip: against the palm,
    /// just behind the knuckles, inside the closing fingers.
    pub fn grip(&self, r: f32) -> Vec3 {
        self.knuckles - self.forward * 0.015 + self.normal * (PALM_HALF_THICKNESS_M + r)
    }
}

#[derive(Clone, Debug)]
pub struct HandRig {
    /// Thumb, Index, Mid, Ring, Pinky.
    pub fingers: [FingerRig; 5],
    /// Sign of the rotation about a finger bone's local Z that closes the finger.
    pub flex: f32,
    pub palm: PalmFrame,
}

impl HandRig {
    pub fn new(rests: [[Transform; 3]; 5]) -> Self {
        let fingers: [FingerRig; 5] = std::array::from_fn(|i| FingerRig::new(rests[i], i == 0));
        let knuckles = fingers[1..].iter().map(|f| f.rest[0].translation).sum::<Vec3>() / 4.;
        let forward = knuckles.normalize();
        let reject = |v: Vec3, n: Vec3| v - n * v.dot(n);
        // The middle fingertip's motion under a small closing flexion, on the side of the thumb.
        let mid = &fingers[2];
        let moved = mid.chain(1., [0.3, 0.3, 0.]).map(|p| p)[3] - mid.chain(1., [0.; 3])[3];
        let moved = reject(moved, forward);
        let thumb = reject(fingers[0].rest[0].translation, forward);
        let flex = if moved.dot(thumb) >= 0. { 1. } else { -1. };
        let normal = (moved * flex).normalize();
        let across = fingers[1].rest[0].translation - fingers[4].rest[0].translation;
        let lateral = reject(reject(across, forward), normal).normalize();
        let palm = PalmFrame { wrist: Vec3::ZERO, knuckles, forward, lateral, normal };
        let mut fingers = fingers;
        for finger in &mut fingers[1..] { finger.axis = Vec3::Z * flex; }
        // The thumb's bones are oriented differently: it closes about whichever local axis
        // brings its tip across the palm towards the grip (opposition), read from the rig.
        let grip = palm.grip(0.015);
        let thumb = &fingers[0];
        let candidates = [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z];
        fingers[0].axis = *candidates.iter().min_by(|a, b| {
            let reach = |axis: Vec3| FingerRig { axis, ..thumb.clone() }.chain(1., [0.6; 3])[3].distance(grip);
            reach(**a).total_cmp(&reach(**b))
        }).unwrap();
        Self { fingers, flex: 1., palm }
    }
}

/// A solid of a held object, in hand space.
#[derive(Clone, Copy, Debug)]
pub struct Solid {
    pub pose: Transform,
    pub half: Vec3,
    pub cylinder: bool,
}

/// The object's solids (its parts, or its box) placed by `object_in_hand`.
pub fn solids(object: &super::OfficeObject, object_in_hand: &Transform) -> Vec<Solid> {
    let place = |position: [f32; 3], half: [f32; 3], cylinder: bool| Solid {
        pose: *object_in_hand * Transform::from_translation(Vec3::from(position)),
        half: Vec3::from(half), cylinder };
    if object.parts.is_empty() {
        vec![place([0.; 3], object.half_extents, false)]
    } else {
        object.parts.iter().map(|p| place(p.position, p.half_extents, p.cylinder)).collect()
    }
}

fn isometry(t: &Transform) -> na::Isometry3<f32> {
    let q = t.rotation;
    na::Isometry3::from_parts(na::Translation3::new(t.translation.x, t.translation.y, t.translation.z),
                              na::UnitQuaternion::from_quaternion(na::Quaternion::new(q.w, q.x, q.y, q.z)))
}

/// Signed gap between a finger segment `a`–`b` of radius `r` and the solids: negative
/// is penetration depth.
fn gap(a: Vec3, b: Vec3, r: f32, solids: &[Solid]) -> f32 {
    let capsule = parry::shape::Capsule::new(na::Point3::new(a.x, a.y, a.z), na::Point3::new(b.x, b.y, b.z), r);
    let origin = na::Isometry3::identity();
    solids.iter().map(|s| {
        let pose = isometry(&s.pose);
        let c = if s.cylinder {
            parry::query::contact(&origin, &capsule, &pose, &parry::shape::Cylinder::new(s.half.y, s.half.x), GAP_RANGE_M)
        } else {
            parry::query::contact(&origin, &capsule, &pose,
                                  &parry::shape::Cuboid::new(na::Vector3::new(s.half.x, s.half.y, s.half.z)), GAP_RANGE_M)
        };
        c.ok().flatten().map_or(f32::INFINITY, |c| c.dist)
    }).fold(f32::INFINITY, f32::min)
}

/// Whether any segment from bone `from` outward presses into the solids beyond skin contact.
fn touching(finger: &FingerRig, flex: f32, angles: [f32; 3], from: usize, solids: &[Solid]) -> bool {
    let points = finger.chain(flex, angles);
    (from..3).any(|k| gap(points[k], points[k + 1], finger.radius, solids) <= -SKIN_CONTACT_M)
}

/// Flexion of one finger closed onto the solids: each joint, proximal to distal, bends
/// until that segment or a more distal one touches, else to its limit. A finger that
/// starts inside the object (through a mug handle's loop) first bends until it is
/// clear, then closes from there; one that cannot clear is left as it was.
pub fn close_finger(finger: &FingerRig, flex: f32, solids: &[Solid]) -> [f32; 3] {
    let mut angles = [0.; 3];
    for j in 0..3 {
        let limit = finger.limits[j];
        let mut start = 0.;
        if touching(finger, flex, angles, j, solids) {
            let mut x = COARSE_STEP;
            let mut clear = None;
            while x < limit + 1e-6 {
                let mut trial = angles;
                trial[j] = x.min(limit);
                if !touching(finger, flex, trial, j, solids) { clear = Some(trial[j]); break; }
                x += COARSE_STEP;
            }
            let Some(clear) = clear else {
                // Cannot clear (a finger across a handle's end): the least penetrating angle.
                let depth = |x: f32| {
                    let mut trial = angles;
                    trial[j] = x;
                    let p = finger.chain(flex, trial);
                    (j..3).map(|k| gap(p[k], p[k + 1], finger.radius, solids)).fold(f32::INFINITY, f32::min)
                };
                let steps = (limit / COARSE_STEP).ceil() as usize;
                angles[j] = (0..=steps).map(|n| (n as f32 * COARSE_STEP).min(limit))
                    .max_by(|a, b| depth(*a).total_cmp(&depth(*b))).unwrap_or(0.);
                continue;
            };
            start = clear;
            angles[j] = clear;
        }
        let (mut open, mut shut) = (start, None);
        let mut x = start + COARSE_STEP;
        while x < limit + 1e-6 {
            let mut trial = angles;
            trial[j] = x.min(limit);
            if touching(finger, flex, trial, j, solids) { shut = Some(trial[j]); break; }
            open = trial[j];
            x += COARSE_STEP;
        }
        angles[j] = match shut {
            None => limit,
            Some(mut shut) => {
                for _ in 0..REFINE_STEPS {
                    let mut trial = angles;
                    trial[j] = (open + shut) / 2.;
                    if touching(finger, flex, trial, j, solids) { shut = trial[j] } else { open = trial[j] }
                }
                open
            }
        };
    }
    angles
}

/// All five fingers closed onto an object held at `object_in_hand`.
pub fn close_hand(rig: &HandRig, object: &super::OfficeObject, object_in_hand: &Transform) -> [[f32; 3]; 5] {
    let solids = solids(object, object_in_hand);
    std::array::from_fn(|i| close_finger(&rig.fingers[i], rig.flex, &solids))
}

/// Lowest object-local y of the object's solids (its resting surface).
pub fn lowest_local_y(object: &super::OfficeObject) -> f32 {
    if object.parts.is_empty() { return -object.half_extents[1]; }
    object.parts.iter().map(|p| p.position[1] - p.half_extents[1]).fold(f32::INFINITY, f32::min)
}

/// Radius of the part a grip segment runs through (its thickness across the segment).
fn handle_radius(object: &super::OfficeObject, a: Vec3, b: Vec3) -> f32 {
    let axis = (b - a).normalize_or_zero();
    let middle = (a + b) / 2.;
    let parts: Vec<(Vec3, Vec3)> = if object.parts.is_empty() {
        vec![(Vec3::ZERO, Vec3::from(object.half_extents))]
    } else {
        object.parts.iter().map(|p| (Vec3::from(p.position), Vec3::from(p.half_extents))).collect()
    };
    let (_, half) = parts.into_iter().min_by(|x, y| {
        let inside = |(c, h): &(Vec3, Vec3)| ((middle - *c).abs() - *h).max(Vec3::ZERO).length();
        inside(x).total_cmp(&inside(y))
    }).unwrap_or((Vec3::ZERO, Vec3::splat(0.015)));
    // Mean half thickness across the segment axis.
    let across: Vec<f32> = [Vec3::X, Vec3::Y, Vec3::Z].iter().zip([half.x, half.y, half.z])
        .filter(|(e, _)| e.dot(axis).abs() < 0.7).map(|(_, h)| h).collect();
    if across.is_empty() { 0.015 } else { across.iter().sum::<f32>() / across.len() as f32 }
}

/// The power-grip radius for a zone on `object` (used for palm grip points).
pub fn grip_radius(object: &super::OfficeObject, zone: &Zone) -> f32 {
    match (zone.kind, zone.a, zone.b) {
        (ZoneKind::Segment, Some(a), Some(b)) => handle_radius(object, Vec3::from(a), Vec3::from(b)),
        _ => 0.015,
    }
}

/// The seated grip: the object's pose in hand space for `grip` on `zone`, chosen
/// closest to `latch` (the object's pose in hand space when it latched). `None`
/// keeps the latch pose (`pinch`, or a zone the grip cannot seat).
pub fn seat(grip: Grip, zone: &Zone, object: &super::OfficeObject, palm: &PalmFrame, latch: &Transform,
            hand: super::super::renderer::Hand) -> Option<Transform> {
    match grip {
        Grip::Power => {
            let (a, b) = (Vec3::from(zone.a?), Vec3::from(zone.b?));
            if zone.kind != ZoneKind::Segment || a.distance(b) < 1e-4 { return None; }
            let axis = (b - a).normalize();
            let target = palm.grip(handle_radius(object, a, b));
            // A short handle (a mug's) is gripped at its middle so the fingers fit between its ends;
            // a long one (a broom stick) at the point nearest the palm grip at latch.
            let local = latch.rotation.inverse() * (target - latch.translation);
            let g = if a.distance(b) < SHORT_HANDLE_M { (a + b) / 2. }
                    else { a + axis * (local - a).dot(axis).clamp(0., a.distance(b)) };
            // Swing the segment onto the wrap axis (the nearer direction), keeping the latch roll.
            let held = latch.rotation * axis;
            let along = palm.lateral * if held.dot(palm.lateral) >= 0. { 1. } else { -1. };
            let mut rotation = Quat::from_rotation_arc(held, along) * latch.rotation;
            // A declared palm direction sets the roll: the hand wraps from that side of the handle.
            // Otherwise a body off the segment (a mug) goes beyond the knuckles, not into the palm.
            let body = -g - axis * (-g).dot(axis);
            if let Some(palm_local) = zone.palm.map(|side| Vec3::from(side.for_hand(hand))) {
                let from = (rotation * (palm_local - axis * palm_local.dot(axis))).normalize();
                let to = (palm.normal - along * palm.normal.dot(along)).normalize();
                let angle = along.dot(from.cross(to)).atan2(from.dot(to));
                rotation = Quat::from_axis_angle(along, angle) * rotation;
            } else if body.length() > 0.02 {
                let from = (rotation * body).normalize();
                let to = (palm.forward - along * palm.forward.dot(along)).normalize();
                let angle = along.dot(from.cross(to)).atan2(from.dot(to));
                rotation = Quat::from_axis_angle(along, angle) * rotation;
            }
            Some(Transform::from_translation(target - rotation * g).with_rotation(rotation.normalize()))
        }
        Grip::PalmTop => {
            // The object's up onto the palm normal, keeping its yaw; its underside on the palm.
            let rotation = (Quat::from_rotation_arc(latch.rotation * Vec3::Y, palm.normal) * latch.rotation).normalize();
            let center = zone.center.map(Vec3::from).unwrap_or(Vec3::ZERO);
            let underside = Vec3::new(center.x, lowest_local_y(object), center.z);
            let target = palm.palm() + palm.forward * 0.04 + palm.normal * 0.002;
            Some(Transform::from_translation(target - rotation * underside).with_rotation(rotation))
        }
        Grip::Pinch => None,
    }
}

/// The latch pose blended towards the seat: smoothstep over [`SEAT_BLEND_S`].
pub fn seat_blend(latch: &Transform, seat: &Transform, elapsed_s: f32) -> Transform {
    let x = (elapsed_s / SEAT_BLEND_S).clamp(0., 1.);
    let w = x * x * (3. - 2. * x);
    Transform::from_translation(latch.translation.lerp(seat.translation, w))
        .with_rotation(latch.rotation.slerp(seat.rotation, w))
}

/// Object pose in hand space from world poses.
pub fn in_hand(hand: &Transform, object: &Transform) -> Transform {
    Transform::from_matrix(hand.compute_matrix().inverse() * object.compute_matrix())
}

/// The zone a coupling anchor grips: the object's zone with that grip hint nearest the
/// anchor's object-local grip point.
pub fn grip_zone(object: &super::OfficeObject, point: [f32; 3], grip: Grip) -> Option<Zone> {
    let p = Vec3::from(point);
    super::interaction::all_zones(object).into_iter().filter(|z| z.grip == Some(grip))
        .min_by(|a, b| a.distance_local(p).total_cmp(&b.distance_local(p)))
}

/// A captured hand: its rig and the bone entities it poses.
pub struct CapturedHand {
    pub rig: HandRig,
    pub hand: Entity,
    pub bones: [[Entity; 3]; 5],
}

/// Both hands' rigs, captured once from the CC5 rest pose.
#[derive(Resource, Default)]
pub struct HandRigs {
    pub hands: [Option<CapturedHand>; 2],
}

fn side(hand: super::super::renderer::Hand) -> usize {
    if hand == super::super::renderer::Hand::Left { 0 } else { 1 }
}

impl HandRigs {
    pub fn get(&self, hand: super::super::renderer::Hand) -> Option<&CapturedHand> {
        self.hands[side(hand)].as_ref()
    }
}

/// Objects held by declared couplings this frame, as (object id, pose in hand space),
/// left then right. Written after coupling, read by the finger system next frame.
#[derive(Resource, Default)]
pub struct HeldInHand {
    pub hands: [Option<(String, Transform)>; 2],
}

impl HeldInHand {
    pub fn set(&mut self, hand: super::super::renderer::Hand, object: String, pose: Transform) {
        self.hands[side(hand)] = Some((object, pose));
    }
}

pub(crate) fn capture_hand_rigs(
    mut rigs: ResMut<HandRigs>,
    bind: Res<super::super::bevy_app::pose::CC5BindRotations>,
    bones: Query<(Entity, &Name, &Transform)>,
) {
    if !bind.captured || rigs.hands.iter().all(Option::is_some) { return; }
    for (i, prefix) in ["CC_Base_L_", "CC_Base_R_"].iter().enumerate() {
        if rigs.hands[i].is_some() { continue; }
        let Some(hand) = bind.entities[if i == 0 { 20 } else { 21 }] else { continue };
        let find = |name: String| bones.iter().find(|(_, n, _)| n.as_str() == name).map(|(e, _, t)| (e, *t));
        let found: Option<Vec<[(Entity, Transform); 3]>> = FINGERS.iter().map(|finger| {
            Some([find(format!("{prefix}{finger}1"))?, find(format!("{prefix}{finger}2"))?, find(format!("{prefix}{finger}3"))?])
        }).collect();
        let Some(found) = found else { continue };
        let rests: [[Transform; 3]; 5] = std::array::from_fn(|f| found[f].map(|(_, t)| t));
        let bones: [[Entity; 3]; 5] = std::array::from_fn(|f| found[f].map(|(e, _)| e));
        rigs.hands[i] = Some(CapturedHand { rig: HandRig::new(rests), hand, bones });
    }
}

/// A holding hand's grasp as rendered: how far the object is from its seated grip and
/// how far each finger closed (radians, summed over its joints), with whether it
/// stopped on contact.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraspReport {
    pub hand: super::super::renderer::Hand,
    pub object: String,
    pub seat_error_m: Option<f32>,
    pub flexion: [f32; 5],
    pub touching: [bool; 5],
}

#[derive(Default)]
pub(crate) struct FingerState {
    angles: [[f32; 3]; 5],
    target: [[f32; 3]; 5],
    key: Option<(String, Transform)>,
}

/// Recompute a closure only when the object moved in the hand by more than this.
const RECLOSE_M: f32 = 0.002;
const RECLOSE_RAD: f32 = 0.03;
/// Finger angles follow their target with this time constant (≈ 0.25 s to settle).
const FINGER_TAU_S: f32 = 0.06;

/// Close each holding hand's fingers onto its object; open them again after release.
/// Runs before transform propagation, in hand space, from the object's pose in the
/// hand on the previous frame (both poses from the same frame).
pub(crate) fn conform_fingers(
    rigs: Res<HandRigs>,
    held: Res<HeldInHand>,
    mut office: ResMut<super::controller::OfficeController>,
    registry: Option<Res<super::super::bevy_app::physics::props::PropRegistry>>,
    globals: Query<&GlobalTransform>,
    mut transforms: Query<&mut Transform>,
    time: Res<Time>,
    mut states: Local<[FingerState; 2]>,
) {
    let blend = 1. - (-time.delta_secs() / FINGER_TAU_S).exp();
    let mut reports = Vec::new();
    for i in 0..2 {
        let Some(captured) = rigs.hands[i].as_ref() else { continue };
        let hand = if i == 0 { super::super::renderer::Hand::Left } else { super::super::renderer::Hand::Right };
        // A declared coupling, else a physical grip.
        let holding = held.hands[i].clone().or_else(|| {
            let (_, id) = office.held.iter().find(|(h, _)| *h == hand)?;
            let entity = *registry.as_ref()?.by_id.get(id)?;
            let (h, o) = (globals.get(captured.hand).ok()?, globals.get(entity).ok()?);
            Some((id.clone(), in_hand(&h.compute_transform(), &o.compute_transform())))
        });
        let state = &mut states[i];
        match holding.as_ref().and_then(|(id, pose)| office.objects.iter().find(|o| &o.id == id).map(|o| (o, pose))) {
            Some((object, pose)) => {
                let moved = state.key.as_ref().is_none_or(|(id, last)| id != &object.id
                    || last.translation.distance(pose.translation) > RECLOSE_M
                    || last.rotation.angle_between(pose.rotation) > RECLOSE_RAD);
                if moved {
                    state.target = close_hand(&captured.rig, object, pose);
                    state.key = Some((object.id.clone(), *pose));
                }
            }
            None => { state.target = [[0.; 3]; 5]; state.key = None; }
        }
        if let Some((id, pose)) = holding.as_ref() {
            // Seat error: distance of the object's grip from its seated pose, when it has one.
            let seat_error_m = office.couplings.iter().find(|c| &c.object == id)
                .and_then(|c| c.latch.as_ref()?.seat).map(|(_, seat, ..)| seat.translation.distance(pose.translation));
            let rig = &captured.rig;
            reports.push(GraspReport {
                hand, object: id.clone(), seat_error_m,
                flexion: std::array::from_fn(|f| state.angles[f].iter().sum()),
                touching: std::array::from_fn(|f| (0..3).any(|k| state.target[f][k] < rig.fingers[f].limits[k] - 1e-3)),
            });
        }
        for f in 0..5 {
            for k in 0..3 {
                let a = &mut state.angles[f][k];
                *a += (state.target[f][k] - *a) * blend;
                if let Ok(mut transform) = transforms.get_mut(captured.bones[f][k]) {
                    transform.rotation = captured.rig.fingers[f].posed(captured.rig.flex, k, *a).rotation;
                }
            }
        }
    }
    office.grasps = reports;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The right hand's rest locals from beautiful_assistant.smpl_aligned.glb.
    pub(crate) fn right_hand() -> HandRig {
        let t = |p: [f32; 3], q: [f32; 4]| Transform::from_translation(Vec3::from(p))
            .with_rotation(Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize());
        HandRig::new([
            [t([-0.013887, -0.015272, 0.021620], [0.102497, 0.380802, 0.834937, 0.383880]),
             t([-0.000017, 0.052421, 0.000009], [-0.164003, 0.009175, -0.051738, 0.985059]),
             t([0.000003, 0.023961, 0.000013], [0.008326, -0.002170, -0.058362, 0.998258])],
            [t([-0.073121, -0.044161, 0.026774], [-0.002943, -0.010822, 0.873668, 0.486394]),
             t([-0.000011, 0.036255, 0.000007], [0.006513, -0.005408, 0.023197, 0.999695]),
             t([0.000001, 0.022570, 0.000012], [0.007170, 0.000002, -0.021362, 0.999746])],
            [t([-0.075040, -0.042278, 0.006962], [0.003351, -0.001215, 0.874617, 0.484802]),
             t([-0.000016, 0.041215, 0.000016], [-0.000416, -0.003511, 0.007948, 0.999962]),
             t([0.000002, 0.025586, 0.000012], [0.006931, -0.000793, -0.008136, 0.999943])],
            [t([-0.070468, -0.043627, -0.009921], [0.006125, -0.003084, 0.870508, 0.492106]),
             t([-0.000006, 0.037616, 0.000011], [-0.001027, 0.000434, 0.038685, 0.999251]),
             t([0.000000, 0.022236, 0.000008], [-0.003814, 0.000022, -0.020508, 0.999782])],
            [t([-0.065722, -0.046465, -0.025992], [0.008551, 0.001807, 0.872757, 0.488077]),
             t([0.000003, 0.026999, 0.000035], [-0.010432, 0.001837, 0.024246, 0.999650]),
             t([-0.000002, 0.014287, 0.000005], [-0.007217, -0.001659, -0.026573, 0.999619])],
        ])
    }

    fn object(parts: serde_json::Value) -> super::super::OfficeObject {
        serde_json::from_value(serde_json::json!({"id":"thing","label":"thing","position":[0.,0.,0.],
            "half_extents":[0.1,0.1,0.1],"color":[1.,1.,1.],"dynamic":true,"approach":[0.,0.,0.5],"parts":parts})).unwrap()
    }
    fn zone(value: serde_json::Value) -> Zone { serde_json::from_value(value).unwrap() }
    fn broom() -> super::super::OfficeObject {
        object(serde_json::json!([{"position":[0.0,-0.62,0.0],"half_extents":[0.06,0.04,0.15]},
                                  {"position":[0.0,0.04,0.0],"half_extents":[0.015,0.62,0.015]}]))
    }
    fn mug() -> super::super::OfficeObject {
        object(serde_json::json!([{"position":[0,0,0],"half_extents":[0.045,0.075,0.045],"cylinder":true},
            {"position":[0.0625,0.04,0],"half_extents":[0.0225,0.008,0.009]},
            {"position":[0.0625,-0.04,0],"half_extents":[0.0225,0.008,0.009]},
            {"position":[0.082,0,0],"half_extents":[0.008,0.04,0.009]}]))
    }
    fn plate() -> super::super::OfficeObject {
        object(serde_json::json!([{"position":[0,0,0],"half_extents":[0.17,0.015,0.17],"cylinder":true}]))
    }

    #[test]
    fn palm_frame_is_read_from_the_rig() {
        let rig = right_hand();
        let p = rig.palm;
        // Orthonormal, fingers forward from the wrist, the thumb on the palm side.
        assert!(p.forward.dot(p.normal).abs() < 1e-4 && p.lateral.dot(p.normal).abs() < 1e-4 && p.lateral.dot(p.forward).abs() < 1e-4);
        assert!(rig.fingers[0].rest[0].translation.dot(p.normal) > 0.);
        // The index knuckle is on the +lateral side of the pinky knuckle.
        assert!((rig.fingers[1].rest[0].translation - rig.fingers[4].rest[0].translation).dot(p.lateral) > 0.03);
        // The thumb closes across the palm towards the grip (opposition), not away from it.
        let thumb = &rig.fingers[0];
        let (open, shut) = (thumb.chain(rig.flex, [0.; 3])[3], thumb.chain(rig.flex, [0.6; 3])[3]);
        assert!(shut.distance(p.grip(0.015)) < open.distance(p.grip(0.015)) - 0.04, "thumb closes away: {open:?} -> {shut:?}");
        // Closing moves every fingertip towards the palm side.
        for finger in &rig.fingers[1..] {
            let (open, shut) = (finger.chain(rig.flex, [0.; 3])[3], finger.chain(rig.flex, [0.5; 3])[3]);
            assert!((shut - open).dot(p.normal) > 0.01, "{:?}", shut - open);
        }
    }

    #[test]
    fn nothing_to_touch_closes_to_the_limits() {
        let rig = right_hand();
        let far = [Solid { pose: Transform::from_xyz(5., 5., 5.), half: Vec3::splat(0.1), cylinder: false }];
        assert_eq!(close_finger(&rig.fingers[2], rig.flex, &far), FLEX_LIMITS);
    }

    #[test]
    fn fingers_wrap_a_seated_broom_stick_without_penetrating_it() {
        let rig = right_hand();
        let broom = broom();
        let grip = zone(serde_json::json!({"id":"grip_upper","type":"segment","a":[0.,0.2,0.],"b":[0.,0.5,0.],
                                           "radius":0.12,"grip":"power"}));
        // Latched 3 cm off the palm and turned 40 degrees off the wrap axis.
        let latch = Transform::from_translation(rig.palm.grip(0.015) + rig.palm.normal * 0.03 - Vec3::Y * 0.3)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, rig.palm.lateral) * Quat::from_rotation_z(0.7));
        let seated = seat(Grip::Power, &grip, &broom, &rig.palm, &latch, crate::cc_render::Hand::Right).unwrap();
        // The stick's axis lies along the wrap axis, through the palm grip point.
        let axis = seated.rotation * Vec3::Y;
        assert!(axis.dot(rig.palm.lateral).abs() > 0.999);
        let g = seated.rotation.inverse() * (rig.palm.grip(0.015) - seated.translation);
        assert!(Vec2::new(g.x, g.z).length() < 1e-4, "grip point off the stick axis: {g:?}");
        let angles = close_hand(&rig, &broom, &seated);
        let solids = solids(&broom, &seated);
        for (i, finger) in rig.fingers.iter().enumerate().skip(1) {
            // Each finger is bent around the stick (not left open, not a full fist) and touches it.
            assert!(angles[i][0] + angles[i][1] > 0.8, "{} barely closed: {:?}", FINGERS[i], angles[i]);
            let points = finger.chain(rig.flex, angles[i]);
            let closest = (0..3).map(|k| gap(points[k], points[k + 1], finger.radius, &solids)).fold(f32::INFINITY, f32::min);
            assert!((-SKIN_CONTACT_M..0.01).contains(&closest), "{} gap {closest}", FINGERS[i]);
        }
    }

    #[test]
    fn a_mug_is_held_by_its_handle_with_the_body_beyond_the_knuckles() {
        let rig = right_hand();
        let mug = mug();
        let handle = zone(serde_json::json!({"id":"handle","type":"segment","a":[0.082,-0.03,0.],"b":[0.082,0.03,0.],
                                             "radius":0.08,"grip":"power"}));
        // Latched near the handle's lower end: the hand still centres on it.
        let latch = Transform::from_translation(rig.palm.grip(0.01) + rig.palm.forward * 0.03 + rig.palm.lateral * 0.03)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, -rig.palm.lateral) * Quat::from_rotation_y(2.0));
        let seated = seat(Grip::Power, &handle, &mug, &rig.palm, &latch, crate::cc_render::Hand::Right).unwrap();
        assert!((seated * Vec3::new(0.082, 0., 0.)).distance(rig.palm.grip(0.0085)) < 1e-3, "not centred on the handle");
        let body = seated.translation; // the mug's origin is its body's centre
        let handle_point = seated * Vec3::new(0.082, 0., 0.);
        assert!((body - handle_point).dot(rig.palm.forward) > 0.07, "mug body not beyond the knuckles");
        let angles = close_hand(&rig, &mug, &seated);
        // Every finger curls: the middle and ring through the handle around the bar, the index
        // and pinky hooked over its ends. None is left straight inside the mug.
        for i in 1..5 {
            assert!(angles[i][0] + angles[i][1] > 0.8, "{} {:?}", FINGERS[i], angles[i]);
        }
        let solids = solids(&mug, &seated);
        for (i, finger) in rig.fingers.iter().enumerate() {
            let p = finger.chain(rig.flex, angles[i]);
            let gaps: Vec<f32> = (0..3).map(|k| gap(p[k], p[k + 1], finger.radius, &solids)).collect();
            let open = finger.chain(rig.flex, [0.; 3]);
            let open_gaps: Vec<f32> = (0..3).map(|k| gap(open[k], open[k + 1], finger.radius, &solids)).collect();
            // Fingers that start clear stay within skin contact; one that starts deep inside the
            // handle's end (the index) ends hooked over it, at most just past that.
            let allowed = if open_gaps.iter().any(|g| *g < -SKIN_CONTACT_M) { 2. * SKIN_CONTACT_M } else { SKIN_CONTACT_M };
            assert!(gaps.iter().all(|g| *g > -allowed - 1e-4),
                    "{} inside the mug: angles {:?} gaps {:?} open {:?}", FINGERS[i], angles[i], gaps, open_gaps);
        }
    }

    #[test]
    fn fingers_stay_flat_under_a_plate_on_the_palm() {
        let rig = right_hand();
        let plate = plate();
        let under = zone(serde_json::json!({"id":"underside","type":"patch","center":[0.,-0.015,0.],
                                            "half_extents":[0.1,0.01,0.1],"grip":"palm_top"}));
        let latch = Transform::from_translation(rig.palm.palm() + rig.palm.normal * 0.05)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, rig.palm.normal) * Quat::from_rotation_x(0.4));
        let seated = seat(Grip::PalmTop, &under, &plate, &rig.palm, &latch, crate::cc_render::Hand::Right).unwrap();
        assert!((seated.rotation * Vec3::Y).dot(rig.palm.normal) > 0.999);
        let angles = close_hand(&rig, &plate, &seated);
        for i in 1..5 {
            assert!(angles[i].iter().sum::<f32>() < 0.6, "{} curled under the plate: {:?}", FINGERS[i], angles[i]);
        }
    }

    #[test]
    fn the_broom_is_grabbed_with_both_hands_apart_from_their_own_sides() {
        let props: Vec<serde_json::Value> = serde_json::from_str(include_str!("interaction_props.json")).unwrap();
        let broom = props.iter().find(|p| p["id"] == "broom").unwrap();
        let grip = |id: &str| -> Zone {
            serde_json::from_value(broom["zones"].as_array().unwrap().iter().find(|z| z["id"] == id).unwrap().clone()).unwrap()
        };
        let (upper, lower) = (grip("grip_upper"), grip("grip_lower"));
        // Nearest points of the two grips: hands never latch closer than the two-hand span
        // (where the stick would switch to one-hand rigid).
        let gap = Vec3::from(upper.a.unwrap()).distance(Vec3::from(lower.b.unwrap()));
        assert!(gap >= 0.26, "grips {gap} m apart");
        for zone in [&upper, &lower] {
            // Each hand wraps the stick from its own side, palm onto it: right from -X, left from +X.
            assert_eq!(zone.palm, Some(super::super::interaction::PalmSide::PerHand { left: [-1., 0., 0.], right: [1., 0., 0.] }));
            assert!(zone.validate().is_ok());
        }
        // Only the grab is constrained: both hands latch, and the phases after it declare no
        // relations, so the motion plays out with the broom in both hands.
        let interactions = broom["interactions"].as_array().unwrap();
        let reach = &interactions[0]["phases"][1];
        assert_eq!(reach["transition"]["anchors"].as_array().unwrap().len(), 2);
        for (interaction, phase) in [(0, 2), (1, 0), (2, 0)] {
            assert!(interactions[interaction]["phases"][phase].get("relations").is_none());
        }
        let mut bad = upper.clone();
        bad.grip = Some(Grip::PalmTop);
        assert!(bad.validate().is_err(), "palm facing needs a power grip");
        let mut along = upper.clone();
        along.palm = Some(super::super::interaction::PalmSide::Either([0., 1., 0.]));
        assert!(along.validate().is_err(), "a palm direction along the handle says nothing");
        // A one-hand seat turns the handle so the declared side faces the palm.
        let rig = right_hand();
        let stick: super::super::OfficeObject = serde_json::from_value(serde_json::json!({"id":"broom","label":"b",
            "position":[0.,0.,0.],"half_extents":[0.1,0.66,0.15],"color":[1.,1.,1.],"dynamic":true,"approach":[0.,0.,-0.6],
            "parts":broom["parts"].clone()})).unwrap();
        let latch = Transform::from_translation(rig.palm.grip(0.015))
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, rig.palm.lateral) * Quat::from_rotation_y(2.5));
        let seated = seat(Grip::Power, &upper, &stick, &rig.palm, &latch, crate::cc_render::Hand::Right).unwrap();
        assert!((seated.rotation * Vec3::X).dot(rig.palm.normal) > 0.999);
    }

    #[test]
    fn a_power_grip_point_lies_on_the_handle_axis() {
        let grip = zone(serde_json::json!({"id":"grip_upper","type":"segment","a":[0.,0.2,0.],"b":[0.,0.5,0.],
                                           "radius":0.12,"grip":"power"}));
        // A wrist latched 9 cm off the stick inside the reach tolerance still grips the stick itself.
        assert_eq!(grip.axis_point(Vec3::new(0.09, 0.3, 0.)), Vec3::new(0., 0.3, 0.));
        assert_eq!(grip.axis_point(Vec3::new(0., 0.9, 0.05)), Vec3::new(0., 0.5, 0.));
    }

    #[test]
    fn the_seat_blends_in_from_the_latch_pose() {
        let (a, b) = (Transform::from_xyz(0., 0., 0.), Transform::from_xyz(1., 0., 0.).with_rotation(Quat::from_rotation_y(1.)));
        assert_eq!(seat_blend(&a, &b, 0.), a);
        assert!(seat_blend(&a, &b, SEAT_BLEND_S).translation.distance(b.translation) < 1e-6);
        let half = seat_blend(&a, &b, SEAT_BLEND_S / 2.).translation.x;
        assert!((half - 0.5).abs() < 1e-4);
    }
}
