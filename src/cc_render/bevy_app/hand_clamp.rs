//! Owner-approved exception (docs/design/hand-ribcage-clamp-exception.md):
//! a displayed hand that the generated motion places inside the rigid ribcage
//! is moved to the nearest position just outside by two-bone arm IK, keeping its
//! world orientation. Renderer-owned fingers that would still pass through it
//! bend at the knuckles until they rest on it. Source packets and service
//! history are never modified. The correction is a pure function of one pose:
//! no temporal state.
use super::tissue_motion::{frame, TissueRig};
use super::tissue_surface::{SkinSubset, TissueSurface};
use crate::cc_render::{
    office::hand::{FingerRig, HandRig, FINGERS},
    renderer::SkeletalPose,
    soft_tissue::{Attachment, RegionAsset, TissueAsset},
};
use bevy::{
    math::{DQuat, DVec3},
    prelude::*,
};
use std::{collections::HashMap, sync::Arc};

/// Clearance kept between hand skin and the ribcage surface.
const MARGIN_M: f64 = 0.001;
/// Larger corrections are reported as unsupported contact, not applied.
const MAX_CORRECTION_M: f64 = 0.05;
/// Ribcage = torso skin dominated by these bones (waist, pelvis and neck excluded).
const RIBCAGE_BONES: [&str; 8] = [
    "CC_Base_Spine01", "CC_Base_Spine02", "CC_Base_L_RibsTwist", "CC_Base_R_RibsTwist",
    "CC_Base_L_Clavicle", "CC_Base_R_Clavicle", "CC_Base_L_Breast", "CC_Base_R_Breast",
];
const HAND_BONES: [&str; 6] = ["Hand", "Thumb", "Index", "Mid", "Ring", "Pinky"];
/// Which hand skin must stay outside: the whole hand, or the wrist and palm
/// (vertices dominated by the hand bone; renderer-only fingers excluded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HandPoints { Whole, Palm }
const HAND_POINT_SPACING_M: f32 = 0.008;
/// SMPL upper arm, forearm and wrist per side, as mapped onto CC5.
const ARMS: [([usize; 3], [&str; 3]); 2] = [
    ([16, 18, 20], ["CC_Base_L_Upperarm", "CC_Base_L_Forearm", "CC_Base_L_Hand"]),
    ([17, 19, 21], ["CC_Base_R_Upperarm", "CC_Base_R_Forearm", "CC_Base_R_Hand"]),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub(crate) struct HandCorrection {
    /// Deepest hand point inside the ribcage before correction.
    pub penetration_m: f64,
    /// Applied wrist translation.
    pub correction_m: f64,
    /// Deepest remaining penetration after the applied correction.
    pub residual_m: f64,
    pub unsupported: bool,
    /// Deepest finger sphere inside the ribcage at the corrected arm pose.
    pub finger_penetration_m: f64,
    /// Largest finger bend applied (radians; negative is extension).
    pub finger_bend_rad: f64,
    /// Deepest finger sphere still inside after bending.
    pub finger_residual_m: f64,
}

/// Corrected arm rotations plus local rotations for every finger bone.
pub(crate) struct Correction {
    pub rotations: [Quat; 22],
    pub fingers: HashMap<Entity, Quat>,
    pub report: [HandCorrection; 2],
}

/// A triangle with its outward normal and which edges/vertices are open
/// boundary of the selected surface (a point closest to those is outside).
#[derive(Clone, Copy, Debug)]
pub(super) struct Face {
    pub points: [DVec3; 3],
    pub normal: DVec3,
    pub open_edges: [bool; 3],
}

struct Finger {
    bones: [Entity; 3],
    rig: FingerRig,
}

struct Arm {
    smpl: [usize; 3],
    bones: [Entity; 3],
    points: SkinSubset,
    fingers: [Finger; 5],
}

struct Cap {
    anchor: Entity,
    faces: Vec<[DVec3; 3]>,
    open_edges: Vec<[bool; 3]>,
}

pub(crate) struct HandRibcageClamp {
    rig: TissueRig,
    torso: SkinSubset,
    torso_faces: Vec<[usize; 3]>,
    torso_open: Vec<[bool; 3]>,
    caps: Vec<Cap>,
    arms: [Arm; 2],
}

#[derive(Resource, Clone)]
pub(crate) struct HandClampResource(pub Arc<HandRibcageClamp>);

/// Open edges of a triangle set: edges used by exactly one triangle.
fn open_edges<T: Copy + Ord>(faces: &[[T; 3]]) -> Vec<[bool; 3]> {
    let key = |a: T, b: T| if a < b { (a, b) } else { (b, a) };
    let mut count = std::collections::BTreeMap::new();
    for f in faces {
        for e in 0..3 {
            *count.entry(key(f[e], f[(e + 1) % 3])).or_insert(0) += 1;
        }
    }
    faces.iter().map(|f| std::array::from_fn(|e| count[&key(f[e], f[(e + 1) % 3])] == 1)).collect()
}

/// Rig bones by name, for construction-time lookups and chain checks.
struct Bones<'a> {
    rig: &'a TissueRig,
    names: &'a HashMap<Entity, String>,
}

