//! Forearm twist distribution. The CC5 rig skins the forearm to two twist bones
//! (`ForearmTwist01` at the elbow, `ForearmTwist02` mid-forearm) that the pose retarget
//! never moves, so any roll of the hand relative to the forearm collapsed into the wrist
//! skin (a pinched "candy wrapper" wrist). Real pronation happens along the forearm: the
//! hand's roll about the forearm axis is shared out to the twist bones each frame.
use bevy::prelude::*;

/// Share of the hand's roll at the elbow-side twist bone, and cumulatively at mid-forearm.
pub const TWIST01_SHARE: f32 = 0.25;
pub const TWIST02_SHARE: f32 = 0.75;

/// Rest data of one forearm.
#[derive(Clone, Copy, Debug)]
pub struct ForearmRig {
    pub hand: Entity,
    pub twist01: Entity,
    pub twist02: Entity,
    /// Hand's rest rotation in the forearm frame, and the forearm axis there (towards the hand).
    pub hand_rest: Quat,
    pub axis: Vec3,
    pub twist01_rest: Quat,
    pub twist02_rest: Quat,
}

/// Signed roll (radians) of the hand about the forearm axis, from its rest pose.
pub fn hand_roll(hand_local: Quat, hand_rest: Quat, axis: Vec3) -> f32 {
    let delta = hand_local * hand_rest.inverse(); // the hand's rotation from rest, in the forearm frame
    let (_, twist) = super::pose::swing_twist_decompose(delta, axis);
    let angle = super::pose::signed_twist_angle(twist, axis);
    // Shortest way round.
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Local rotations of the two twist bones for a hand roll of `roll` radians.
pub fn twist_locals(rig: &ForearmRig, roll: f32) -> (Quat, Quat) {
    let t1 = Quat::from_axis_angle(rig.axis, TWIST01_SHARE * roll) * rig.twist01_rest;
    let axis_in_t1 = rig.twist01_rest.inverse() * rig.axis;
    let t2 = Quat::from_axis_angle(axis_in_t1, (TWIST02_SHARE - TWIST01_SHARE) * roll) * rig.twist02_rest;
    (t1, t2)
}

#[derive(Resource, Default)]
pub struct ForearmRigs(pub Vec<ForearmRig>);

pub(crate) fn capture_forearm_rigs(
    mut rigs: ResMut<ForearmRigs>,
    bind: Res<super::pose::CC5BindRotations>,
    bones: Query<(Entity, &Name, &Transform)>,
) {
    if !bind.captured || !rigs.0.is_empty() { return; }
    for side in ["L", "R"] {
        let find = |name: String| bones.iter().find(|(_, n, _)| n.as_str() == name).map(|(e, _, t)| (e, *t));
        let (Some((hand, hand_t)), Some((twist01, t1)), Some((twist02, t2))) = (
            find(format!("CC_Base_{side}_Hand")),
            find(format!("CC_Base_{side}_ForearmTwist01")),
            find(format!("CC_Base_{side}_ForearmTwist02")),
        ) else { continue };
        rigs.0.push(ForearmRig { hand, twist01, twist02, hand_rest: hand_t.rotation,
            axis: hand_t.translation.normalize(), twist01_rest: t1.rotation, twist02_rest: t2.rotation });
    }
}

/// After the pose is applied, before propagation: share each hand's roll along its forearm.
pub(crate) fn distribute_forearm_twist(rigs: Res<ForearmRigs>, mut transforms: Query<&mut Transform>) {
    for rig in &rigs.0 {
        let Ok(hand) = transforms.get(rig.hand).map(|t| t.rotation) else { continue };
        let (t1, t2) = twist_locals(rig, hand_roll(hand, rig.hand_rest, rig.axis));
        if let Ok(mut t) = transforms.get_mut(rig.twist01) { t.rotation = t1; }
        if let Ok(mut t) = transforms.get_mut(rig.twist02) { t.rotation = t2; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> ForearmRig {
        ForearmRig { hand: Entity::from_raw(1), twist01: Entity::from_raw(2), twist02: Entity::from_raw(3),
            hand_rest: Quat::from_rotation_z(0.3), axis: Vec3::new(-0.866, -0.5, 0.).normalize(),
            twist01_rest: Quat::IDENTITY, twist02_rest: Quat::from_rotation_y(0.1) }
    }

    #[test]
    fn the_hands_roll_is_shared_along_the_forearm() {
        let r = rig();
        let roll = 1.6; // a 92-degree pronation
        let hand = Quat::from_axis_angle(r.axis, roll) * r.hand_rest;
        assert!((hand_roll(hand, r.hand_rest, r.axis) - roll).abs() < 1e-4);
        // A bend (no roll) contributes nothing.
        let bent = Quat::from_axis_angle(r.axis.cross(Vec3::Z).normalize(), 0.7) * r.hand_rest;
        assert!(hand_roll(bent, r.hand_rest, r.axis).abs() < 1e-4);
        // The twist bones turn a quarter and three quarters of the way, about the forearm axis.
        let (t1, t2) = twist_locals(&r, roll);
        let at_t1 = t1 * r.twist01_rest.inverse();
        let at_t2 = t1 * t2 * (r.twist01_rest * r.twist02_rest).inverse();
        let angle = |q: Quat| super::super::pose::signed_twist_angle(q, r.axis);
        assert!((angle(at_t1) - 0.25 * roll).abs() < 1e-4);
        assert!((angle(at_t2) - 0.75 * roll).abs() < 1e-4);
        // At rest nothing moves.
        assert_eq!(twist_locals(&r, 0.), (r.twist01_rest, r.twist02_rest));
    }
}
