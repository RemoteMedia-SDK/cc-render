//! Declared object interactions (openspec add-generic-object-interactions).
//!
//! Objects declare zones in their own frame and interactions as phases of
//! body-to-zone relations, coupling transitions and outcome checks. This is
//! data only: nothing here authors a pose. Relations are evaluated on the
//! observed (rendered) avatar, so phase completion is always an observation.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const MAX_ZONES: usize = 32;
pub const MAX_INTERACTIONS: usize = 16;
pub const MAX_PHASES: usize = 8;
pub const MAX_PROMPT_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneKind { Point, Segment, Box, Patch, Sphere, Support, WorkingPoint }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grip { Power, PalmTop, Pinch }

/// An object-local region. Geometry fields used depend on `kind`:
/// point/sphere/working_point: `center` (+ `radius`); segment: `a`, `b`, `radius`;
/// box: `center`, `half_extents`; patch/support: a horizontal rectangle at
/// `center` with `half_extents` x/z (y is the thickness tolerance above it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ZoneKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_extents: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grip: Option<Grip>,
    /// Object-local direction a `power` grip's palm faces at the grasp point (across the
    /// handle): which side of the handle the hand wraps from, for both hands or per hand.
    /// It turns with the object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palm: Option<PalmSide>,
    /// A `power` handle whose object must stay upright while carried (a mug's contents):
    /// the motion service keeps the holding hand's wrap axis vertical.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub upright: bool,
}

/// A grip's palm direction: one for either hand, or one per hand (each wraps from its side).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PalmSide {
    Either([f32; 3]),
    PerHand {
        #[serde(rename = "LeftHand")] left: [f32; 3],
        #[serde(rename = "RightHand")] right: [f32; 3],
    },
}