impl Bones<'_> {
    fn name(&self, entity: &Entity) -> &str {
        self.names.get(entity).map(String::as_str).unwrap_or("")
    }

    fn find(&self, wanted: &str) -> Entity {
        self.names.iter().find(|(e, n)| n.as_str() == wanted && self.rig.local_of(**e).is_some())
            .map(|(e, _)| *e).unwrap_or(Entity::PLACEHOLDER)
    }

    /// `chain` is a direct parent-to-child sequence below `root`.
    fn is_chain(&self, root: Entity, chain: &[Entity]) -> bool {
        chain.iter().scan(root, |parent, bone| {
            let ok = self.rig.parent_of(*bone) == Some(*parent);
            *parent = *bone;
            Some(ok)
        }).all(|ok| ok)
    }
}

/// Torso skin triangles dominated by ribcage bones, as a compact vertex subset.
fn ribcage(surface: &TissueSurface, bones: &Bones) -> anyhow::Result<(Vec<usize>, Vec<[usize; 3]>)> {
    let dominant = surface.dominant_joints();
    let selected: Vec<_> = surface.collision_triangles.iter()
        .filter(|t| t.iter().all(|v| RIBCAGE_BONES.contains(&bones.name(&dominant[*v]))))
        .copied().collect();
    anyhow::ensure!(!selected.is_empty(), "hand clamp found no ribcage skin");
    let mut vertices: Vec<usize> = selected.iter().flatten().copied().collect();
    vertices.sort_unstable();
    vertices.dedup();
    let local: HashMap<_, _> = vertices.iter().enumerate().map(|(i, v)| (*v, i)).collect();
    Ok((vertices, selected.iter().map(|t| t.map(|v| local[&v])).collect()))
}

/// The posterior backing of a tissue region's fixed cap: the rigid chest the
/// solver attaches the breast to. Faces are oriented toward the tissue.
fn cap(region: &RegionAsset, anchor: Entity) -> Cap {
    let fixed: Vec<bool> = region.attachments.iter().map(|a| matches!(a, Attachment::Fixed)).collect();
    let p = |i: usize| DVec3::from(region.initial_positions[i]);
    let face = |tet: &[usize; 4], skip: usize| -> [usize; 3] { std::array::from_fn(|k| tet[(skip + 1 + k) % 4]) };
    let sorted = |mut f: [usize; 3]| { f.sort_unstable(); f };
    let mut count = std::collections::BTreeMap::<[usize; 3], usize>::new();
    for tet in &region.tetrahedra {
        (0..4).for_each(|skip| *count.entry(sorted(face(tet, skip))).or_insert(0) += 1);
    }
    let boundary: Vec<(&[usize; 4], usize)> = region.tetrahedra.iter()
        .flat_map(|tet| (0..4).map(move |skip| (tet, skip)))
        .filter(|(tet, skip)| count[&sorted(face(tet, *skip))] == 1)
        .collect();
    // Skin-side rim nodes: fixed nodes on a boundary face with a free node.
    // Faces touching them are the cap's side walls, not chest.
    let mut rim = vec![false; fixed.len()];
    for (tet, skip) in &boundary {
        let f = face(tet, *skip);
        if !f.iter().all(|i| fixed[*i]) {
            f.iter().filter(|i| fixed[**i]).for_each(|i| rim[*i] = true);
        }
    }
    let mut faces: Vec<[usize; 3]> = boundary.iter()
        .filter(|(tet, skip)| face(tet, *skip).iter().all(|i| fixed[*i] && !rim[*i]))
        .map(|(tet, skip)| {
            let mut f = face(tet, *skip);
            if (p(f[1]) - p(f[0])).cross(p(f[2]) - p(f[0])).dot(p(tet[*skip]) - p(f[0])) < 0.0 {
                f.swap(1, 2);
            }
            f
        }).collect();
    // Tetgen adds nodes along the side walls, so topology alone keeps some
    // wall faces. Keep only faces coplanar with the backing: the normal
    // shared by the most faces.
    let normal = |f: &[usize; 3]| (p(f[1]) - p(f[0])).cross(p(f[2]) - p(f[0])).normalize();
    let backing = faces.iter().map(normal)
        .max_by_key(|n| faces.iter().filter(|f| normal(f).dot(*n) > 0.99).count())
        .unwrap_or(DVec3::ZERO);
    faces.retain(|f| normal(f).dot(backing) > 0.99);
    Cap { anchor, open_edges: open_edges(&faces), faces: faces.iter().map(|f| f.map(p)).collect() }
}

/// Hand skin vertices of one side, thinned to one per `HAND_POINT_SPACING_M` cell.
fn hand_vertices(surface: &TissueSurface, bones: &Bones, prefix: &str, points: HandPoints) -> Vec<usize> {
    let dominant = surface.dominant_joints();
    let mut cells = std::collections::BTreeSet::new();
    (0..dominant.len()).filter(|v| {
        let Some(bone) = bones.name(&dominant[*v]).strip_prefix(prefix) else { return false };
        match points {
            HandPoints::Palm => bone == HAND_BONES[0],
            HandPoints::Whole => HAND_BONES.iter().any(|h| bone.starts_with(h)),
        }
    }).filter(|v| {
        let cell = (surface.bind_positions()[*v] / HAND_POINT_SPACING_M).floor().as_ivec3();
        cells.insert(cell.to_array())
    }).collect()
}

/// Finger kinematics from the rig's own rest transforms (flexion axis and
/// closing sign read from the rig, as for grasps).
fn fingers(bones: &Bones, prefix: &str, hand: Entity) -> anyhow::Result<[Finger; 5]> {
    let chains: Vec<[Entity; 3]> = FINGERS.iter()
        .map(|finger| std::array::from_fn(|k| bones.find(&format!("{prefix}{finger}{}", k + 1))))
        .collect();
    for (chain, finger) in chains.iter().zip(FINGERS) {
        anyhow::ensure!(bones.is_chain(hand, chain), "hand clamp finger chain {prefix}{finger}");
    }
    let rests: [[Transform; 3]; 5] = std::array::from_fn(|i| chains[i].map(|e| bones.rig.local_of(e).unwrap()));
    let hand_rig = HandRig::new(rests);
    Ok(std::array::from_fn(|i| Finger { bones: chains[i], rig: hand_rig.fingers[i].clone() }))
}

fn arm(
    bones: &Bones,
    mapped: &[Option<Entity>; 22],
    surface: &TissueSurface,
    (smpl, expected): ([usize; 3], [&str; 3]),
    points: HandPoints,
) -> anyhow::Result<Arm> {
    let chain: [Entity; 3] = std::array::from_fn(|k| mapped[smpl[k]].unwrap_or(Entity::PLACEHOLDER));
    for (bone, expected) in chain.iter().zip(expected) {
        anyhow::ensure!(bones.name(bone) == expected, "hand clamp arm mapping: expected {expected}, found {:?}", bones.name(bone));
    }
    anyhow::ensure!(bones.is_chain(chain[0], &chain[1..]), "hand clamp requires a direct upper arm, forearm, hand chain");
    let prefix = &expected[2][..10]; // "CC_Base_L_" / "CC_Base_R_"
    let vertices = hand_vertices(surface, bones, prefix, points);
    anyhow::ensure!(vertices.len() >= 16, "hand clamp found too few hand skin points");
    Ok(Arm { smpl, bones: chain, points: surface.subset(&vertices), fingers: fingers(bones, prefix, chain[2])? })
}