impl PalmSide {
    pub fn for_hand(&self, hand: crate::cc_render::Hand) -> [f32; 3] {
        match *self {
            Self::Either(v) => v,
            Self::PerHand { left, right } => if hand == crate::cc_render::Hand::Left { left } else { right },
        }
    }
    fn directions(&self) -> Vec<[f32; 3]> {
        match *self { Self::Either(v) => vec![v], Self::PerHand { left, right } => vec![left, right] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Joint { LeftHand, RightHand, LeftFoot, RightFoot, Hips }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Require { All, Any }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Body { Feet, Pelvis }

/// Body-to-zone relations. `zone` is a zone id on the interaction's object, or
/// `other_object/zone` for a zone on another object. Windows are seconds from
/// the start of the phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Relation {
    Touch { joints: Vec<Joint>, zone: String, window_s: [f32; 2], min_frames: u32,
            #[serde(default)] tolerance_m: f32 },
    Hold { joints: Vec<Joint>, require: Require, zone: String, window_s: [f32; 2],
           #[serde(default)] tolerance_m: f32 },
    Support { body: Body, zone: String, window_s: [f32; 2], #[serde(default)] tolerance_m: f32 },
    Pair { joints: [Joint; 2], distance_m: [f32; 2], window_s: [f32; 2] },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor { pub joint: Joint, pub zone: String }

/// Coupling changes applied by the renderer when a phase's relations are met.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transition {
    /// Attach to 1..N hand anchors; the first anchor is primary (roll source).
    Attach { anchors: Vec<Anchor>,
             /// Sliding attach: keep the object on this support/patch zone's plane.
             #[serde(default, skip_serializing_if = "Option::is_none")] slide_on: Option<String> },
    /// Release to the object's authored home transform, or onto `onto`
    /// (`object/zone` support or patch zone), keeping yaw.
    Release { #[serde(default)] onto: Option<String> },
    SupportOn { zone: String },
    SupportOff,
    /// Grip a handle physically with one hand (openspec add-declared-articulated-grips): the
    /// renderer's contact grip, admitted only on actual shape contact, joints the hand to the
    /// part; the phase passes once that grip is observed. A later `release` lets go.
    Grip { anchor: Anchor },
}

/// Observed joint travel a phase must reach while its object stays gripped: the part moves
/// only because the gripped hand drags it along its passive rail. Either bound may be open.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Travel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_m: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_m: Option<f32>,
}

impl Travel {
    pub fn contains(&self, travel_m: f32) -> bool {
        self.min_m.is_none_or(|m| travel_m >= m) && self.max_m.is_none_or(|m| travel_m <= m)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandAt {
    pub zone: String,
    /// Object-local facing, degrees (0 = local +Z); default faces the object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing_deg: Option<f32>,
}

/// A phase whose hand assignment is interchangeable: the generated motion's own hands pick
/// which grip each takes (openspec add-conforming-hand-grasps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandChoice { Either }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionPhase {
    pub id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hands: Option<HandChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stand_at: Option<StandAt>,
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Pass on observed joint travel instead of relations (which then only guide the motion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub travel: Option<Travel>,
    pub timeout_s: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interaction {
    pub id: String,
    pub phases: Vec<InteractionPhase>,
}

fn finite(v: &[f32]) -> bool { v.iter().all(|x| x.is_finite()) }

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Zone {
    pub fn validate(&self) -> Result<(), String> {
        let bad = |what: &str| Err(format!("zone {} {what}", self.id));
        if !valid_id(&self.id) { return Err(format!("invalid zone id {:?}", self.id)); }
        let positive = |v: Option<f32>| v.is_some_and(|r| r.is_finite() && r > 0. && r <= 5.);
        let vector = |v: Option<[f32; 3]>| v.is_some_and(|v| finite(&v) && v.iter().all(|x| x.abs() <= 20.));
        let half = |v: Option<[f32; 3]>| v.is_some_and(|v| finite(&v) && v.iter().all(|x| *x >= 0. && *x <= 10.));
        let ok = match self.kind {
            ZoneKind::Point | ZoneKind::Sphere => vector(self.center) && positive(self.radius),
            ZoneKind::WorkingPoint => vector(self.center),
            ZoneKind::Segment => vector(self.a) && vector(self.b) && positive(self.radius)
                && self.a.zip(self.b).is_some_and(|(a, b)| Vec3::from(a).distance(Vec3::from(b)) > 1e-3),
            ZoneKind::Box => vector(self.center) && half(self.half_extents)
                && self.half_extents.is_some_and(|h| h.iter().all(|x| *x > 0.)),
            ZoneKind::Patch | ZoneKind::Support => vector(self.center) && half(self.half_extents)
                && self.half_extents.is_some_and(|h| h[0] > 0. && h[2] > 0.),
        };
        if !ok { return bad("has missing or invalid geometry for its type"); }
        if self.grip.is_some() && !matches!(self.kind, ZoneKind::Segment | ZoneKind::Point | ZoneKind::Box | ZoneKind::Patch) {
            return bad("grip hint requires a graspable zone");
        }
        if let Some(side) = self.palm {
            let axis = Vec3::from(self.b.unwrap_or_default()) - Vec3::from(self.a.unwrap_or_default());
            let across = |palm: Vec3| palm.is_finite() && palm.length() >= 1e-3
                && palm.normalize().cross(axis.normalize_or_zero()).length() >= 0.5;
            if !(self.grip == Some(Grip::Power) && self.kind == ZoneKind::Segment)
                || !side.directions().into_iter().all(|v| across(Vec3::from(v))) {
                return bad("palm facing requires a power grip on a segment and a direction across it");
            }
        }
        if self.upright && !(self.grip == Some(Grip::Power) && self.kind == ZoneKind::Segment) {
            return bad("upright carry requires a power grip on a segment");
        }
        Ok(())
    }

    /// Distance (m) from an object-local point to the zone (0 inside).
    pub fn distance_local(&self, p: Vec3) -> f32 {
        let center = Vec3::from(self.center.unwrap_or_default());
        match self.kind {
            ZoneKind::Point | ZoneKind::Sphere => (p.distance(center) - self.radius.unwrap_or(0.)).max(0.),
            ZoneKind::WorkingPoint => p.distance(center),
            ZoneKind::Segment => {
                let (a, b) = (Vec3::from(self.a.unwrap()), Vec3::from(self.b.unwrap()));
                let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0., 1.);
                (p.distance(a + (b - a) * t) - self.radius.unwrap_or(0.)).max(0.)
            }
            ZoneKind::Box => {
                let q = (p - center).abs() - Vec3::from(self.half_extents.unwrap());
                q.max(Vec3::ZERO).length()
            }
            ZoneKind::Patch | ZoneKind::Support => {
                let h = self.half_extents.unwrap();
                let d = p - center;
                let lateral = Vec2::new((d.x.abs() - h[0]).max(0.), (d.z.abs() - h[2]).max(0.));
                let vertical = if d.y < 0. { -d.y } else { (d.y - h[1]).max(0.) };
                (lateral.length_squared() + vertical * vertical).sqrt()
            }
        }
    }
}

impl Zone {
    /// The point on a segment zone's axis nearest `p` (other zones: `p` itself).
    pub fn axis_point(&self, p: Vec3) -> Vec3 {
        match (self.kind, self.a, self.b) {
            (ZoneKind::Segment, Some(a), Some(b)) => {
                let (a, b) = (Vec3::from(a), Vec3::from(b));
                let length = a.distance(b);
                if length < 1e-6 { return a; }
                a + (b - a) / length * (p - a).dot((b - a) / length).clamp(0., length)
            }
            _ => p,
        }
    }

    /// Nearest object-local point inside the zone (a grip latches here).
    pub fn nearest_local(&self, p: Vec3) -> Vec3 {
        let center = Vec3::from(self.center.unwrap_or_default());
        let toward = |c: Vec3, r: f32| { let d = p - c; if d.length() <= r { p } else { c + d.normalize_or_zero() * r } };
        match self.kind {
            ZoneKind::Point | ZoneKind::Sphere => toward(center, self.radius.unwrap_or(0.)),
            ZoneKind::WorkingPoint => center,
            ZoneKind::Segment => {
                let (a, b) = (Vec3::from(self.a.unwrap()), Vec3::from(self.b.unwrap()));
                let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0., 1.);
                toward(a + (b - a) * t, self.radius.unwrap_or(0.))
            }
            ZoneKind::Box => { let h = Vec3::from(self.half_extents.unwrap()); p.clamp(center - h, center + h) }
            ZoneKind::Patch | ZoneKind::Support => {
                let h = self.half_extents.unwrap();
                p.clamp(center - Vec3::new(h[0], 0., h[2]), center + Vec3::new(h[0], h[1], h[2]))
            }
        }
    }
}

/// Declared zones plus legacy affordances as zones of the same name (approach:
/// point r=0.25, grasp: point r=0.02, seat: support on the seat slab top).
pub fn all_zones(object: &super::OfficeObject) -> Vec<Zone> {
    let mut zones = object.zones.clone();
    let mut add = |zone: Zone| if !zones.iter().any(|z| z.id == zone.id) { zones.push(zone) };
    let point = |id: &str, c: [f32; 3], r: f32| Zone { id: id.into(), kind: ZoneKind::Point, center: Some(c),
        radius: Some(r), a: None, b: None, half_extents: None, grip: None, palm: None, upright: false };
    add(point("approach", object.local_keypoint("approach").map_or(object.approach, |t| t.translation.to_array()), 0.25));
    if let Some(c) = object.local_keypoint("grasp").map(|t| t.translation.to_array()).or(object.grasp) {
        add(point("grasp", c, 0.02));
    }
    if let Some(part) = object.seat_surface_part.and_then(|i| object.parts.get(i)) {
        add(Zone { id: "seat".into(), kind: ZoneKind::Support,
            center: Some([part.position[0], part.position[1] + part.half_extents[1], part.position[2]]),
            radius: None, a: None, b: None, half_extents: Some([part.half_extents[0], 0.05, part.half_extents[2]]), grip: None, palm: None,
            upright: false });
    }
    zones
}

/// Resolve `zone`, `object/zone` or `@home/zone` (this object's authored pose)
/// to the zone and the world transform its coordinates are in.
pub fn resolve_zone(owner: &super::OfficeObject, reference: &str, objects: &[super::OfficeObject],
                    homes: &std::collections::BTreeMap<String, [f32; 7]>) -> Option<(Zone, Transform)> {
    let (object, name, home) = match reference.split_once('/') {
        Some(("@home", name)) => (owner, name, true),
        Some((id, name)) => (objects.iter().find(|o| o.id == id)?, name, false),
        None => (owner, reference, false),
    };
    let zone = all_zones(object).into_iter().find(|z| z.id == name)?;
    let transform = if home {
        let h = homes.get(&object.id)?;
        Transform::from_translation(Vec3::new(h[0], h[1], h[2])).with_rotation(Quat::from_xyzw(h[3], h[4], h[5], h[6]))
    } else { object.transform() };
    Some((zone, transform))
}

pub fn zone_distance(zone: &Zone, transform: &Transform, world: Vec3) -> f32 {
    zone.distance_local(transform.rotation.inverse() * (world - transform.translation))
}

/// Coupling changes the lifecycle asks the controller to apply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CouplingCommand {
    /// Hands with their object-local grip points (primary first) and grip hints.
    Attach { object: String, anchors: Vec<(crate::cc_render::Hand, [f32; 3], Option<Grip>)>,
             #[serde(default)] slide_on: Option<String> },
    Release { object: String, onto: Option<String> },
    SupportOn { object: String },
    SupportOff { object: String },
    /// A physical contact grip request (the renderer's grab, never a declared coupling).
    Grip { object: String, hand: crate::cc_render::Hand },
}

impl Relation {
    pub fn zone(&self) -> &str {
        match self { Self::Touch { zone, .. } | Self::Hold { zone, .. } | Self::Support { zone, .. } => zone,
                     Self::Pair { .. } => "" }
    }
    pub fn window_s(&self) -> [f32; 2] {
        match self { Self::Touch { window_s, .. } | Self::Hold { window_s, .. }
                     | Self::Support { window_s, .. } | Self::Pair { window_s, .. } => *window_s }
    }
}

/// Validate one object's zones and interactions; `others` resolves `object/zone` references.
pub fn validate(object_id: &str, zones: &[Zone], interactions: &[Interaction],
                others: &dyn Fn(&str, &str) -> bool) -> Result<(), String> {
    if zones.len() > MAX_ZONES || interactions.len() > MAX_INTERACTIONS {
        return Err(format!("{object_id} declares too many zones or interactions"));
    }
    let mut ids = std::collections::HashSet::new();
    for zone in zones {
        zone.validate()?;
        if !ids.insert(zone.id.as_str()) { return Err(format!("{object_id} repeats zone {}", zone.id)); }
    }
    let legacy = ["approach", "grasp", "seat"];
    let resolves = |reference: &str| match reference.split_once('/') {
        Some(("@home", zone)) => ids.contains(zone) || legacy.contains(&zone),
        Some((object, zone)) => others(object, zone),
        None => ids.contains(reference) || legacy.contains(&reference),
    };
    let mut interaction_ids = std::collections::HashSet::new();
    for interaction in interactions {
        if !valid_id(&interaction.id) || !interaction_ids.insert(interaction.id.as_str())
            || interaction.phases.is_empty() || interaction.phases.len() > MAX_PHASES {
            return Err(format!("{object_id} interaction {:?} is invalid", interaction.id));
        }
        let mut phase_ids = std::collections::HashSet::new();
        let mut gripping = false;
        for phase in &interaction.phases {
            let bad = |what: &str| Err(format!("{object_id}/{}/{} {what}", interaction.id, phase.id));
            if !valid_id(&phase.id) || !phase_ids.insert(phase.id.as_str()) { return bad("has an invalid id"); }
            if phase.prompt.trim().is_empty() || phase.prompt.chars().count() > MAX_PROMPT_CHARS
                || phase.prompt.chars().any(char::is_control) {
                return bad("has an invalid prompt");
            }
            if !phase.timeout_s.is_finite() || !(1. ..=120.).contains(&phase.timeout_s) {
                return bad("needs a timeout of 1..120 s");
            }
            if phase.stand_at.as_ref().is_some_and(|s| !resolves(&s.zone) || s.facing_deg.is_some_and(|f| !f.is_finite())) {
                return bad("stands at an unknown zone");
            }
            for relation in &phase.relations {
                let [t0, t1] = relation.window_s();
                if !(t0.is_finite() && t1.is_finite() && 0. <= t0 && t0 < t1 && t1 <= phase.timeout_s) {
                    return bad("has a relation window outside the phase");
                }
                let ok = match relation {
                    Relation::Touch { joints, zone, min_frames, tolerance_m, .. } =>
                        !joints.is_empty() && resolves(zone) && (1..=200).contains(min_frames) && (0. ..=0.2).contains(tolerance_m),
                    Relation::Hold { joints, zone, tolerance_m, .. } =>
                        !joints.is_empty() && resolves(zone) && (0. ..=0.2).contains(tolerance_m),
                    Relation::Support { zone, tolerance_m, .. } => resolves(zone) && (0. ..=0.2).contains(tolerance_m),
                    Relation::Pair { joints, distance_m: [lo, hi], .. } =>
                        joints[0] != joints[1] && lo.is_finite() && hi.is_finite() && 0. <= *lo && lo < hi && *hi <= 3.,
                };
                if !ok { return bad("has an invalid relation"); }
            }
            let ok = match &phase.transition {
                None | Some(Transition::SupportOff) | Some(Transition::Release { onto: None }) => true,
                Some(Transition::Attach { anchors, slide_on }) => !anchors.is_empty() && anchors.len() <= 4
                    && slide_on.as_ref().is_none_or(|zone| resolves(zone))
                    && anchors.iter().all(|a| matches!(a.joint, Joint::LeftHand | Joint::RightHand) && resolves(&a.zone))
                    && (anchors.len() < 2 || anchors[0].joint != anchors[1].joint),
                Some(Transition::Release { onto: Some(zone) }) => zone.contains('/') && resolves(zone),
                Some(Transition::SupportOn { zone }) => resolves(zone),
                // One hand on a handle of this object, held with a grip hint.
                Some(Transition::Grip { anchor }) => !gripping && hand(anchor.joint).is_some()
                    && zones.iter().any(|z| z.id == anchor.zone && matches!(z.grip, Some(Grip::Power | Grip::Pinch))),
            };
            if !ok { return bad("has an invalid transition"); }
            if let Some(travel) = &phase.travel {
                let bound = |m: Option<f32>| m.is_none_or(|m| m.is_finite() && (0. ..=2.).contains(&m));
                if !gripping || (travel.min_m.is_none() && travel.max_m.is_none()) || !bound(travel.min_m)
                    || !bound(travel.max_m) || travel.min_m.zip(travel.max_m).is_some_and(|(lo, hi)| lo > hi)
                    || matches!(phase.transition, Some(Transition::Grip { .. })) {
                    return bad("needs a travel range while its object is gripped");
                }
            }
            match &phase.transition {
                Some(Transition::Grip { .. }) => gripping = true,
                Some(Transition::Release { .. }) => gripping = false,
                _ => {}
            }
        }
    }
    Ok(())
}

impl Joint {
    /// The other hand (feet and pelvis are unchanged).
    pub fn mirrored(self) -> Self {
        match self { Self::LeftHand => Self::RightHand, Self::RightHand => Self::LeftHand, other => other }
    }
}

impl InteractionPhase {
    /// This phase with its left and right hands exchanged in every relation and anchor.
    pub fn swapped_hands(&self) -> Self {
        let mut phase = self.clone();
        for relation in &mut phase.relations {
            match relation {
                Relation::Touch { joints, .. } | Relation::Hold { joints, .. } =>
                    joints.iter_mut().for_each(|j| *j = j.mirrored()),
                Relation::Pair { joints, .. } => joints.iter_mut().for_each(|j| *j = j.mirrored()),
                Relation::Support { .. } => {}
            }
        }
        match &mut phase.transition {
            Some(Transition::Attach { anchors, .. }) => anchors.iter_mut().for_each(|a| a.joint = a.joint.mirrored()),
            Some(Transition::Grip { anchor }) => anchor.joint = anchor.joint.mirrored(),
            _ => {}
        }
        phase
    }
}

/// Observed joint positions (world) for a relation joint.
pub fn hand(joint: Joint) -> Option<crate::cc_render::Hand> {
    match joint { Joint::LeftHand => Some(crate::cc_render::Hand::Left),
                  Joint::RightHand => Some(crate::cc_render::Hand::Right), _ => None }
}

pub fn observed_joint(joint: Joint, wrists: &[[f32; 3]; 2], feet: &[[f32; 3]; 2], pelvis: [f32; 3]) -> Vec3 {
    Vec3::from(match joint {
        Joint::LeftHand => wrists[0], Joint::RightHand => wrists[1],
        Joint::LeftFoot => feet[0], Joint::RightFoot => feet[1], Joint::Hips => pelvis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(json: serde_json::Value) -> Zone { serde_json::from_value(json).unwrap() }

    #[test]
    fn zone_geometry_measures_distance_in_the_object_frame() {
        let handle = zone(serde_json::json!({"id":"grip","type":"segment","a":[0.,0.4,0.],"b":[0.,1.2,0.],"radius":0.03,"grip":"power"}));
        handle.validate().unwrap();
        // Only a power handle can be carried upright.
        zone(serde_json::json!({"id":"h","type":"segment","a":[0.,0.,0.],"b":[0.,0.1,0.],"radius":0.03,"grip":"power",
                                "upright":true})).validate().unwrap();
        assert!(zone(serde_json::json!({"id":"t","type":"support","center":[0.,0.3,0.],"half_extents":[0.2,0.05,0.15],
                                        "upright":true})).validate().is_err());
        assert_eq!(handle.distance_local(Vec3::new(0.02, 0.8, 0.)), 0.);
        assert!((handle.distance_local(Vec3::new(0.13, 0.8, 0.)) - 0.10).abs() < 1e-5);
        assert!((handle.distance_local(Vec3::new(0., 1.5, 0.)) - 0.27).abs() < 1e-5);
        let top = zone(serde_json::json!({"id":"top","type":"support","center":[0.,0.3,0.],"half_extents":[0.2,0.05,0.15]}));
        top.validate().unwrap();
        assert_eq!(top.distance_local(Vec3::new(0.1, 0.32, 0.1)), 0.);
        assert!((top.distance_local(Vec3::new(0., 0.25, 0.)) - 0.05).abs() < 1e-5);
        assert!((top.distance_local(Vec3::new(0.3, 0.3, 0.)) - 0.10).abs() < 1e-5);
    }

    #[test]
    fn bundled_declarations_validate_and_reach_their_objects() {
        let objects = super::super::starter_objects();
        super::super::validate_objects(&objects).unwrap();
        let declared = |id: &str| objects.iter().find(|o| o.id == id).unwrap().interactions.iter()
            .map(|i| i.id.clone()).collect::<Vec<_>>();
        assert_eq!(declared("broom"), ["pick_up", "hold", "put_back"]);
        assert_eq!(declared("plate"), ["pick_up", "place_on_table", "place_in_sink"]);
        assert_eq!(declared("toilet"), ["stand_up"]);
        assert!(declared("shelf").is_empty());
    }

    #[test]
    fn invalid_zones_and_interactions_are_rejected() {
        assert!(zone(serde_json::json!({"id":"p","type":"point","center":[0.,0.,0.]})).validate().is_err());
        assert!(serde_json::from_value::<Zone>(serde_json::json!({"id":"p","type":"point","center":[0.,0.,0.],
            "radius":0.1,"pose":[1]})).is_err(), "unknown fields fail closed");
        let zones = vec![zone(serde_json::json!({"id":"grip","type":"segment","a":[0.,0.4,0.],"b":[0.,1.2,0.],"radius":0.03}))];
        let none = |_: &str, _: &str| false;
        let interaction = |relation: serde_json::Value| -> Interaction {
            serde_json::from_value(serde_json::json!({"id":"pick_up","phases":[{"id":"reach","prompt":"A person grabs it.",
                "relations":[relation],"timeout_s":10.}]})).unwrap()
        };
        let ok = interaction(serde_json::json!({"kind":"touch","joints":["RightHand"],"zone":"grip","window_s":[0.,8.],"min_frames":3}));
        validate("broom", &zones, &[ok], &none).unwrap();
        for bad in [
            serde_json::json!({"kind":"touch","joints":["RightHand"],"zone":"missing","window_s":[0.,8.],"min_frames":3}),
            serde_json::json!({"kind":"touch","joints":[],"zone":"grip","window_s":[0.,8.],"min_frames":3}),
            serde_json::json!({"kind":"touch","joints":["RightHand"],"zone":"grip","window_s":[0.,30.],"min_frames":3}),
            serde_json::json!({"kind":"pair","joints":["LeftHand","LeftHand"],"distance_m":[0.3,0.6],"window_s":[0.,8.]}),
        ] {
            assert!(validate("broom", &zones, &[interaction(bad.clone())], &none).is_err(), "{bad}");
        }
        assert!(serde_json::from_value::<Interaction>(serde_json::json!({"id":"x","phases":[{"id":"a","prompt":"p",
            "timeout_s":5.,"joint_positions":[[0,0,0]]}]})).is_err(), "authored poses fail closed");
    }
}