impl HandRibcageClamp {
    pub(super) fn new(
        rig: &TissueRig,
        surfaces: &[TissueSurface],
        names: &HashMap<Entity, String>,
        mapped: &[Option<Entity>; 22],
        asset: &TissueAsset,
        anchors: &[Entity],
        hand_points: HandPoints,
    ) -> anyhow::Result<Self> {
        let bones = Bones { rig, names };
        let torso = surfaces.iter().find(|s| s.binding().mesh_name == "CC_Base_Body")
            .ok_or_else(|| anyhow::anyhow!("hand clamp requires the tissue-bound body surface"))?;
        let (vertices, torso_faces) = ribcage(torso, &bones)?;
        let hands = surfaces.iter()
            .find(|s| s.dominant_joints().iter().any(|j| bones.name(j) == "CC_Base_L_Hand"))
            .ok_or_else(|| anyhow::anyhow!("hand clamp requires the hand contact surface"))?;
        let [left, right] = ARMS.map(|side| arm(&bones, mapped, hands, side, hand_points));
        Ok(Self {
            rig: rig.clone(),
            torso: torso.subset(&vertices),
            torso_open: open_edges(&torso_faces),
            torso_faces,
            caps: asset.regions.iter().zip(anchors).map(|(region, anchor)| cap(region, *anchor)).collect(),
            arms: [left?, right?],
        })
    }

    pub fn counts(&self) -> (usize, usize, [usize; 2]) {
        (self.torso_faces.len(), self.caps.iter().map(|c| c.faces.len()).sum(),
            [self.arms[0].points.len(), self.arms[1].points.len()])
    }

    fn faces(&self, matrices: &HashMap<Entity, Mat4>) -> anyhow::Result<Vec<Face>> {
        let torso = self.torso.positions(matrices)?;
        let mut faces: Vec<Face> = self.torso_faces.iter().zip(&self.torso_open).map(|(f, open)| {
            let points = f.map(|i| torso[i]);
            Face { points, normal: (points[1] - points[0]).cross(points[2] - points[0]).normalize_or_zero(), open_edges: *open }
        }).collect();
        for cap in &self.caps {
            let anchor = frame(*matrices.get(&cap.anchor).ok_or_else(|| anyhow::anyhow!("missing cap anchor"))?)?;
            faces.extend(cap.faces.iter().zip(&cap.open_edges).map(|(f, open)| {
                let points = f.map(|p| anchor.point(p));
                Face { points, normal: (points[1] - points[0]).cross(points[2] - points[0]).normalize_or_zero(), open_edges: *open }
            }));
        }
        Ok(faces)
    }

    /// Corrected CC5 local rotations for `rotations` (the retarget of `pose`).
    pub fn correct(&self, pose: &SkeletalPose, mut rotations: [Quat; 22]) -> anyhow::Result<Correction> {
        let matrices = self.rig.sample_rotations(pose, &rotations);
        let faces = self.faces(&matrices)?;
        let mut report = [HandCorrection::default(); 2];
        for (arm, report) in self.arms.iter().zip(&mut report) {
            let points = arm.points.positions(&matrices)?;
            let nearby = nearby_faces(&faces, &points, MAX_CORRECTION_M + MARGIN_M);
            let push = push_out(&points, &nearby, MARGIN_M);
            report.penetration_m = push.penetration;
            if push.delta == DVec3::ZERO {
                continue;
            }
            // An unconverged but bounded correction is still applied (and its
            // residual reported): withholding it would make the hand jump.
            if push.delta.length() > MAX_CORRECTION_M {
                report.unsupported = true;
                continue;
            }
            let world = |e: Entity| -> anyhow::Result<(DQuat, DVec3)> {
                let (_, r, t) = matrices.get(&e).ok_or_else(|| anyhow::anyhow!("missing arm bone"))?
                    .to_scale_rotation_translation();
                Ok((DQuat::from_array(r.to_array().map(f64::from)).normalize(), t.as_dvec3()))
            };
            let [upper, fore, hand] = arm.bones;
            let (upper_rotation, shoulder) = world(upper)?;
            let (fore_rotation, elbow) = world(fore)?;
            let (hand_rotation, wrist) = world(hand)?;
            let parent = self.rig.parent_of(upper).ok_or_else(|| anyhow::anyhow!("upper arm parent"))?;
            let (parent_rotation, _) = world(parent)?;
            let Some((new_upper, new_fore)) = two_bone(shoulder, elbow, wrist, wrist + push.delta, upper_rotation, fore_rotation) else {
                report.unsupported = true;
                continue;
            };
            let local = |q: DQuat| Quat::from_array(q.normalize().to_array().map(|v| v as f32));
            rotations[arm.smpl[0]] = local(parent_rotation.inverse() * new_upper);
            rotations[arm.smpl[1]] = local(new_upper.inverse() * new_fore);
            rotations[arm.smpl[2]] = local(new_fore.inverse() * hand_rotation);
            report.correction_m = push.delta.length();
        }
        // Residual against the same ribcage (torso and caps do not depend on arms).
        let matrices = self.rig.sample_rotations(pose, &rotations);
        let mut fingers = HashMap::new();
        for (arm, report) in self.arms.iter().zip(&mut report) {
            if report.correction_m > 0.0 {
                let points = arm.points.positions(&matrices)?;
                report.residual_m = push_out(&points, &nearby_faces(&faces, &points, MARGIN_M), 0.0).penetration;
            }
            let hand = *matrices.get(&arm.bones[2]).ok_or_else(|| anyhow::anyhow!("missing hand bone"))?;
            let nearby = nearby_faces(&faces, &[hand.w_axis.truncate().as_dvec3()], FINGER_REACH_M);
            for finger in &arm.fingers {
                let fit = conform_finger(&finger.rig, hand, &nearby, MARGIN_M);
                report.finger_penetration_m = report.finger_penetration_m.max(fit.penetration);
                report.finger_residual_m = report.finger_residual_m.max(fit.residual);
                if fit.bend.abs() > report.finger_bend_rad.abs() {
                    report.finger_bend_rad = fit.bend;
                }
                for (k, bone) in finger.bones.iter().enumerate() {
                    fingers.insert(*bone, finger.rig.posed(1.0, k, fit.angles[k]).rotation);
                }
            }
        }
        Ok(Correction { rotations, fingers, report })
    }
}

/// Faces within this distance of the wrist can touch a finger.
const FINGER_REACH_M: f64 = 0.15;
/// Knuckle (first joint) hyperextension allowed to lift a fingertip out.
const FINGER_EXTENSION_RAD: f32 = 0.5;
const FINGER_STEP_RAD: f32 = 0.05;

pub(super) struct FingerFit {
    pub angles: [f32; 3],
    /// Signed bend parameter: flexion > 0, knuckle extension < 0.
    pub bend: f64,
    pub penetration: f64,
    pub residual: f64,
}

/// Bisect between a penetrating and a clear bend to the clear side of the
/// boundary, so the chosen bend varies continuously with the pose.
fn boundary(violation: &impl Fn(f32) -> f64, mut inside: f32, mut outside: f32) -> f32 {
    for _ in 0..12 {
        let middle = 0.5 * (inside + outside);
        if violation(middle) <= 0.0 { outside = middle } else { inside = middle }
    }
    outside
}

/// Joint angles for one bend parameter: flexion spread over all joints
/// within the rig limits, or extension at the knuckle only.
fn finger_angles(rig: &FingerRig, bend: f32) -> [f32; 3] {
    if bend >= 0.0 {
        [bend.min(rig.limits[0]), bend.min(rig.limits[1]), (0.8 * bend).min(rig.limits[2])]
    } else {
        [bend.max(-FINGER_EXTENSION_RAD), 0.0, 0.0]
    }
}

/// Smallest bend that keeps the finger's spheres (joints, midpoints and tip,
/// at the rig's finger radius) `margin` outside the surface. The boundary is
/// bisected so the bend varies continuously; without a clear bend, the least
/// penetrating one is used and its residual reported.
pub(super) fn conform_finger(rig: &FingerRig, hand: Mat4, faces: &[Face], margin: f64) -> FingerFit {
    let violation = |bend: f32| {
        let chain = rig.chain(1.0, finger_angles(rig, bend));
        let spheres = [chain[0], (chain[0] + chain[1]) * 0.5, chain[1], (chain[1] + chain[2]) * 0.5,
            chain[2], (chain[2] + chain[3]) * 0.5, chain[3]];
        spheres.iter()
            .filter_map(|p| signed_distance(hand.transform_point3(*p).as_dvec3(), faces))
            .map(|(d, _)| rig.radius as f64 + margin - d)
            .fold(f64::NEG_INFINITY, f64::max)
    };
    // Reported depths exclude the margin: how far finger skin is inside.
    let fit = |bend: f32, violation: f64, penetration: f64| FingerFit {
        angles: finger_angles(rig, bend), bend: bend as f64, penetration, residual: (violation - margin).max(0.0),
    };
    let initial = violation(0.0);
    let penetration = (initial - margin).max(0.0);
    if initial <= 0.0 {
        return fit(0.0, initial, penetration);
    }
    let reach = rig.limits.iter().copied().fold(0.0_f32, f32::max);
    let mut best = (0.0_f32, initial);
    let mut magnitude = FINGER_STEP_RAD;
    while magnitude <= reach + 1e-6 {
        for sign in [1.0_f32, -1.0] {
            let bend = sign * magnitude;
            if bend < -FINGER_EXTENSION_RAD {
                continue;
            }
            let v = violation(bend);
            if v < best.1 {
                best = (bend, v);
            }
            if v <= 0.0 {
                let bend = boundary(&violation, bend - sign * FINGER_STEP_RAD, bend);
                return fit(bend, violation(bend), penetration);
            }
        }
        magnitude += FINGER_STEP_RAD;
    }
    fit(best.0, best.1, penetration)
}

pub(super) fn nearby_faces(faces: &[Face], points: &[DVec3], radius: f64) -> Vec<Face> {
    let (lo, hi) = points.iter().fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let (lo, hi) = (lo - DVec3::splat(radius), hi + DVec3::splat(radius));
    faces.iter().filter(|f| {
        let (a, b) = f.points.iter().fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(a, b), p| (a.min(*p), b.max(*p)));
        a.cmple(hi).all() && b.cmpge(lo).all()
    }).copied().collect()
}

/// Closest point on a triangle (Ericson 5.1.5) and the feature it lies on:
/// `None` for the interior, `Some(edge)` for edge 0 (ab), 1 (bc), 2 (ca), or
/// `Some(3 + vertex)` for a vertex.
fn closest(p: DVec3, [a, b, c]: [DVec3; 3]) -> (DVec3, Option<usize>) {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 { return (a, Some(3)); }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 { return (b, Some(4)); }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 { return (a + ab * (d1 / (d1 - d3)), Some(0)); }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 { return (c, Some(5)); }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 { return (a + ac * (d2 / (d2 - d6)), Some(2)); }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return (b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6))), Some(1));
    }
    let denom = 1.0 / (va + vb + vc);
    (a + ab * (vb * denom) + ac * (vc * denom), None)
}

/// Signed distance to the surface (negative inside) and the pseudo-normal of
/// the nearest feature. `None` when the nearest feature is an open boundary:
/// such a point lies beside the selected surface, not inside it.
pub(super) fn signed_distance(p: DVec3, faces: &[Face]) -> Option<(f64, DVec3)> {
    let mut best = f64::INFINITY;
    let mut hits: Vec<(f64, &Face, Option<usize>)> = Vec::new();
    for face in faces {
        let (q, feature) = closest(p, face.points);
        let d = p.distance(q);
        if d < best - 1e-9 {
            best = d;
            hits.retain(|h| h.0 <= best + 1e-9);
        }
        if d <= best + 1e-9 {
            hits.push((d, face, feature));
        }
    }
    if hits.is_empty() {
        return None;
    }
    let open = |face: &Face, feature: Option<usize>| match feature {
        None => false,
        Some(e @ 0..=2) => face.open_edges[e],
        Some(v) => { let v = v - 3; face.open_edges[v] || face.open_edges[(v + 2) % 3] }
    };
    if hits.iter().any(|(_, f, feature)| open(f, *feature)) {
        return None;
    }
    let normal = hits.iter().map(|(_, f, _)| f.normal).sum::<DVec3>().normalize_or_zero();
    let (q, _) = closest(p, hits[0].1.points);
    let sign = if (p - q).dot(normal) < 0.0 { -1.0 } else { 1.0 };
    Some((sign * best, normal))
}

pub(super) struct PushOut {
    pub penetration: f64,
    pub delta: DVec3,
    pub converged: bool,
}

/// Points within this distance of the surface constrain the translation, so a
/// push out of one face cannot drive a neighbouring point into another.
const CONSTRAINT_BAND_M: f64 = 0.005;

/// Minimal-norm rigid translation leaving every point at least `margin`
/// outside the surface. Each outer pass linearizes signed distance at the
/// current translation (half-spaces `n·Δ ≥ b`) and projects zero onto their
/// intersection with Dykstra's algorithm, which converges to the nearest
/// point of the intersection and varies continuously with the constraints.
pub(super) fn push_out(points: &[DVec3], faces: &[Face], margin: f64) -> PushOut {
    let penetration = points.iter().filter_map(|p| signed_distance(*p, faces))
        .map(|(d, _)| -d).fold(0.0_f64, f64::max);
    let mut delta = DVec3::ZERO;
    for _ in 0..8 {
        let mut worst = 0.0_f64;
        let constraints: Vec<(DVec3, f64)> = points.iter()
            .filter_map(|p| signed_distance(*p + delta, faces))
            .filter(|(d, n)| *d < margin + CONSTRAINT_BAND_M && *n != DVec3::ZERO)
            .map(|(d, n)| {
                worst = worst.max(margin - d);
                (n, margin - d + n.dot(delta))
            })
            .collect();
        if worst <= 1e-5 {
            return PushOut { penetration, delta, converged: true };
        }
        let mut x = DVec3::ZERO;
        let mut increments = vec![DVec3::ZERO; constraints.len()];
        for _ in 0..500 {
            let before = x;
            for ((n, b), increment) in constraints.iter().zip(&mut increments) {
                let y = x + *increment;
                let gap = b - n.dot(y);
                x = if gap > 0.0 { y + *n * gap } else { y };
                *increment = y - x;
            }
            if x.distance(before) < 1e-9 {
                break;
            }
        }
        delta = x;
    }
    let converged = points.iter().filter_map(|p| signed_distance(*p + delta, faces))
        .all(|(d, _)| d >= margin - 1e-5);
    PushOut { penetration, delta, converged }
}

/// Two-bone IK: world rotations of the upper arm and forearm that move the
/// wrist to `target`, keeping the elbow on its current side of the
/// shoulder-target line. `None` when the target is out of reach.
pub(super) fn two_bone(
    shoulder: DVec3, elbow: DVec3, wrist: DVec3, target: DVec3, upper: DQuat, fore: DQuat,
) -> Option<(DQuat, DQuat)> {
    let (a, b) = (elbow.distance(shoulder), wrist.distance(elbow));
    let d = target.distance(shoulder);
    if d >= a + b - 1e-6 || d <= (a - b).abs() + 1e-6 {
        return None;
    }
    let u = (target - shoulder) / d;
    let pole = (elbow - shoulder) - u * (elbow - shoulder).dot(u);
    if pole.length() < 1e-9 {
        return None;
    }
    let p = pole.normalize();
    let cos = (a * a + d * d - b * b) / (2.0 * a * d);
    let new_elbow = shoulder + a * (cos * u + (1.0 - cos * cos).max(0.0).sqrt() * p);
    let r1 = DQuat::from_rotation_arc((elbow - shoulder).normalize(), (new_elbow - shoulder).normalize());
    let carried = shoulder + r1 * (wrist - shoulder);
    let r2 = DQuat::from_rotation_arc((carried - new_elbow).normalize(), (target - new_elbow).normalize());
    Some((r1 * upper, r2 * r1 * fore))
}

#[cfg(test)]
#[path = "hand_clamp_tests.rs"]
mod tests;
