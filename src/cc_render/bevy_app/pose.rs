//! Pose plumbing — bridges the renderer's input channel into Bevy's
//! ECS so a Bevy system can read the latest ARKit-52 pose each tick.
//!
//! Uses [`tokio::sync::watch`] (lock-free, single-slot, latest-wins)
//! since we only ever care about the freshest pose; older blendshape
//! frames the renderer didn't tick on can be safely dropped.

use bevy::prelude::*;
use bevy::render::mesh::morph::MeshMorphWeights;
use std::collections::HashMap;
use tokio::sync::watch;

use super::super::renderer::{ArkitPose, SkeletalPose};
use super::assets::ArkitMapping;
use super::camera::AvatarRoot;
use super::capture::LastAppliedPts;

/// Wraps the watch receiver so a Bevy system can pull the latest pose.
#[derive(Resource)]
pub(crate) struct PoseWatchRx(pub watch::Receiver<ArkitPose>);

/// Skeletal-pose receiver. FIFO bounded queue (replaces the prior
/// `watch::Receiver`), so paced motion frames are delivered to
/// [`apply_skeletal_pose`] **in order** even if Bevy is briefly stalled
/// (e.g. the bind-capture settle frames between scene-spawn and
/// `bind.captured = true`). Each enqueued item is `Some(pose)` to
/// apply or `None` to release the override + resume baked animation.
#[derive(Resource)]
pub(crate) struct SkeletalPoseWatchRx(pub crossbeam_channel::Receiver<Option<SkeletalPose>>);

/// Pure FK retargeter, factored out of [`apply_skeletal_pose`] so it
/// can be unit-tested without spinning up Bevy + a real avatar GLB.
///
/// Given:
///   - 22 SMPL parent-local quats from the streamed pose,
///   - the precomputed `cc5_rest_world` and `inter_rest` from
///     [`CC5BindRotations`],
///   - the pelvis's `parent_chain_rest`,
///
/// returns the 22 CC5-local quats to write onto the bones — exactly
/// matching the per-frame loop in `scripts/avatars/smpl_to_cc5_retarget.py`.
pub(crate) fn fk_retarget_smpl_to_cc5(
    smpl_local: &[[f32; 4]; 22],
    cc5_rest_world: &[Quat; 22],
    inter_rest: &[Quat; 22],
    pelvis_parent_chain_rest: Quat,
) -> [Quat; 22] {
    fk_retarget_smpl_to_cc5_with_align(
        smpl_local,
        cc5_rest_world,
        inter_rest,
        pelvis_parent_chain_rest,
        Quat::IDENTITY,
    )
}

/// Same as `fk_retarget_smpl_to_cc5` but accepts an explicit alignment
/// quaternion `r_align` that's applied at the SMPL pelvis in WORLD space
/// (e.g. 180° Y to flip facing for rigs where SMPL +Z forward conflicts
/// with CC5 -Z forward).
pub(crate) fn fk_retarget_smpl_to_cc5_with_align(
    smpl_local: &[[f32; 4]; 22],
    cc5_rest_world: &[Quat; 22],
    inter_rest: &[Quat; 22],
    pelvis_parent_chain_rest: Quat,
    r_align: Quat,
) -> [Quat; 22] {
    let mut w_smpl: [Quat; 22] = [Quat::IDENTITY; 22];
    let mut w_cc5: [Quat; 22] = [Quat::IDENTITY; 22];
    let mut out: [Quat; 22] = [Quat::IDENTITY; 22];

    for &i in SMPL_TOPO.iter() {
        let parent = SMPL_PARENTS[i];

        let raw = Quat::from_xyzw(
            smpl_local[i][0],
            smpl_local[i][1],
            smpl_local[i][2],
            smpl_local[i][3],
        );
        let dq_smpl = if raw.length_squared() > 1e-8 {
            raw.normalize()
        } else {
            Quat::IDENTITY
        };

        // SMPL world rotation: chain-accumulate parent-local rotations.
        // Pelvis: optionally pre-rotate by r_align (world-space facing fix).
        w_smpl[i] = if parent < 0 {
            r_align * dq_smpl
        } else {
            w_smpl[parent as usize] * dq_smpl
        };

        // World-FK: apply SMPL world delta on top of CC5 bind world.
        // Since SMPL bind = identity locals, w_smpl IS the world delta.
        //
        // Pre-multiply (`w_smpl * cc5_rest_world`) preserves chain
        // propagation: a parent's animated world feeds children correctly.
        // Tried post-multiply for "axis conformation" but it breaks chain
        // propagation — pelvis rotation no longer carries through the
        // spine into the arms because each bone's `w_cc5` is independently
        // re-axed instead of inheriting the parent's animated world.
        //
        // The remaining shoulder-axis-mismatch artifact (CC5 A-pose vs
        // SMPL T-pose bone axes) requires PER-BONE correction matrices,
        // not a global formula change.
        w_cc5[i] = w_smpl[i] * cc5_rest_world[i];

        // Convert to CC5-local using ANIMATED parent world. Pelvis uses
        // the un-animated parent chain (its CC5 parent doesn't move).
        // Other bones compose the animated parent's CC5 world with
        // `inter_rest[i]` (rest contribution of any non-mapped CC5
        // bones between this bone and its SMPL-mapped ancestor).
        let parent_world = if parent < 0 {
            pelvis_parent_chain_rest
        } else {
            w_cc5[parent as usize] * inter_rest[i]
        };
        out[i] = parent_world.inverse() * w_cc5[i];
    }

    out
}

/// Decompose a unit quaternion into the swing component (rotation
/// orthogonal to `axis`) and twist component (rotation around `axis`).
/// Standard Day-2007 formulation: project the quaternion's vector
/// part onto the twist axis to isolate the twist; the remainder is
/// the swing.
///
/// Used by [`fk_retarget_smpl_to_cc5_axis_match_chained`] to extract
/// the bone's twist around its own axis from the SMPL local rotation,
/// so the chained direction-only solve doesn't drop it. Without this,
/// pure rotations around a bone's own axis (e.g. Y-twist on the
/// spine column during a full-body spin) become invisible to
/// `from_rotation_arc(bone_axis, bone_axis) = identity`.
pub(crate) fn swing_twist_decompose(q: Quat, axis: Vec3) -> (Quat, Quat) {
    let axis_n = axis.normalize_or_zero();
    if axis_n.length_squared() < 1e-12 {
        return (q, Quat::IDENTITY);
    }
    // Project quaternion's vector part onto twist axis.
    let proj = q.x * axis_n.x + q.y * axis_n.y + q.z * axis_n.z;
    let tx = axis_n.x * proj;
    let ty = axis_n.y * proj;
    let tz = axis_n.z * proj;
    let tw = q.w;
    let len_sq = tx * tx + ty * ty + tz * tz + tw * tw;
    let twist = if len_sq > 1e-12 {
        let inv_len = len_sq.sqrt().recip();
        Quat::from_xyzw(tx * inv_len, ty * inv_len, tz * inv_len, tw * inv_len)
    } else {
        // q.w ~ 0 and q.xyz orthogonal to axis: swing-only rotation.
        Quat::IDENTITY
    };
    let swing = q * twist.conjugate();
    (swing, twist)
}

/// Signed rotation angle (radians) of a unit quaternion around `axis`.
/// For a quaternion already known to be a twist around `axis` (i.e.
/// the second return value from [`swing_twist_decompose`]):
///   q = cos(θ/2) + sin(θ/2)·unit_axis
/// so signed sin(θ/2) = q.xyz · axis (preserves sign for negative
/// rotations) and θ = 2·atan2(sin, cos).
pub(crate) fn signed_twist_angle(twist: Quat, axis: Vec3) -> f32 {
    let axis_n = axis.normalize_or_zero();
    let s = twist.x * axis_n.x + twist.y * axis_n.y + twist.z * axis_n.z;
    let c = twist.w;
    2.0 * s.atan2(c)
}

/// Chain-aware axis-match retarget WITH twist preservation.
///
/// Same per-bone direction-correcting `R_align` as the inline
/// `axis_match_chained` mode (which prevents bind-tilt errors from
/// accumulating through long chains the way static `R_corr` does in
/// `axis_match`), PLUS the twist component of the bone's local SMPL
/// rotation around `SMPL_BONE_AXES[i]`. Without the twist step, any
/// rotation aligned with a bone's own axis (most importantly: a
/// Y-axis spin propagating down the +Y-axis spine column) is dropped
/// because `from_rotation_arc(axis, axis) = identity`.
///
/// **Math** (per bone in `SMPL_TOPO` order):
///
/// ```text
/// w_smpl[i]            = w_smpl[parent] · dq_smpl[i]
/// parent_anim_delta    = w_cc5[parent] · cc5_rest_world[parent]⁻¹
/// cc5_dir_now          = parent_anim_delta · cc5_bone_axis_world[i]
/// smpl_dir_now         = w_smpl[i] · SMPL_BONE_AXES[i]
/// R_align              = from_rotation_arc(cc5_dir_now, smpl_dir_now)   // swing
/// (_, twist_smpl)      = swing_twist_decompose(dq_smpl[i], SMPL_BONE_AXES[i])
/// twist_world          = from_axis_angle(smpl_dir_now, signed_angle(twist_smpl))
/// w_cc5[i]             = twist_world · R_align · (parent_anim_delta · cc5_rest_world[i])
/// out[i]               = parent_world_for_local⁻¹ · w_cc5[i]
/// ```
///
/// `freeze_leaves` keeps the wrist/foot/head bones at their bind
/// orientation relative to parent (their bone-axis basis on most CC5
/// rigs doesn't match SMPL's twist convention, which produces wrong
/// distal twists). The chain still propagates through them via
/// parent inheritance, so e.g. Head still spins with Neck during a
/// body rotation even when frozen — its OWN twist is just not
/// applied on top.
pub(crate) fn fk_retarget_smpl_to_cc5_axis_match_chained(
    joint_quats: &[[f32; 4]; 22],
    cc5_rest_world: &[Quat; 22],
    cc5_bone_axis_world: &[Vec3; 22],
    inter_rest: &[Quat; 22],
    own_rest_local: &[Quat; 22],
    pelvis_parent_chain_rest: Quat,
    align_y_quat: Quat,
    freeze_leaves: bool,
) -> [Quat; 22] {
    let mut w_smpl: [Quat; 22] = [Quat::IDENTITY; 22];
    let mut w_cc5: [Quat; 22] = [Quat::IDENTITY; 22];
    let mut out: [Quat; 22] = [Quat::IDENTITY; 22];

    for &i in SMPL_TOPO.iter() {
        let parent = SMPL_PARENTS[i];
        let raw = Quat::from_xyzw(
            joint_quats[i][0],
            joint_quats[i][1],
            joint_quats[i][2],
            joint_quats[i][3],
        );
        let dq_smpl = if raw.length_squared() > 1e-8 {
            raw.normalize()
        } else {
            Quat::IDENTITY
        };
        // `align_y_quat` only prepends at the pelvis to compensate for
        // SMPL ↔ CC5 axis-convention differences (see CC_AVATAR_SMPL_ALIGN_Y_DEG).
        let dq_smpl = if i == 0 {
            align_y_quat * dq_smpl
        } else {
            dq_smpl
        };

        w_smpl[i] = if parent < 0 {
            dq_smpl
        } else {
            w_smpl[parent as usize] * dq_smpl
        };

        let is_leaf = SMPL_PRIMARY_CHILD[i].is_none();
        if freeze_leaves && is_leaf && parent >= 0 {
            let parent_anim_world = w_cc5[parent as usize] * inter_rest[i];
            w_cc5[i] = parent_anim_world * own_rest_local[i];
        } else {
            let parent_anim_delta = if parent < 0 {
                Quat::IDENTITY
            } else {
                w_cc5[parent as usize] * cc5_rest_world[parent as usize].inverse()
            };

            let smpl_axis = Vec3::from_array(SMPL_BONE_AXES[i]);
            let cc5_dir_now = (parent_anim_delta * cc5_bone_axis_world[i]).normalize_or_zero();
            let smpl_dir_now = (w_smpl[i] * smpl_axis).normalize_or_zero();

            let r_align =
                if cc5_dir_now.length_squared() > 1e-8 && smpl_dir_now.length_squared() > 1e-8 {
                    Quat::from_rotation_arc(cc5_dir_now, smpl_dir_now)
                } else {
                    Quat::IDENTITY
                };

            // Twist preservation: dq_smpl carries the bone's local
            // rotation; its component around SMPL_BONE_AXES[i] is the
            // twist that the swing-only direction match drops. Re-apply
            // it around the FINAL bone direction in world (= smpl_dir_now,
            // by definition of R_align).
            let (_, twist_smpl) = swing_twist_decompose(dq_smpl, smpl_axis);
            let twist_angle = signed_twist_angle(twist_smpl, smpl_axis);
            let twist_world = if smpl_dir_now.length_squared() > 1e-8 && twist_angle.abs() > 1e-6 {
                Quat::from_axis_angle(smpl_dir_now, twist_angle)
            } else {
                Quat::IDENTITY
            };

            let cc5_bind_world_carried = parent_anim_delta * cc5_rest_world[i];
            w_cc5[i] = twist_world * r_align * cc5_bind_world_carried;
        }

        let parent_world_for_local = if parent < 0 {
            pelvis_parent_chain_rest
        } else {
            w_cc5[parent as usize] * inter_rest[i]
        };
        out[i] = parent_world_for_local.inverse() * w_cc5[i];
    }
    out
}

#[cfg(test)]
mod fk_tests {
    use super::*;

    fn ident_local() -> [[f32; 4]; 22] {
        [[0.0, 0.0, 0.0, 1.0]; 22]
    }

    /// Identity SMPL pose against identity rest data must produce
    /// identity local rotations everywhere.
    #[test]
    fn identity_pose_against_identity_rest_yields_identity() {
        let cc5_rest_world = [Quat::IDENTITY; 22];
        let inter_rest = [Quat::IDENTITY; 22];
        let out =
            fk_retarget_smpl_to_cc5(&ident_local(), &cc5_rest_world, &inter_rest, Quat::IDENTITY);
        for q in out.iter() {
            assert!(q.abs_diff_eq(Quat::IDENTITY, 1e-5));
        }
    }

    /// Identity SMPL against a non-identity CC5 rest pose must
    /// reproduce CC5's rest local rotations — i.e. the avatar should
    /// stay in its bind pose when the streamed motion is "no motion".
    /// This exercises the precompute relations:
    ///   cc5_rest_world[i]      = parent_chain_rest[i] * own_rest_local[i]
    ///   inter_rest[i]          = inv(cc5_rest_world[parent_smpl[i]])
    ///                            * parent_chain_rest[i]
    /// so that with src=identity:
    ///   parent_world * cc5_local[i] == cc5_rest_world[i]
    /// where parent_world = cc5_rest_world[parent_smpl[i]] * inter_rest[i]
    ///                    = parent_chain_rest[i]
    /// hence cc5_local[i] = inv(parent_chain_rest[i]) * cc5_rest_world[i]
    ///                    = own_rest_local[i].
    #[test]
    fn identity_pose_against_real_rest_returns_bind_locals() {
        // Synthesize plausible CC5 bind data: every bone at a different
        // small rotation. Build parent_chain_rest topologically so the
        // invariants hold.
        let own_rest_local: [Quat; 22] = std::array::from_fn(|i| {
            // Distinct, small rotations around mixed axes.
            let angle = 0.07 * (i as f32 + 1.0);
            let axis = Vec3::new(
                ((i % 3) as f32 - 1.0).max(0.1),
                (((i + 1) % 3) as f32 - 1.0).max(0.1),
                (((i + 2) % 3) as f32 - 1.0).max(0.1),
            )
            .normalize();
            Quat::from_axis_angle(axis, angle)
        });

        let mut parent_chain_rest: [Quat; 22] = [Quat::IDENTITY; 22];
        let mut cc5_rest_world: [Quat; 22] = [Quat::IDENTITY; 22];
        // Walk topo so parents are computed first. Pelvis: assume some
        // arbitrary scene-root rotation rather than identity, to make
        // the test stronger.
        let scene_root_rest = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_4);
        for &i in SMPL_TOPO.iter() {
            let parent = SMPL_PARENTS[i];
            // For the SMPL pelvis (parent = -1) we still pretend there's
            // a non-identity ancestor chain (e.g. CC_Base_BoneRoot →
            // CC_Base_Hip): inject `scene_root_rest`.
            parent_chain_rest[i] = if parent < 0 {
                scene_root_rest
            } else {
                cc5_rest_world[parent as usize]
            };
            cc5_rest_world[i] = parent_chain_rest[i] * own_rest_local[i];
        }
        // inter_rest: with NO non-mapped intermediates, this collapses
        // to identity for every joint whose CC5 parent IS its SMPL
        // parent — the topo walk above ensured exactly that. So
        // inter_rest = identity here. (The non-trivial inter_rest case
        // is exercised by the `intermediate_bone_rest` test below.)
        let inter_rest = [Quat::IDENTITY; 22];

        let out = fk_retarget_smpl_to_cc5(
            &ident_local(),
            &cc5_rest_world,
            &inter_rest,
            scene_root_rest,
        );
        for i in 0..22 {
            assert!(
                out[i].abs_diff_eq(own_rest_local[i], 1e-4),
                "joint {i}: got {:?}, want {:?}",
                out[i],
                own_rest_local[i],
            );
        }
    }

    /// Inter-bone rest correction: when the CC5 chain has a non-mapped
    /// intermediate bone between a SMPL joint and its SMPL parent
    /// (e.g. CC_Base_Pelvis between CC_Base_Hip and CC_Base_L_Thigh),
    /// the precompute folds that intermediate's rest rotation into
    /// `inter_rest`. Identity SMPL pose must STILL reproduce the
    /// downstream bone's bind local rotation regardless of the
    /// intermediate's rest contribution.
    #[test]
    fn intermediate_bone_rest_collapses_to_bind_pose() {
        // Pelvis (SMPL 0) → L_Hip (SMPL 1). Insert a virtual non-mapped
        // intermediate with non-identity rest rotation between them.
        let pelvis_own = Quat::from_axis_angle(Vec3::Y, 0.3);
        let lhip_own = Quat::from_axis_angle(Vec3::X, -0.2);
        let inter_world = Quat::from_axis_angle(Vec3::Z, 0.5); // CC_Base_Pelvis-style

        let mut own_rest_local = [Quat::IDENTITY; 22];
        own_rest_local[0] = pelvis_own;
        own_rest_local[1] = lhip_own;

        let pelvis_parent_chain = Quat::IDENTITY;
        let mut parent_chain_rest = [Quat::IDENTITY; 22];
        parent_chain_rest[0] = pelvis_parent_chain;
        // L_Hip's parent chain is pelvis_world * inter_world
        parent_chain_rest[1] = pelvis_parent_chain * pelvis_own * inter_world;

        let mut cc5_rest_world = [Quat::IDENTITY; 22];
        cc5_rest_world[0] = parent_chain_rest[0] * pelvis_own;
        cc5_rest_world[1] = parent_chain_rest[1] * lhip_own;

        // inter_rest[1] = inv(cc5_rest_world[0]) * parent_chain_rest[1]
        //              = inv(pelvis_world) * (pelvis_world * inter_world)
        //              = inter_world
        let mut inter_rest = [Quat::IDENTITY; 22];
        inter_rest[1] = cc5_rest_world[0].inverse() * parent_chain_rest[1];
        assert!(inter_rest[1].abs_diff_eq(inter_world, 1e-5));

        let out = fk_retarget_smpl_to_cc5(
            &ident_local(),
            &cc5_rest_world,
            &inter_rest,
            pelvis_parent_chain,
        );
        // Even with the intermediate, identity SMPL → bind locals.
        assert!(out[0].abs_diff_eq(pelvis_own, 1e-4));
        assert!(out[1].abs_diff_eq(lhip_own, 1e-4));
    }

    /// Pure-Y rotation at the pelvis (the dominant motion of a "spin")
    /// must propagate to `cc5_local[Pelvis]`. Regression test for the
    /// "spin doesn't reach the spine column" bug: the pre-fix swing-only
    /// direction match returned identity for this case because the pelvis
    /// bone axis (+Y) is invariant under Y-rotation, so
    /// `from_rotation_arc(+Y, +Y) = identity` and the entire spin was
    /// silently dropped. Twist preservation must keep the rotation.
    #[test]
    fn axis_match_chained_preserves_y_twist_at_pelvis() {
        let theta = 45.0_f32.to_radians();
        let q_spin = Quat::from_rotation_y(theta);
        let mut joint_quats = [[0.0_f32, 0.0, 0.0, 1.0_f32]; 22];
        joint_quats[0] = [q_spin.x, q_spin.y, q_spin.z, q_spin.w];

        // Identity rest data — equivalent to a clean T-pose rig.
        let cc5_rest_world = [Quat::IDENTITY; 22];
        let cc5_bone_axis_world: [Vec3; 22] =
            std::array::from_fn(|i| Vec3::from_array(SMPL_BONE_AXES[i]));
        let inter_rest = [Quat::IDENTITY; 22];
        let own_rest_local = [Quat::IDENTITY; 22];

        let out = fk_retarget_smpl_to_cc5_axis_match_chained(
            &joint_quats,
            &cc5_rest_world,
            &cc5_bone_axis_world,
            &inter_rest,
            &own_rest_local,
            Quat::IDENTITY, // pelvis_parent_chain_rest
            Quat::IDENTITY, // align_y_quat
            false,          // freeze_leaves (irrelevant for non-leaf pelvis)
        );

        assert!(
            out[0].abs_diff_eq(q_spin, 1e-4),
            "pelvis Y-twist should produce cc5_local[0]={:?}, got {:?}",
            q_spin,
            out[0],
        );
    }
}

/// SMPL-22 → CC5 bone-name table. Order MATCHES `SkeletalPose::joint_quats`
/// indexing (also matches `SMPL_22_NAMES` in `kimodo_gen.py` and the
/// `mapping` block of `scripts/avatars/smpl22_to_cc5.bone_map.json`).
///
/// Compile-time constant so we don't parse JSON every frame; if you
/// change the bone-map JSON, update this table.
pub(crate) const SMPL22_TO_CC5: [(&str, &str); 22] = [
    ("Pelvis", "CC_Base_Hip"),
    ("L_Hip", "CC_Base_L_Thigh"),
    ("R_Hip", "CC_Base_R_Thigh"),
    ("Spine1", "CC_Base_Waist"),
    ("L_Knee", "CC_Base_L_Calf"),
    ("R_Knee", "CC_Base_R_Calf"),
    ("Spine2", "CC_Base_Spine01"),
    ("L_Ankle", "CC_Base_L_Foot"),
    ("R_Ankle", "CC_Base_R_Foot"),
    ("Spine3", "CC_Base_Spine02"),
    ("L_Foot", "CC_Base_L_ToeBase"),
    ("R_Foot", "CC_Base_R_ToeBase"),
    ("Neck", "CC_Base_NeckTwist01"),
    ("L_Collar", "CC_Base_L_Clavicle"),
    ("R_Collar", "CC_Base_R_Clavicle"),
    ("Head", "CC_Base_Head"),
    ("L_Shoulder", "CC_Base_L_Upperarm"),
    ("R_Shoulder", "CC_Base_R_Upperarm"),
    ("L_Elbow", "CC_Base_L_Forearm"),
    ("R_Elbow", "CC_Base_R_Forearm"),
    ("L_Wrist", "CC_Base_L_Hand"),
    ("R_Wrist", "CC_Base_R_Hand"),
];

/// SMPL-22 kinematic chain parents (matches `kinematic_chain_parents` in
/// `smpl22_to_cc5.bone_map.json`). `-1` denotes the root (pelvis).
pub(crate) const SMPL_PARENTS: [i8; 22] = [
    -1, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 9, 9, 12, 13, 14, 16, 17, 18, 19,
];

/// Topological order over `SMPL_PARENTS` — every parent precedes its
/// children. Validated against the table above; same as `range(22)`
/// since SMPL is conveniently ordered that way in HumanML3D/Kimodo.
pub(crate) const SMPL_TOPO: [usize; 22] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
];

/// Per-bone primary child for axis derivation (Option-2 axis-match).
/// `None` ⇒ leaf bone (no SMPL child); axis falls back to the bone's
/// SMPL bind direction rotated by the bone's animated world rotation.
pub(crate) const SMPL_PRIMARY_CHILD: [Option<usize>; 22] = [
    Some(3),  // 0  Pelvis    -> Spine1
    Some(4),  // 1  L_Hip     -> L_Knee
    Some(5),  // 2  R_Hip     -> R_Knee
    Some(6),  // 3  Spine1    -> Spine2
    Some(7),  // 4  L_Knee    -> L_Ankle
    Some(8),  // 5  R_Knee    -> R_Ankle
    Some(9),  // 6  Spine2    -> Spine3
    Some(10), // 7  L_Ankle   -> L_Foot
    Some(11), // 8  R_Ankle   -> R_Foot
    Some(12), // 9  Spine3    -> Neck
    None,     // 10 L_Foot    leaf
    None,     // 11 R_Foot    leaf
    Some(15), // 12 Neck      -> Head
    Some(16), // 13 L_Collar  -> L_Shoulder
    Some(17), // 14 R_Collar  -> R_Shoulder
    None,     // 15 Head      leaf
    Some(18), // 16 L_Shoulder-> L_Elbow
    Some(19), // 17 R_Shoulder-> R_Elbow
    Some(20), // 18 L_Elbow   -> L_Wrist
    Some(21), // 19 R_Elbow   -> R_Wrist
    None,     // 20 L_Wrist   leaf
    None,     // 21 R_Wrist   leaf
];

/// SMPL T-pose primary axes — the world-space direction from each
/// bone toward its primary child, AT SMPL T-POSE. SMPL T-pose has
/// arms straight out along ±X, legs/spine along ±Y, feet pointing
/// +Z (forward). For leaves: same axis as the parent uses to reach
/// the leaf (so e.g. Head leaf inherits Neck's +Y axis).
pub(crate) const SMPL_BONE_AXES: [[f32; 3]; 22] = [
    [0.0, 1.0, 0.0],  // 0  Pelvis    -> Spine1: +Y
    [0.0, -1.0, 0.0], // 1  L_Hip     -> L_Knee: -Y
    [0.0, -1.0, 0.0], // 2  R_Hip     -> R_Knee: -Y
    [0.0, 1.0, 0.0],  // 3  Spine1    -> Spine2: +Y
    [0.0, -1.0, 0.0], // 4  L_Knee    -> L_Ankle: -Y
    [0.0, -1.0, 0.0], // 5  R_Knee    -> R_Ankle: -Y
    [0.0, 1.0, 0.0],  // 6  Spine2    -> Spine3: +Y
    [0.0, 0.0, 1.0],  // 7  L_Ankle   -> L_Foot: +Z (forward)
    [0.0, 0.0, 1.0],  // 8  R_Ankle   -> R_Foot: +Z
    [0.0, 1.0, 0.0],  // 9  Spine3    -> Neck: +Y
    [0.0, 0.0, 1.0],  // 10 L_Foot    leaf, +Z
    [0.0, 0.0, 1.0],  // 11 R_Foot    leaf, +Z
    [0.0, 1.0, 0.0],  // 12 Neck      -> Head: +Y
    [1.0, 0.0, 0.0],  // 13 L_Collar  -> L_Shoulder: +X
    [-1.0, 0.0, 0.0], // 14 R_Collar  -> R_Shoulder: -X
    [0.0, 1.0, 0.0],  // 15 Head      leaf, +Y
    [1.0, 0.0, 0.0],  // 16 L_Shoulder-> L_Elbow: +X
    [-1.0, 0.0, 0.0], // 17 R_Shoulder-> R_Elbow: -X
    [1.0, 0.0, 0.0],  // 18 L_Elbow   -> L_Wrist: +X
    [-1.0, 0.0, 0.0], // 19 R_Elbow   -> R_Wrist: -X
    [1.0, 0.0, 0.0],  // 20 L_Wrist   leaf, +X
    [-1.0, 0.0, 0.0], // 21 R_Wrist   leaf, -X
];

/// All the rest-pose data we need to retarget SMPL → CC5 at full FK
/// quality, captured ONCE the first time the avatar GLB has all 22
/// CC5 bones in the spawned scene.
///
/// Mirrors `scripts/avatars/smpl_to_cc5_retarget.py`'s precompute step:
///
/// ```text
/// own_rest_local[i]      = CC5 bone's own local rest rotation (bind)
/// parent_chain_rest[i]   = world rotation of CC5 bone's parent chain
///                          (root → leaf, EXCLUDING this bone — matches
///                          mrt.world_rest_rotation in mixamo_retarget.py)
/// cc5_rest_world[i]      = parent_chain_rest[i] * own_rest_local[i]
/// inter_rest[i]          = inv(cc5_rest_world[parent_smpl[i]])
///                          * parent_chain_rest[i]
///                          (rest-rotation of CC5 bones BETWEEN this
///                          bone and its SMPL-mapped ancestor — e.g.
///                          CC_Base_Pelvis sits between CC_Base_Hip and
///                          CC_Base_L_Thigh, contributing rest rotation
///                          that the SMPL chain doesn't see). Identity
///                          if the immediate parent is mapped, or for
///                          the pelvis itself.
/// pelvis_rest_translation = CC_Base_Hip's bind translation
/// pelvis_parent_chain_rest = parent_chain_rest[0] (kept separate for
///                          clarity since it's used for the root_pos
///                          delta math).
/// ```
#[derive(Resource, Default)]
pub(crate) struct CC5BindRotations {
    pub captured: bool,
    /// Each mapped bone's own bind-pose local rotation.
    pub own_rest_local: [Quat; 22],
    /// Parent-chain world rotation (root → leaf, EXCLUDING the bone).
    pub parent_chain_rest: [Quat; 22],
    /// Bone's own world rest rotation = parent_chain_rest * own_rest_local.
    pub cc5_rest_world: [Quat; 22],
    /// Per-bone inter-bone rest correction (see struct docs). Identity
    /// for the pelvis (no SMPL parent).
    pub inter_rest: [Quat; 22],
    /// Cached entity handles for the 22 mapped CC5 bones. `None`
    /// before capture.
    pub entities: [Option<Entity>; 22],
    /// Pelvis bind-pose translation (CC_Base_Hip).
    pub pelvis_rest_translation: Vec3,
    /// World-space pelvis translation captured at bind time. Used by
    /// `auto_orient_avatar` to determine where to look from.
    pub pelvis_world: Option<Vec3>,
    /// World-space unit vector pointing from L_Eye to R_Eye (= avatar's
    /// right-hand axis in world). `None` if either eye bone wasn't
    /// found. Used by `auto_orient_avatar` to derive the bind-pose
    /// facing direction.
    pub eye_axis_world: Option<Vec3>,
    /// World-space bone direction at bind, per retargeted bone. For
    /// bones with a primary child (`SMPL_PRIMARY_CHILD[i].is_some()`):
    /// normalized vector from this bone's joint position to its child's
    /// joint position. For leaves: the bone's bind world rotation
    /// applied to `SMPL_BONE_AXES[i]` as a fallback. Used by the
    /// `axis_match` retarget mode to align CC5 bones to SMPL's posed
    /// world directions.
    pub cc5_bone_axis_world: [Vec3; 22],
    /// World-space bone tip position at bind (= primary child joint
    /// world position). For leaves: bone joint + `cc5_bone_axis_world`
    /// scaled by a small length. Captured for downstream debugging.
    #[allow(dead_code)]
    pub cc5_bone_tip_world: [Vec3; 22],
}

/// Bevy resource that mirrors the latest applied pose's `pts_ms`.
/// Updated by `apply_arkit_pose`; extracted into the RenderApp by the
/// `ExtractResourcePlugin<LastAppliedPts>` so the capture system can
/// stamp each frame.
#[derive(Resource, Clone, Default)]
pub(crate) struct LatestPose {
    pub weights: HashMap<String, f32>,
    pub pts_ms: u64,
}

/// Per-system diagnostic state for `apply_arkit_pose`. Bevy systems
/// have a 16-parameter limit; bundling our `Local`s into one struct
/// keeps us under the cap.
#[derive(Default)]
pub(crate) struct ApplyArkitDiag {
    pub tick: u64,
    pub dumped_morph_names: bool,
}

/// Apply the most-recent pose's morph weights to every mesh that has
/// a `MorphWeights` component, looking up each ARKit channel through
/// the resolved CC5 mapping.
///
/// **Entity layout contract (CC5 GLB on Bevy 0.15).** The glTF importer
/// puts `Name` + `MorphWeights` on the *node* entity (e.g. `CC_Base_Body`)
/// and `Mesh3d` on a *child primitive* entity. Empirically: 7 entities
/// have `MorphWeights`, 27 entities have `Mesh3d`, and the two sets
/// are *disjoint*. So we query node-level (Name + MorphWeights) and
/// walk Children → Mesh3d to look up `morph_target_names()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_arkit_pose(
    mut commands: Commands,
    mut pose_rx: ResMut<PoseWatchRx>,
    mapping: Res<ArkitMapping>,
    meshes: Res<Assets<Mesh>>,
    mut last_pts: ResMut<LastAppliedPts>,
    mut q: Query<(Entity, Option<&Name>, &mut MorphWeights, Option<&Children>)>,
    parent_q: Query<&Parent>,
    name_q: Query<&Name>,
    mesh_q: Query<&Mesh3d>,
    // Diagnostic-only queries bundled in a ParamSet to keep
    // `apply_arkit_pose` under Bevy's 16-system-param limit. Each is
    // accessed exclusively (one at a time) inside the tick == 1 / 300
    // logging branch below.
    //
    // - p0: entities with MorphWeights (count)
    // - p1: entities with Mesh3d but no MorphWeights (count)
    // - p2: SceneRoots (load-state report)
    // - p3: proto layout (Name + Mesh3d + MorphWeights co-located).
    //   If this is 0, the GLB's MorphWeights live on parent nodes
    //   while Mesh3d lives on child primitives — and the per-frame
    //   Commands::insert(MorphWeights) workaround is the only path.
    mut diag_qs: ParamSet<(
        Query<Entity, With<MorphWeights>>,
        Query<Entity, (With<Mesh3d>, Without<MorphWeights>)>,
        Query<&SceneRoot>,
        Query<Entity, (With<Name>, With<Mesh3d>, With<MorphWeights>)>,
    )>,
    asset_server: Res<AssetServer>,
    scene_assets: Res<Assets<Scene>>,
    mut diag: Local<ApplyArkitDiag>,
    avatar_q: Query<Entity, With<AvatarRoot>>,
) {
    diag.tick += 1;
    let tick_val = diag.tick;
    if tick_val == 1 || tick_val % 300 == 0 {
        let n_morph = q.iter().count();
        let n_morph_only = diag_qs.p0().iter().count();
        let n_mesh_no_morph = diag_qs.p1().iter().count();
        let n_mesh = mesh_q.iter().count();
        let n_named = name_q.iter().count();
        let mut scene_states: Vec<String> = Vec::new();
        for sr in diag_qs.p2().iter() {
            let id = sr.0.id();
            let loaded_in_assets = scene_assets.get(&sr.0).is_some();
            let path = asset_server
                .get_path(id.untyped())
                .map(|p| p.to_string())
                .unwrap_or_else(|| "<no path>".into());
            let load_state = format!(
                "{:?}",
                asset_server
                    .get_load_state(id.untyped())
                    .map(|s| format!("{:?}", s))
            );
            scene_states.push(format!(
                "Scene[{}] in_assets={} state={}",
                path, loaded_in_assets, load_state
            ));
        }
        // Dump up to ~40 names of entities WITH a Mesh3d so we can see
        // whether the avatar's morph-bearing parts (CC_Base_Body etc.)
        // were spawned at all.
        let mesh_names: Vec<String> = mesh_q
            .iter()
            .filter_map(|_| None::<String>) // placeholder; we need a join query
            .collect();
        let _ = mesh_names; // unused — see next block
                            // Better: walk the named-query and report any whose name starts
                            // with a CC5 / avatar-part prefix.
        let mut avatar_parts: Vec<String> = Vec::new();
        for n in name_q.iter() {
            let s = n.as_str();
            if s.starts_with("CC_Base_")
                || s.starts_with("Brows_")
                || s.starts_with("Lash_")
                || s == "Side_part_wavy"
                || s.starts_with("Std_")
            {
                avatar_parts.push(s.to_string());
            }
        }
        avatar_parts.sort();
        avatar_parts.dedup();
        let avatar_parts_summary = if avatar_parts.is_empty() {
            "<none>".to_string()
        } else if avatar_parts.len() > 8 {
            format!(
                "{}... ({} total)",
                avatar_parts[..8].join(","),
                avatar_parts.len()
            )
        } else {
            avatar_parts.join(",")
        };

        let n_proto = diag_qs.p3().iter().count();
        tracing::info!(
            target: "cc_render",
            "apply_arkit_pose tick #{}: morph={} morph_only={} mesh_no_morph={} \
             all_mesh={} named={} proto_layout={} avatar_parts=[{}] scene_roots=[{}]",
            tick_val, n_morph, n_morph_only, n_mesh_no_morph, n_mesh, n_named,
            n_proto, avatar_parts_summary, scene_states.join(", ")
        );

        // One-time diagnostic: dump the ARKit map's keys and what they
        // map to. If the map's keys don't include "jawOpen" (or whatever
        // case Audio2Face emits), apply_arkit_pose's lookup will silently
        // produce 0 hits.
        if tick_val == 1 && !diag.dumped_morph_names {
            let mut keys: Vec<&String> = mapping.map.mapping.keys().collect();
            keys.sort();
            tracing::info!(
                target: "cc_render",
                "arkit_map: {} keys: [{}]",
                keys.len(),
                keys.iter().take(20).map(|s| s.as_str()).collect::<Vec<_>>().join(",")
            );
            // Show 3 sample entries with their refs (mesh names + morph
            // target names + weight).
            for k in ["jawOpen", "JawOpen", "mouthClose", "eyeBlinkLeft"] {
                if let Some(refs) = mapping.map.mapping.get(k) {
                    let summary: Vec<String> = refs
                        .iter()
                        .take(4)
                        .map(|r| format!("{}::{}*{:.2}", r.meshes.join("|"), r.morph, r.weight))
                        .collect();
                    tracing::info!(target: "cc_render", "  arkit_map['{}'] -> [{}]", k, summary.join(", "));
                } else {
                    tracing::info!(target: "cc_render", "  arkit_map['{}'] -> NOT FOUND", k);
                }
            }
        }
    }

    // Dump morph_target_names ONCE per session — but only after meshes
    // have actually been spawned (the GLB loads asynchronously, so on
    // the first few ticks `q` is empty). Gated on a Local<bool> flag.
    if !diag.dumped_morph_names && q.iter().count() > 0 {
        for (entity, _, _, children) in q.iter() {
            for &child in children.into_iter().flat_map(|c| c.iter()) {
                let Ok(mesh3d) = mesh_q.get(child) else {
                    continue;
                };
                let Some(mesh) = meshes.get(&mesh3d.0) else {
                    continue;
                };
                let Some(target_names) = mesh.morph_target_names() else {
                    continue;
                };
                let parent_name = name_q
                    .get(entity)
                    .map(|n| n.as_str().to_string())
                    .unwrap_or_else(|_| "?".into());
                // CRITICAL: morph_target_names being present does NOT mean
                // the morph DELTAS (vertex displacements) are loaded. Bevy
                // 0.15 stores those in `mesh.morph_targets()` as an `Image`
                // asset; if that's None, MorphWeights writes are GPU no-ops.
                let has_targets_image = mesh.morph_targets().is_some();
                let has_jaw_open = target_names.iter().any(|n| n == "Jaw_Open");
                let opens: Vec<&str> = target_names
                    .iter()
                    .filter(|n| n.contains("Open") || n.contains("open"))
                    .map(|s| s.as_str())
                    .collect();
                tracing::info!(
                    target: "cc_render",
                    "morph_target_names[{}] n={} HAS_DELTA_IMAGE={} has_Jaw_Open={} *Open*=[{}]",
                    parent_name,
                    target_names.len(),
                    has_targets_image,
                    has_jaw_open,
                    opens.join(",")
                );
            }
        }
        diag.dumped_morph_names = true;
    }

    // Snapshot the latest pose without blocking.
    let mut pose = pose_rx.0.borrow_and_update().clone();
    let max_w = pose.weights.values().copied().fold(0.0_f32, f32::max);
    tracing::debug!(
        target: "timing",
        stage = "bevy_apply_bs",
        pts_ms = pose.pts_ms as i64,
        max_w = max_w as f64,
        n_weights = pose.weights.len() as u64,
    );
    last_pts.0 = pose.pts_ms;

    // CC_RENDER_DEBUG_OVERRIDE=<key> short-circuits the upstream pose stream
    // and substitutes a single hard-coded weight=1.0 on that ARKit channel.
    // Useful as a sanity test: e.g. CC_RENDER_DEBUG_OVERRIDE=jawOpen will
    // pin the mouth open at full amplitude, isolating whether the morph-
    // application code path produces VISIBLE deformation independent of
    // Audio2Face's signal magnitude or timing.
    if let Ok(key) = std::env::var("CC_RENDER_DEBUG_OVERRIDE") {
        if !key.is_empty() {
            // Amplifier (default 1.0). CC5 visemes peak at ~5mm — below
            // pixel resolution at 512² face crops. Set CC_RENDER_DEBUG_AMP=10
            // to drive the weight far past 1.0 (Bevy doesn't clamp morph
            // weights). Useful for verifying the morph path produces
            // visible deformation independent of texture resolution.
            let amp: f32 = std::env::var("CC_RENDER_DEBUG_AMP")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.0);
            pose.weights.clear();
            pose.weights.insert(key.clone(), amp);
            if tick_val == 1 || tick_val % 60 == 0 {
                tracing::info!(
                    target: "cc_render",
                    "apply_arkit_pose: DEBUG override active — {}={:.2} (other weights cleared)",
                    key, amp
                );
            }
        }
    }

    if pose.weights.is_empty() {
        return;
    }

    // Restrict morph application to the avatar SceneRoot subtree so a
    // separately-loaded environment scene with same-named meshes can't
    // accidentally pick up morph weights.
    let avatars: Vec<Entity> = avatar_q.iter().collect();

    for (entity, own_name, mut weights, children) in q.iter_mut() {
        if !is_under_avatar(entity, &parent_q, &avatars) {
            continue;
        }
        // Resolve the CC5 mesh name. Bevy 0.15 glTF puts `Name` on the
        // node entity (which is also the MorphWeights-bearing one);
        // we still fall back to ancestor walk for any layout where the
        // name is on a higher node.
        let mesh_name: Option<String> = own_name
            .map(|n| n.as_str().to_string())
            .or_else(|| ancestor_name(entity, &parent_q, &name_q));
        let Some(mesh_name) = mesh_name else {
            continue;
        };

        // Iterate the children, computing per-child morph weights
        // based on each child mesh's OWN morph_target_names ordering.
        // Different primitives can have different morph orderings;
        // collapsing them all to the first child's ordering would
        // smear weights across the wrong targets.
        let n_parent = weights.weights().len();
        let mut wrote_any = false;
        let mut last_hits = 0usize;
        let mut last_max = 0.0_f32;
        let mut last_n_targets = 0usize;
        let mut last_new_w_for_parent: Option<Vec<f32>> = None;

        for &child in children.into_iter().flat_map(|c| c.iter()) {
            let Ok(mesh3d) = mesh_q.get(child) else {
                continue;
            };
            let Some(mesh) = meshes.get(&mesh3d.0) else {
                continue;
            };
            let Some(target_names) = mesh.morph_target_names() else {
                continue;
            };
            let n = target_names.len();
            if n == 0 {
                continue;
            }

            let mut name_to_idx: HashMap<&str, usize> = HashMap::with_capacity(n);
            for (i, t) in target_names.iter().enumerate() {
                name_to_idx.insert(t.as_str(), i);
            }

            let mut new_w = vec![0.0_f32; n];
            let mut hits = 0usize;
            for (arkit, w) in &pose.weights {
                if !w.is_finite() || *w == 0.0 {
                    continue;
                }
                let Some(refs) = mapping.map.mapping.get(arkit) else {
                    continue;
                };
                for r in refs {
                    if !r.meshes.iter().any(|m| m == &mesh_name) {
                        continue;
                    }
                    if let Some(&idx) = name_to_idx.get(r.morph.as_str()) {
                        // CC_RENDER_DEBUG_AMP multiplies upstream weights AND
                        // raises the clamp ceiling. Audio2Face emits peaks
                        // ~0.4 across the envelope; AMP=3 lifts that to ~1.2
                        // for visible motion at face-close camera framing.
                        // (Default 1.0 = no-op pass-through.)
                        let amp = std::env::var("CC_RENDER_DEBUG_AMP")
                            .ok()
                            .and_then(|s| s.parse::<f32>().ok())
                            .unwrap_or(1.0);
                        let upper = amp.max(1.0);
                        new_w[idx] = (new_w[idx] + w * r.weight * amp).clamp(0.0, upper);
                        hits += 1;
                    }
                }
            }

            // Track the most-active write for the periodic log line +
            // for the parent buffer mirror (chosen as the largest n
            // across siblings — heuristic for "this is the canonical
            // buffer"). All children with matching n get the actual
            // weight write.
            if hits > last_hits || last_new_w_for_parent.is_none() {
                last_hits = hits;
                let m = new_w.iter().copied().fold(0.0_f32, |a, b| a.max(b.abs()));
                if m >= last_max {
                    last_max = m;
                }
                last_n_targets = n;
                if n == n_parent {
                    last_new_w_for_parent = Some(new_w.clone());
                }
            }

            // Write to the child's MorphWeights via Commands. The GPU
            // morph extraction reads MorphWeights from the Mesh3d
            // entity, so this is the load-bearing write.
            if let Ok(component) = MorphWeights::new(new_w, None) {
                commands.entity(child).insert(component);
            }
            wrote_any = true;
        }

        // Mirror the canonical buffer to the parent's MorphWeights
        // (best-effort) so any inheritance system Bevy DOES wire up
        // also sees it. Skipped if no child shared the parent's
        // weight-buffer length.
        if let Some(canonical) = last_new_w_for_parent {
            weights.weights_mut().copy_from_slice(&canonical);
        }

        if wrote_any && tick_val % 30 == 0 {
            // debug — emitted at 6+ lines/sec/mesh (one per parent
            // mesh modulo 30 ticks) and the lines are large; demoted
            // from info because the synchronous tee→grep→file pipe
            // in run-avatar-s2s.sh starves the Bevy thread when this
            // is on by default.
            tracing::debug!(
                target: "cc_render",
                "morph_apply tick={} mesh={} hits={} max_w={:.3} target_names={}",
                tick_val, mesh_name, last_hits, last_max, last_n_targets
            );
        }
    }
}

/// Walk up the `ChildOf` chain looking for the nearest ancestor with a
/// `Name` component. Returns `None` if no named ancestor is found
/// (shouldn't happen for a glTF-imported scene; defensive).
fn ancestor_name(
    entity: Entity,
    parent_q: &Query<&Parent>,
    name_q: &Query<&Name>,
) -> Option<String> {
    let mut current = entity;
    // Bound the walk to prevent infinite loops on pathological scenes.
    for _ in 0..32 {
        let Ok(parent_ref) = parent_q.get(current) else {
            return None;
        };
        let parent = parent_ref.get();
        if let Ok(name) = name_q.get(parent) {
            return Some(name.as_str().to_string());
        }
        current = parent;
    }
    None
}

/// True iff `entity` is the avatar `SceneRoot` itself or one of its
/// descendants. Used to scope name-based bone walks (jaw / eye / CC5
/// joint names) to the avatar so a separately-loaded environment scene
/// can't accidentally match — even if it happens to ship entities with
/// the same names.
///
/// `avatars` is empty when the avatar `SceneRoot` hasn't spawned yet
/// (very early ticks). In that case we return `true` so existing
/// systems retain their pre-marker behavior and don't silently no-op
/// during scene warmup.
pub(crate) fn is_under_avatar(
    entity: Entity,
    parent_q: &Query<&Parent>,
    avatars: &[Entity],
) -> bool {
    if avatars.is_empty() {
        return true;
    }
    if avatars.contains(&entity) {
        return true;
    }
    let mut current = entity;
    // Bound the walk to prevent infinite loops on pathological scenes.
    for _ in 0..64 {
        let Ok(parent_ref) = parent_q.get(current) else {
            return false;
        };
        let parent = parent_ref.get();
        if avatars.contains(&parent) {
            return true;
        }
        current = parent;
    }
    false
}

/// Holds the animation graph + node index used to play the GLB's
/// first animation clip (the FBX-exported pose, e.g. sitting). Bevy
/// 0.15 doesn't auto-play glTF animations — we build the graph in
/// `setup_scene` and `start_pose_animation` attaches it to the
/// AnimationPlayer entity once SceneSpawner spawns it.
#[derive(Resource, Default)]
pub(crate) struct PoseAnimation {
    pub graph: Option<Handle<AnimationGraph>>,
    pub node: Option<AnimationNodeIndex>,
}

pub(crate) fn start_pose_animation(
    mut commands: Commands,
    pose_anim: Res<PoseAnimation>,
    mut q: Query<(Entity, &mut AnimationPlayer), Added<AnimationPlayer>>,
    parent_q: Query<&Parent>,
    avatar_q: Query<Entity, With<AvatarRoot>>,
) {
    // CC_RENDER_DISABLE_BAKED_ANIM defaults to ON. Embedded FBX animations
    // (TempMotion etc.) interfere with skeletal driving — they overdrive
    // bones at frame 0 and cause arm-cross / palm-twist artifacts. Set
    // CC_RENDER_DISABLE_BAKED_ANIM=0 to opt back in (e.g. when rendering
    // the avatar's authored idle without skeletal pose input).
    if std::env::var("CC_RENDER_DISABLE_BAKED_ANIM")
        .ok()
        .as_deref()
        != Some("0")
    {
        for (_entity, _player) in q.iter_mut() {
            tracing::warn!(
                target: "cc_render",
                "skeletal: baked animation disabled (default); set CC_RENDER_DISABLE_BAKED_ANIM=0 to re-enable"
            );
        }
        return;
    }
    let (Some(graph), Some(node)) = (pose_anim.graph.clone(), pose_anim.node) else {
        return;
    };
    // Scope to AnimationPlayers under the avatar SceneRoot so an
    // optional environment scene with its own AnimationPlayer doesn't
    // get the avatar's clip stamped onto it.
    let avatars: Vec<Entity> = avatar_q.iter().collect();
    for (entity, mut player) in q.iter_mut() {
        if !is_under_avatar(entity, &parent_q, &avatars) {
            continue;
        }
        commands
            .entity(entity)
            .insert(AnimationGraphHandle(graph.clone()));
        player.play(node).repeat();
    }
}

/// Walk the entity hierarchy upward from `start`, accumulating
/// each ancestor's `Transform.rotation` in root-→-leaf order. Skips
/// `start` itself — the result is the world rest rotation of `start`'s
/// parent chain, matching `mrt.world_rest_rotation(g, node_idx)` in
/// `scripts/avatars/mixamo_retarget.py`.
fn parent_chain_world_rotation(
    start: Entity,
    parent_q: &Query<&Parent>,
    transform_q: &Query<&Transform>,
) -> Quat {
    // Collect ancestors leaf→root.
    let mut chain: Vec<Entity> = Vec::with_capacity(8);
    let mut cur = start;
    while let Ok(p) = parent_q.get(cur) {
        let parent_entity = p.get();
        chain.push(parent_entity);
        cur = parent_entity;
    }
    // Walk root→leaf accumulating rotations.
    let mut q = Quat::IDENTITY;
    for entity in chain.iter().rev() {
        if let Ok(t) = transform_q.get(*entity) {
            q *= t.rotation;
        }
    }
    q
}

/// Walks the spawned scene looking for the 22 CC5 bones in
/// `SMPL22_TO_CC5`. The first frame all 22 are found, capture each
/// bone's bind-pose local rotation, walk parent chains for the world-
/// rest-rotation precompute, and derive the per-bone constants
/// (`cc5_rest_world`, `inter_rest`) needed by `apply_skeletal_pose`.
///
/// Runs every frame UNTIL captured (then early-returns). Cheap either
/// way: `name_q.iter()` is O(named entities) — small — and we exit on
/// first success.
pub(crate) fn capture_cc5_bind_rotations(
    name_q: Query<(Entity, &Name)>,
    transform_q: Query<&Transform>,
    parent_q: Query<&Parent>,
    transform_global_q: Query<&GlobalTransform>,
    mut bind: ResMut<CC5BindRotations>,
    avatar_q: Query<Entity, With<AvatarRoot>>,
) {
    if bind.captured {
        return;
    }

    // Restrict the bone search to the avatar SceneRoot subtree. An
    // environment scene shouldn't ship `CC_Base_*` named entities, but
    // this guards against that edge case (and keeps the name→entity
    // map smaller when one is present).
    let avatars: Vec<Entity> = avatar_q.iter().collect();

    // First pass: build a name → Entity map for everything currently
    // in the world that we care about. Only the 22 CC5 bones have
    // distinctive names; everything else is ignored.
    let mut name_to_entity: HashMap<&str, Entity> = HashMap::new();
    for (entity, name) in name_q.iter() {
        let s = name.as_str();
        // Cheap filter: only stash CC_Base_* entities to keep the map
        // small. (CC5 prefixes everything in the avatar's skeleton with
        // CC_Base_; non-bone scene props don't.)
        if !s.starts_with("CC_Base_") {
            continue;
        }
        if !is_under_avatar(entity, &parent_q, &avatars) {
            continue;
        }
        name_to_entity.insert(s, entity);
    }

    // Need all 22 present before we lock in.
    let mut found: [Option<Entity>; 22] = [None; 22];
    for (i, (_smpl, cc5)) in SMPL22_TO_CC5.iter().enumerate() {
        if let Some(&e) = name_to_entity.get(cc5) {
            found[i] = Some(e);
        } else {
            return; // wait for the GLB to finish settling
        }
    }

    // All 22 present; capture bind-pose local rotations + parent chain
    // rest world rotations.
    let mut own_rest_local: [Quat; 22] = [Quat::IDENTITY; 22];
    let mut parent_chain_rest: [Quat; 22] = [Quat::IDENTITY; 22];
    let mut pelvis_rest_translation = Vec3::ZERO;

    for i in 0..22 {
        let entity = found[i].expect("checked above");
        let Ok(t) = transform_q.get(entity) else {
            tracing::warn!(
                target: "cc_render",
                "skeletal: bone {} ({}) entity has no Transform — retry",
                SMPL22_TO_CC5[i].1, SMPL22_TO_CC5[i].0
            );
            return;
        };
        own_rest_local[i] = t.rotation;
        if i == 0 {
            pelvis_rest_translation = t.translation;
        }
        parent_chain_rest[i] = parent_chain_world_rotation(entity, &parent_q, &transform_q);
    }

    // cc5_rest_world[i] = parent_chain_rest[i] * own_rest_local[i]
    let mut cc5_rest_world: [Quat; 22] = [Quat::IDENTITY; 22];
    for i in 0..22 {
        cc5_rest_world[i] = parent_chain_rest[i] * own_rest_local[i];
    }

    // inter_rest[i] captures the contribution of CC5 intermediate
    // (non-mapped) bones between this bone and its SMPL-mapped parent.
    // Pelvis: identity (no SMPL parent).
    let mut inter_rest: [Quat; 22] = [Quat::IDENTITY; 22];
    for i in 0..22 {
        let parent_smpl = SMPL_PARENTS[i];
        if parent_smpl < 0 {
            inter_rest[i] = Quat::IDENTITY;
            continue;
        }
        let p = parent_smpl as usize;
        // inter_rest = inv(cc5_rest_world[parent]) * parent_chain_rest[child]
        // Hamilton-product order matches mixamo's `quat_mul(quat_inverse(...), ...)`.
        inter_rest[i] = cc5_rest_world[p].inverse() * parent_chain_rest[i];
    }

    // World-space pelvis + eye axis for the auto-orient system. We use
    // `GlobalTransform`, which Bevy populates after the first
    // `TransformSystem::TransformPropagate` tick — by the time this
    // system passes its "all 22 bones present" gate, that's already
    // happened. If it hasn't (extremely cold-start case), the value is
    // identity and `auto_orient_avatar` simply skips this frame.
    let pelvis_entity = found[0].expect("pelvis present");
    let pelvis_world = transform_global_q
        .get(pelvis_entity)
        .ok()
        .map(|gt| gt.translation());

    let eye_axis_world = {
        let l_eye = name_to_entity.get("CC_Base_L_Eye").copied();
        let r_eye = name_to_entity.get("CC_Base_R_Eye").copied();
        match (l_eye, r_eye) {
            (Some(l), Some(r)) => {
                let lp = transform_global_q.get(l).ok().map(|gt| gt.translation());
                let rp = transform_global_q.get(r).ok().map(|gt| gt.translation());
                match (lp, rp) {
                    (Some(lpos), Some(rpos)) => {
                        let v = rpos - lpos;
                        if v.length_squared() > 1e-8 {
                            Some(v.normalize())
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => {
                tracing::warn!(
                    target: "cc_render",
                    "auto-orient: CC_Base_L_Eye / CC_Base_R_Eye not found; \
                     auto facing-camera will be skipped (l={:?} r={:?})",
                    l_eye, r_eye,
                );
                None
            }
        }
    };

    // ─── Per-bone bind world directions (for axis_match mode) ───────
    // For each retargeted bone, capture its world position and the
    // world position of its primary child (if any). Direction =
    // normalize(child_world_pos - bone_world_pos). Leaves fall back
    // to the bone's bind world rotation applied to SMPL_BONE_AXES[i].
    let mut bone_world_pos: [Vec3; 22] = [Vec3::ZERO; 22];
    for i in 0..22 {
        let entity = found[i].expect("checked above");
        if let Ok(gt) = transform_global_q.get(entity) {
            bone_world_pos[i] = gt.translation();
        }
    }
    let mut cc5_bone_axis_world: [Vec3; 22] = [Vec3::ZERO; 22];
    let mut cc5_bone_tip_world: [Vec3; 22] = [Vec3::ZERO; 22];
    for i in 0..22 {
        let dir = match SMPL_PRIMARY_CHILD[i] {
            Some(child_smpl) => {
                let v = bone_world_pos[child_smpl] - bone_world_pos[i];
                if v.length_squared() > 1e-8 {
                    cc5_bone_tip_world[i] = bone_world_pos[child_smpl];
                    v.normalize()
                } else {
                    // Coincident parent/child positions — fall back.
                    let smpl_axis = Vec3::from_array(SMPL_BONE_AXES[i]);
                    let fallback = (cc5_rest_world[i] * smpl_axis).normalize_or_zero();
                    cc5_bone_tip_world[i] = bone_world_pos[i] + fallback * 0.05;
                    fallback
                }
            }
            None => {
                // Leaf: use bind world rotation applied to SMPL_BONE_AXES.
                let smpl_axis = Vec3::from_array(SMPL_BONE_AXES[i]);
                let dir = (cc5_rest_world[i] * smpl_axis).normalize_or_zero();
                cc5_bone_tip_world[i] = bone_world_pos[i] + dir * 0.05;
                dir
            }
        };
        cc5_bone_axis_world[i] = dir;
    }

    bind.own_rest_local = own_rest_local;
    bind.parent_chain_rest = parent_chain_rest;
    bind.cc5_rest_world = cc5_rest_world;
    bind.inter_rest = inter_rest;
    bind.entities = found;
    bind.pelvis_rest_translation = pelvis_rest_translation;
    bind.pelvis_world = pelvis_world;
    bind.eye_axis_world = eye_axis_world;
    bind.cc5_bone_axis_world = cc5_bone_axis_world;
    bind.cc5_bone_tip_world = cc5_bone_tip_world;
    bind.captured = true;
    tracing::info!(
        target: "cc_render",
        "skeletal: captured FK rest data for 22 SMPL-mapped bones \
         (pelvis_rest_translation={:?} pelvis_world={:?} eye_axis_world={:?})",
        pelvis_rest_translation,
        pelvis_world,
        eye_axis_world,
    );
}

/// Stream-driven full-body animation. When a `SkeletalPose` is present
/// on the watch channel, run a SMPL→CC5 full FK retarget per frame and
/// write per-bone rotations on the 22 mapped CC5 bones, plus a parent-
/// local pelvis translation for root-locomotion.
///
/// **Math** (per frame, walked in `SMPL_TOPO` order):
///
/// 1. `W_smpl[i] = W_smpl[parent] * src_local[i]` — SMPL world
///    rotation. Pelvis (no parent): `W_smpl[0] = src_local[0]`. SMPL
///    bind pose has identity local rotations, so `W_smpl[i]` is also
///    the world delta from rest.
/// 2. `W_cc5[i] = W_smpl[i] * cc5_rest_world[i]` — apply the same
///    world delta on top of CC5's rest world rotation.
/// 3. Convert to CC5-local using the *animated* parent world rotation:
///    - Pelvis: `parent_world = parent_chain_rest[0]` (its CC5 parent
///      doesn't move).
///    - Other: `parent_world = W_cc5[parent_smpl] * inter_rest[i]`,
///      composing the parent's animated CC5 world with the rest
///      contribution of any non-mapped intermediate bones.
///    - `cc5_local[i] = inv(parent_world) * W_cc5[i]`
///
/// **Root translation** (pelvis only):
/// - First pose of each streaming session sets `anchor_root_pos`.
/// - `delta_world = pose.root_pos - anchor`
/// - `delta_local = inv(parent_chain_rest[0]) * delta_world` (rotated)
/// - `pelvis.translation = pelvis_rest_translation + delta_local`
///
/// While a skeletal stream is active, the baked AnimationPlayer is
/// paused so it doesn't fight per-bone overrides. Sending `None` on
/// the channel resumes it AND clears the root anchor.
///
/// **Runtime defaults (env-overridable):**
/// - `CC_AVATAR_RETARGET_MODE=axis_match_chained` — bind-tilt-invariant
///   retargeter; corrects accumulated chain error per-bone. Modes:
///   `world` / `local` / `axis_match` available for A/B testing.
/// - `CC_AVATAR_FREEZE_LEAF_BONES=1` — leaf bones (wrists, feet, head)
///   keep their bind orientation; set `0` to opt out.
/// - `CC_RENDER_DISABLE_BAKED_ANIM=1` — embedded FBX animations
///   (TempMotion etc.) are disabled by default to avoid frame-0
///   bone-overdrive artifacts; set `0` to re-enable.
pub(crate) fn apply_skeletal_pose(
    rx: Res<SkeletalPoseWatchRx>,
    bind: Res<CC5BindRotations>,
    mut transforms: Query<&mut Transform>,
    mut players: Query<&mut AnimationPlayer>,
    mut active: Local<bool>,
    mut anchor_root_pos: Local<Option<Vec3>>,
    mut tick: Local<u64>,
    mut last_pose: Local<Option<SkeletalPose>>,
    mut idle_ticks: Local<u64>,
    mut total_streams: Local<u64>,
) {
    if !bind.captured {
        return;
    }

    *tick += 1;

    // Queue drain: pull at most ONE pose this frame so paced motion
    // plays through in order (FIFO). If the producer is briefly faster
    // than us the surplus stays in the queue; if it's slower we re-use
    // `last_pose` to keep the avatar held at its most recent pose
    // rather than snapping back to baked animation between updates.
    let mut cleared = false;
    let mut new_pose: Option<SkeletalPose> = None;
    match rx.0.try_recv() {
        Ok(Some(p)) => new_pose = Some(p),
        Ok(None) => cleared = true,
        Err(crossbeam_channel::TryRecvError::Empty) => {}
        Err(crossbeam_channel::TryRecvError::Disconnected) => {}
    }

    // Burst-edge logging at INFO so we can correlate pipeline events to
    // actual avatar driving without enabling firehose `timing=debug`.
    // Two transitions matter: idle→streaming (first pose of a new burst)
    // and streaming→idle (~1 s after the last pose of a burst).
    if new_pose.is_some() {
        if *idle_ticks > 0 {
            *total_streams += 1;
            let p = new_pose.as_ref().expect("checked above");
            // R_Shoulder / R_Elbow: wave gestures put non-trivial swing
            // here. Identity (≈0,0,0,1) across the whole burst means the
            // daemon emitted a no-op clip — not a Bevy render bug.
            let q_rsh = p.joint_quats[17];
            let q_relb = p.joint_quats[19];
            tracing::info!(
                target: "cc_render",
                "skeletal: burst-edge #{} (idle={}, active={}, \
                 pts_ms={}, root=({:.3},{:.3},{:.3}), \
                 R_Shoulder=({:.3},{:.3},{:.3},{:.3}), \
                 R_Elbow=({:.3},{:.3},{:.3},{:.3}))",
                *total_streams, *idle_ticks, *active,
                p.pts_ms, p.root_pos[0], p.root_pos[1], p.root_pos[2],
                q_rsh[0], q_rsh[1], q_rsh[2], q_rsh[3],
                q_relb[0], q_relb[1], q_relb[2], q_relb[3],
            );
        }
        *idle_ticks = 0;
    } else {
        *idle_ticks += 1;
        if *idle_ticks == 30 && *active && !cleared {
            tracing::info!(
                target: "cc_render",
                "skeletal: stream went idle (~1 s of no poses; \
                 *active still true, last_pose held)"
            );
        }
    }

    if cleared {
        tracing::debug!(
            target: "timing",
            stage = "bevy_apply_skel",
            pts_ms = -1_i64,
            mode = "cleared",
        );
        *last_pose = None;
        if *active {
            for mut player in players.iter_mut() {
                player.resume_all();
            }
            *active = false;
            *anchor_root_pos = None;
        }
        return;
    }

    if let Some(p) = new_pose {
        *last_pose = Some(p);
    }

    let Some(pose) = last_pose.clone() else {
        tracing::debug!(
            target: "timing",
            stage = "bevy_apply_skel",
            pts_ms = -1_i64,
            mode = "none",
        );
        return;
    };
    tracing::debug!(
        target: "timing",
        stage = "bevy_apply_skel",
        pts_ms = pose.pts_ms as i64,
        root_y = pose.root_pos[1] as f64,
    );

    // First time we see a skeletal pose, pause the baked animation so
    // it doesn't double-drive the bones we're about to overwrite.
    if !*active {
        for mut player in players.iter_mut() {
            player.pause_all();
        }
        *active = true;
        *anchor_root_pos = None;
        tracing::info!(
            target: "cc_render",
            "skeletal: stream active, paused AnimationPlayer ({} player(s))",
            players.iter().count()
        );
    }

    // Anchor root_pos at the first pose of this streaming session so
    // subsequent root_pos deltas are relative — same convention as
    // smpl_to_cc5_retarget.py's `root_pos[f] - root_pos[0]`.
    let pose_root = Vec3::new(pose.root_pos[0], pose.root_pos[1], pose.root_pos[2]);
    if anchor_root_pos.is_none() {
        *anchor_root_pos = Some(pose_root);
    }
    let anchor = anchor_root_pos.expect("set above");

    // ─── Per-frame FK retarget (pure function, unit-tested) ──────────
    //
    // CC_AVATAR_SMPL_ALIGN_Y_DEG (default 0) prepends a Y-axis rotation
    // to the SMPL pose at the pelvis to compensate for bone-local axis
    // convention differences between SMPL and the host CC5 rig. Raw
    // CC5/Reallusion exports (no FBX bake) typically need 180; rigs
    // whose bake already includes a root rotation need 0.
    let align_y_deg = std::env::var("CC_AVATAR_SMPL_ALIGN_Y_DEG")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.0);
    let r_align = Quat::from_rotation_y(align_y_deg.to_radians());

    // CC_AVATAR_RETARGET_MODE: default `axis_match_chained` (the chain-aware
    // retargeter — bind-tilt-invariant, works on any rig regardless of
    // A-pose vs T-pose authoring). Other modes available for A/B testing:
    //   - "world"        full-FK (legacy default)
    //   - "local"        per-bone local composition (no axis match)
    //   - "axis_match"   non-chained R_corr conjugation
    let mode = std::env::var("CC_AVATAR_RETARGET_MODE")
        .ok()
        .unwrap_or_else(|| "axis_match_chained".to_string());

    let cc5_local: [Quat; 22] = match mode.as_str() {
        "local" => {
            let mut out: [Quat; 22] = [Quat::IDENTITY; 22];
            for i in 0..22 {
                let raw = Quat::from_xyzw(
                    pose.joint_quats[i][0],
                    pose.joint_quats[i][1],
                    pose.joint_quats[i][2],
                    pose.joint_quats[i][3],
                );
                let dq_smpl = if raw.length_squared() > 1e-8 {
                    raw.normalize()
                } else {
                    Quat::IDENTITY
                };
                let dq = if i == 0 { r_align * dq_smpl } else { dq_smpl };
                out[i] = bind.own_rest_local[i] * dq;
            }
            out
        }
        "axis_match" => {
            // CC_AVATAR_FREEZE_LEAF_BONES defaults to ON. Keeps leaf bones
            // (wrists, feet, head) at their bind orientation while still
            // moving with the parent chain. The leaf-bone bind axis
            // fallback (`cc5_rest_world * SMPL_BONE_AXES`) doesn't match
            // SMPL's twist convention on most rigs, producing wrong
            // wrist/hand orientation. Set CC_AVATAR_FREEZE_LEAF_BONES=0
            // to opt back into leaf-bone driving.
            let freeze_leaves =
                std::env::var("CC_AVATAR_FREEZE_LEAF_BONES").ok().as_deref() != Some("0");

            // Per-bone full-rotation conjugation. For each bone:
            //
            //   R_corr[i] = from_rotation_arc(SMPL_BONE_AXES[i],
            //                                 cc5_bone_axis_world[i])
            //
            // R_corr is a swing-only rotation that maps SMPL's bind
            // bone-axis to CC5's bind bone-axis. Conjugating w_smpl[i]
            // through R_corr re-axes the rotation so its bone-axis
            // basis matches CC5's bone-axis basis at bind:
            //
            //   - Twist around SMPL bone-axis  →  twist around CC5 bone-axis
            //   - Swing around perp(SMPL axis) →  swing around perp(CC5 axis)
            //
            // Then apply on top of CC5 bind world:
            //   cc5_world[i] = R_corr · w_smpl[i] · R_corrᵀ · cc5_rest_world[i]
            //
            // Direction matches SMPL's posed direction (via swing re-axing);
            // wrist/hand twist is preserved (via conjugation through the
            // bone axis); chain accumulation is implicit in w_smpl.
            //
            // For pelvis: skip conjugation (R_corr would be identity-ish
            // anyway since SMPL_BONE_AXES[0]=+Y matches the spine direction
            // in any reasonable rig).
            let mut w_smpl: [Quat; 22] = [Quat::IDENTITY; 22];
            let mut w_cc5: [Quat; 22] = [Quat::IDENTITY; 22];
            let mut out: [Quat; 22] = [Quat::IDENTITY; 22];

            for &i in SMPL_TOPO.iter() {
                let parent = SMPL_PARENTS[i];
                let raw = Quat::from_xyzw(
                    pose.joint_quats[i][0],
                    pose.joint_quats[i][1],
                    pose.joint_quats[i][2],
                    pose.joint_quats[i][3],
                );
                let dq_smpl = if raw.length_squared() > 1e-8 {
                    raw.normalize()
                } else {
                    Quat::IDENTITY
                };
                let dq_smpl = if i == 0 { r_align * dq_smpl } else { dq_smpl };

                // SMPL world chain.
                w_smpl[i] = if parent < 0 {
                    dq_smpl
                } else {
                    w_smpl[parent as usize] * dq_smpl
                };

                // Leaf-bone freeze: substitute the parent's animated
                // delta as this bone's world rotation, so the bone moves
                // with the chain but doesn't get its own SMPL rotation
                // re-axed (which can produce wrong wrist twists).
                let is_leaf = SMPL_PRIMARY_CHILD[i].is_none();
                if freeze_leaves && is_leaf && parent >= 0 {
                    // Use parent's animated CC5 world (with inter_rest)
                    // composed with bind's own_rest_local — i.e. this
                    // bone stays at bind-relative-to-parent.
                    let parent_anim_world = w_cc5[parent as usize] * bind.inter_rest[i];
                    w_cc5[i] = parent_anim_world * bind.own_rest_local[i];
                } else {
                    let smpl_bind_axis = Vec3::from_array(SMPL_BONE_AXES[i]);
                    let cc5_bind_axis = bind.cc5_bone_axis_world[i];
                    let r_corr = if cc5_bind_axis.length_squared() > 1e-8
                        && smpl_bind_axis.length_squared() > 1e-8
                    {
                        Quat::from_rotation_arc(
                            smpl_bind_axis.normalize(),
                            cc5_bind_axis.normalize(),
                        )
                    } else {
                        Quat::IDENTITY
                    };

                    // Conjugate the SMPL world rotation through R_corr to
                    // re-axis it into CC5's bind bone-axis basis. Then apply
                    // on top of CC5 bind world.
                    let dq_corrected = r_corr * w_smpl[i] * r_corr.inverse();
                    w_cc5[i] = dq_corrected * bind.cc5_rest_world[i];
                }

                // Convert to local using animated parent world (with
                // inter_rest correction for non-mapped CC5 intermediates).
                let parent_world_for_local = if parent < 0 {
                    bind.parent_chain_rest[0]
                } else {
                    w_cc5[parent as usize] * bind.inter_rest[i]
                };
                out[i] = parent_world_for_local.inverse() * w_cc5[i];
            }
            out
        }
        "axis_match_chained" => {
            // Chain-aware axis match WITH twist preservation. See
            // `fk_retarget_smpl_to_cc5_axis_match_chained` doc-comment
            // for the full math; the inner loop is unit-tested
            // (pose::fk_tests::axis_match_chained_preserves_y_twist_at_pelvis).
            //
            // FREEZE_LEAF_BONES default ON: keeps wrist/foot/head bones
            // at bind orientation relative to parent on rigs whose
            // distal bone-axis basis doesn't match SMPL's twist
            // convention. The chain still propagates through them, so
            // a body spin still rotates the head world-space via Neck.
            let freeze_leaves =
                std::env::var("CC_AVATAR_FREEZE_LEAF_BONES").ok().as_deref() != Some("0");

            fk_retarget_smpl_to_cc5_axis_match_chained(
                &pose.joint_quats,
                &bind.cc5_rest_world,
                &bind.cc5_bone_axis_world,
                &bind.inter_rest,
                &bind.own_rest_local,
                bind.parent_chain_rest[0],
                r_align,
                freeze_leaves,
            )
        }
        _ => fk_retarget_smpl_to_cc5_with_align(
            &pose.joint_quats,
            &bind.cc5_rest_world,
            &bind.inter_rest,
            bind.parent_chain_rest[0],
            r_align,
        ),
    };

    // ─── Diagnostic: dump SMPL pose values for shoulders/elbows. ─────
    // CC_RENDER_LOG_SMPL_POSE=1 prints, every N ticks (default 30):
    //   - SMPL local quat for each retargeted bone (raw input data)
    //   - Computed SMPL world rotation (chain-accumulated)
    //   - Computed SMPL bone direction in world (where SMPL says the
    //     bone points: w_smpl * SMPL_BONE_AXES[i])
    //   - Final cc5_local[i] (what we write to the bone)
    //
    // Lets us distinguish "SMPL data has arms behind" (content) from
    // "our retargeting flips the direction" (math bug). If SMPL bone
    // directions show arms going backward (negative Z when avatar
    // faces +Z), the source motion encodes that; otherwise the bug
    // is downstream.
    if std::env::var("CC_RENDER_LOG_SMPL_POSE").ok().as_deref() == Some("1") {
        let log_every: u64 = std::env::var("CC_RENDER_LOG_SMPL_POSE_EVERY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        if *tick % log_every == 0 {
            // Re-derive w_smpl chain so we can report SMPL world dirs
            // even in `local` mode (which doesn't compute it).
            let mut w_smpl: [Quat; 22] = [Quat::IDENTITY; 22];
            for &i in SMPL_TOPO.iter() {
                let parent = SMPL_PARENTS[i];
                let raw = Quat::from_xyzw(
                    pose.joint_quats[i][0],
                    pose.joint_quats[i][1],
                    pose.joint_quats[i][2],
                    pose.joint_quats[i][3],
                );
                let dq = if raw.length_squared() > 1e-8 {
                    raw.normalize()
                } else {
                    Quat::IDENTITY
                };
                w_smpl[i] = if parent < 0 {
                    dq
                } else {
                    w_smpl[parent as usize] * dq
                };
            }
            // Bones of interest: pelvis(0), spine3(9), L_Collar(13),
            // R_Collar(14), L_Shoulder(16), R_Shoulder(17), L_Elbow(18),
            // R_Elbow(19), L_Wrist(20), R_Wrist(21).
            let names_idx = [
                ("Pelvis", 0_usize),
                ("Spine3", 9),
                ("L_Collar", 13),
                ("R_Collar", 14),
                ("L_Shoulder", 16),
                ("R_Shoulder", 17),
                ("L_Elbow", 18),
                ("R_Elbow", 19),
                ("L_Wrist", 20),
                ("R_Wrist", 21),
            ];
            tracing::info!(
                target: "cc_render",
                "─── SMPL pose dump (tick #{}, pts_ms={}) — mode={} ───",
                *tick, pose.pts_ms, mode
            );
            for (name, i) in names_idx.iter() {
                let smpl_local_q = pose.joint_quats[*i];
                let smpl_dir =
                    (w_smpl[*i] * Vec3::from_array(SMPL_BONE_AXES[*i])).normalize_or_zero();
                let cc5_dir = bind.cc5_bone_axis_world[*i];
                tracing::info!(
                    target: "cc_render",
                    "  {:>10} smpl_local=[{:+.3},{:+.3},{:+.3},{:+.3}] \
                     smpl_world_dir=({:+.3},{:+.3},{:+.3}) \
                     cc5_bind_dir=({:+.3},{:+.3},{:+.3}) \
                     cc5_local=[{:+.3},{:+.3},{:+.3},{:+.3}]",
                    name,
                    smpl_local_q[0], smpl_local_q[1], smpl_local_q[2], smpl_local_q[3],
                    smpl_dir.x, smpl_dir.y, smpl_dir.z,
                    cc5_dir.x, cc5_dir.y, cc5_dir.z,
                    cc5_local[*i].x, cc5_local[*i].y, cc5_local[*i].z, cc5_local[*i].w,
                );
            }
        }
    }

    // ─── Write rotations to the bones ─────────────────────────────────
    for i in 0..22 {
        let Some(entity) = bind.entities[i] else {
            continue;
        };
        if let Ok(mut t) = transforms.get_mut(entity) {
            t.rotation = cc5_local[i];
        }
    }

    // ─── Root translation: write pelvis (CC_Base_Hip) parent-local pos.
    // delta_local = inv(pelvis_parent_chain_rest) * (pose_root - anchor)
    let delta_world = pose_root - anchor;
    let delta_local = bind.parent_chain_rest[0].inverse() * delta_world;
    if let Some(pelvis_entity) = bind.entities[0] {
        if let Ok(mut t) = transforms.get_mut(pelvis_entity) {
            t.translation = bind.pelvis_rest_translation + delta_local;
        }
    }
}

/// Per-system diagnostic state for `inspect_morph_pipeline`.
#[derive(Default)]
pub(crate) struct InspectMorphDiag {
    pub tick: u64,
    pub asset_dumped: bool,
}

/// Read what is ABOUT to be sent to the GPU and log it. Runs in PostUpdate
/// AFTER Bevy's `inherit_weights` system, so child `MeshMorphWeights`
/// reflect the propagated parent `MorphWeights` from the same frame.
///
/// Three observation points:
///  1. Per-mesh `MeshMorphWeights` max + nonzero count — confirms whether
///     `inherit_weights` actually propagates our parent writes downstream.
///  2. `MorphTargetImage` asset peek — confirms the morph deltas got
///     loaded into the GPU-bound `Image` asset (size, non-zero byte count).
///  3. Parent vs. child weights diff — detects propagation drift /
///     truncation.
///
/// Logs at `tracing::info!` target `cc_render` once per second.
#[allow(clippy::too_many_arguments)]
pub(crate) fn inspect_morph_pipeline(
    parent_q: Query<(Option<&Name>, &MorphWeights, &Children), Without<Mesh3d>>,
    child_q: Query<(
        Option<&Name>,
        &Mesh3d,
        &MeshMorphWeights,
        Option<&ViewVisibility>,
        Option<&InheritedVisibility>,
    )>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    mut diag: Local<InspectMorphDiag>,
) {
    diag.tick += 1;
    let log_now = diag.tick == 1 || diag.tick % 30 == 0; // ~1 Hz at 30 fps

    if !log_now {
        return;
    }

    // (3) parent vs first-child diff per parent
    for (own_name, parent_w, children) in parent_q.iter() {
        let parent_max = parent_w
            .weights()
            .iter()
            .fold(0.0_f32, |a, &b| a.max(b.abs()));
        let parent_nz = parent_w.weights().iter().filter(|&&w| w > 1e-4).count();
        let parent_name = own_name.map(|n| n.as_str()).unwrap_or("?");

        // Top-5 active weights with their morph_target_names so we can
        // see WHICH morphs are getting signal (mouth vs eyes vs brows).
        // Need the morph names — take the first child's mesh asset.
        let mut top_named: Vec<(String, f32)> = Vec::new();
        for &child in children.iter() {
            let Ok((_, mesh3d, _, _, _)) = child_q.get(child) else {
                continue;
            };
            let Some(mesh) = meshes.get(&mesh3d.0) else {
                continue;
            };
            let Some(target_names) = mesh.morph_target_names() else {
                continue;
            };
            let mut indexed: Vec<(usize, f32)> = parent_w
                .weights()
                .iter()
                .enumerate()
                .map(|(i, &w)| (i, w))
                .collect();
            indexed.sort_by(|a, b| {
                b.1.abs()
                    .partial_cmp(&a.1.abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for (i, w) in indexed.into_iter().take(5) {
                if w.abs() < 1e-4 {
                    break;
                }
                let nm = target_names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("idx{}", i));
                top_named.push((nm, w));
            }
            break; // first child is enough — siblings share the same target_names ordering
        }
        let top_str: Vec<String> = top_named
            .iter()
            .map(|(n, w)| format!("{}={:.3}", n, w))
            .collect();

        let mut child_summary = Vec::new();
        let mut sample_handle: Option<Handle<Image>> = None;
        for &child in children.iter() {
            let Ok((cname, mesh3d, child_w, vv, iv)) = child_q.get(child) else {
                continue;
            };
            let child_max = child_w
                .weights()
                .iter()
                .fold(0.0_f32, |a, &b| a.max(b.abs()));
            let child_nz = child_w.weights().iter().filter(|&&w| w > 1e-4).count();
            let drift = (parent_max - child_max).abs();
            // Render-extract gate: extract_morphs requires ViewVisibility(true).
            // Both ViewVisibility AND InheritedVisibility need to be inserted by
            // Bevy's visibility plugin. Missing == not extracted == GPU sees nothing.
            let vv_str = vv
                .map(|v| if v.get() { "VISIBLE" } else { "HIDDEN" })
                .unwrap_or("NO_VV_COMPONENT");
            let iv_str = iv
                .map(|v| if v.get() { "INH_VIS" } else { "INH_HIDDEN" })
                .unwrap_or("NO_IV");
            child_summary.push(format!(
                "child={:?} len={} max={:.3} nz={} drift={:.3} vis=[{}/{}]",
                cname.map(|n| n.as_str()).unwrap_or("?"),
                child_w.weights().len(),
                child_max,
                child_nz,
                drift,
                vv_str,
                iv_str
            ));
            // Capture the morph_targets image handle for asset peek.
            if sample_handle.is_none() {
                if let Some(mesh) = meshes.get(&mesh3d.0) {
                    sample_handle = mesh.morph_targets().cloned();
                }
            }
        }

        if parent_max > 1e-4 || diag.tick == 1 {
            // debug — fires every tick of speech for each parent mesh,
            // each line is multi-KB (top5 + per-child summaries).
            // Same back-pressure rationale as `morph_apply` above. Re-
            // enable for diagnosis with RUST_LOG=cc_render=debug.
            tracing::debug!(
                target: "cc_render",
                "[morph_inspect tick={}] parent={} parent_w(len={} max={:.3} nz={}) top5=[{}] | {}",
                diag.tick, parent_name, parent_w.weights().len(), parent_max, parent_nz,
                top_str.join(", "),
                child_summary.join(" | ")
            );
        }

        // (2) Asset peek — fire on the first few diagnostic ticks where
        // we have a parent. Goes through every branch loudly so we see
        // exactly why we aren't getting the dump (handle missing, asset
        // not loaded, or asset has zero bytes).
        if !diag.asset_dumped {
            match sample_handle.as_ref() {
                None => {
                    tracing::warn!(
                        target: "cc_render",
                        "MORPH_TARGET_IMAGE[{}]: NO HANDLE — mesh.morph_targets() returned None for every child",
                        parent_name
                    );
                }
                Some(handle) => match images.get(handle) {
                    None => {
                        tracing::warn!(
                            target: "cc_render",
                            "MORPH_TARGET_IMAGE[{}]: handle EXISTS but Image asset NOT LOADED in main world (id={:?})",
                            parent_name, handle.id()
                        );
                    }
                    Some(image) => {
                        let bytes = &image.data;
                        let n = bytes.len();
                        let nonzero_bytes = bytes.iter().filter(|&&b| b != 0).count();
                        let max_byte = bytes.iter().copied().max().unwrap_or(0);
                        let n_floats = n / 4;
                        let mut nonzero_floats = 0usize;
                        let mut max_abs_float: f32 = 0.0;
                        for i in 0..n_floats.min(50_000) {
                            let off = i * 4;
                            let f = f32::from_le_bytes([
                                bytes[off],
                                bytes[off + 1],
                                bytes[off + 2],
                                bytes[off + 3],
                            ]);
                            if f != 0.0 {
                                nonzero_floats += 1;
                            }
                            if f.abs() > max_abs_float {
                                max_abs_float = f.abs();
                            }
                        }
                        tracing::info!(
                            target: "cc_render",
                            "MORPH_TARGET_IMAGE[{}]: {} bytes, {} nonzero bytes, max_byte={}, \
                             scanned {} floats, {} nonzero, max_abs_float={:.6}, dims={:?}, format={:?}",
                            parent_name, n, nonzero_bytes, max_byte,
                            n_floats.min(50_000), nonzero_floats, max_abs_float,
                            image.texture_descriptor.size,
                            image.texture_descriptor.format
                        );
                    }
                },
            }
        }
    }

    // After the FIRST eligible tick that actually saw at least one parent,
    // mark asset_dumped so we don't spam the log thereafter.
    if !diag.asset_dumped && parent_q.iter().count() > 0 {
        diag.asset_dumped = true;
    }
}

/// Cached entities + bind-pose rotations for the CC5 face bones we
/// drive directly from ARKit weights. CC5 jaw, eyes, upper jaw, and
/// tongue are *bone-driven* in the rig, not morph-driven — vertex
/// morphs alone only deform skin around the lips/lids; the actual
/// jaw rotation comes from rotating `CC_Base_JawRoot`, tongue
/// articulation from the `Tongue01/02/03` chain, etc. This resource
/// is captured once at scene-load (same gating pattern as
/// `CC5BindRotations`) and consumed every frame by
/// [`apply_arkit_face_bones`].
#[derive(Resource, Default)]
pub(crate) struct FaceBones {
    pub captured: bool,
    pub jaw_root: Option<Entity>,
    pub l_eye: Option<Entity>,
    pub r_eye: Option<Entity>,
    pub upper_jaw: Option<Entity>,
    pub tongue1: Option<Entity>,
    pub tongue2: Option<Entity>,
    pub tongue3: Option<Entity>,
    pub jaw_root_rest: Quat,
    pub jaw_root_rest_t: Vec3,
    pub l_eye_rest: Quat,
    pub r_eye_rest: Quat,
    pub upper_jaw_rest: Quat,
    pub tongue1_rest: Quat,
    pub tongue2_rest: Quat,
    pub tongue3_rest: Quat,
}

/// Walk the named entities once and cache the face-bone handles +
/// their bind-pose local rotations. Idempotent: once `captured` is
/// true the system early-returns.
pub(crate) fn capture_face_bones(
    name_q: Query<(Entity, &Name)>,
    transform_q: Query<&Transform>,
    parent_q: Query<&Parent>,
    avatar_q: Query<Entity, With<AvatarRoot>>,
    mut face: ResMut<FaceBones>,
) {
    if face.captured {
        return;
    }
    // Restrict the search to the avatar SceneRoot subtree so a
    // separately-loaded environment scene with same-named bones
    // (`CC_Base_JawRoot`, etc.) can't shadow the avatar's.
    let avatars: Vec<Entity> = avatar_q.iter().collect();
    // Resolve all face-bone entities by name in a single pass.
    let mut by_name: HashMap<&'static str, Entity> = HashMap::new();
    for (entity, name) in name_q.iter() {
        let key: Option<&'static str> = match name.as_str() {
            "CC_Base_JawRoot" => Some("jaw"),
            "CC_Base_L_Eye" => Some("l_eye"),
            "CC_Base_R_Eye" => Some("r_eye"),
            "CC_Base_UpperJaw" => Some("upper_jaw"),
            "CC_Base_Tongue01" => Some("tongue1"),
            "CC_Base_Tongue02" => Some("tongue2"),
            "CC_Base_Tongue03" => Some("tongue3"),
            _ => None,
        };
        if let Some(k) = key {
            if !is_under_avatar(entity, &parent_q, &avatars) {
                continue;
            }
            by_name.entry(k).or_insert(entity);
        }
    }
    // Jaw is the only required bone — everything else is rig-dependent
    // and we soft-fail (the system just doesn't drive that bone).
    let Some(&jaw) = by_name.get("jaw") else {
        return;
    };
    let Ok(jaw_t) = transform_q.get(jaw) else {
        return;
    };
    face.jaw_root = Some(jaw);
    face.jaw_root_rest = jaw_t.rotation;
    face.jaw_root_rest_t = jaw_t.translation;

    let mut grab = |key: &'static str| -> Option<(Entity, Quat)> {
        let &e = by_name.get(key)?;
        let t = transform_q.get(e).ok()?;
        Some((e, t.rotation))
    };
    if let Some((e, r)) = grab("l_eye") {
        face.l_eye = Some(e);
        face.l_eye_rest = r;
    }
    if let Some((e, r)) = grab("r_eye") {
        face.r_eye = Some(e);
        face.r_eye_rest = r;
    }
    if let Some((e, r)) = grab("upper_jaw") {
        face.upper_jaw = Some(e);
        face.upper_jaw_rest = r;
    }
    if let Some((e, r)) = grab("tongue1") {
        face.tongue1 = Some(e);
        face.tongue1_rest = r;
    }
    if let Some((e, r)) = grab("tongue2") {
        face.tongue2 = Some(e);
        face.tongue2_rest = r;
    }
    if let Some((e, r)) = grab("tongue3") {
        face.tongue3 = Some(e);
        face.tongue3_rest = r;
    }
    face.captured = true;
    tracing::info!(
        target: "cc_render",
        "face_bones: captured jaw={} l_eye={} r_eye={} upper_jaw={} tongue1={} tongue2={} tongue3={}",
        face.jaw_root.is_some(),
        face.l_eye.is_some(),
        face.r_eye.is_some(),
        face.upper_jaw.is_some(),
        face.tongue1.is_some(),
        face.tongue2.is_some(),
        face.tongue3.is_some(),
    );
}

/// Drive the CC5 jaw + eye bones from the latest ARKit weights.
///
/// CC5 ARKit retargeting rules of thumb (these match what RL exports
/// when you use the Live Link / iPhone profile in Character Creator):
/// - `jawOpen` rotates `CC_Base_JawRoot` ~28° around the bone's local
///   X-axis (chin drops). Without this, the open-mouth visemes are
///   visually flat — the V_Open / Mouth_Open morphs deform the lips
///   around a closed jaw, not a hanging one.
/// - `eyeLookUp/Down/In/Out` (left/right pairs) rotate the per-eye
///   bone ~13° max. We compose: `up = +X rotation`, `down = -X`,
///   `in/out = ±Y` (mirrored for L vs R since `eyeLookInLeft` looks
///   right relative to the avatar).
///
/// Applied as `T.rotation = bind_rest * delta` so we don't drift if
/// some other system also writes to these bones.
pub(crate) fn apply_arkit_face_bones(
    pose_rx: Res<PoseWatchRx>,
    face: Res<FaceBones>,
    mut transforms: Query<&mut Transform>,
) {
    if !face.captured {
        return;
    }
    let pose = pose_rx.0.borrow().clone();
    if pose.weights.is_empty() {
        return;
    }
    let w = |k: &str| pose.weights.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);

    // ─── Lower jaw (CC_Base_JawRoot) ─────────────────────────────────
    //
    // ARKit channels driven:
    //   jawOpen  → rotate around head +Z (the hinge axis on this rig,
    //              verified by pinning to 1.0 and observing the chin
    //              drop). Default max = 45° at weight=1.
    //   jawForward → translate the jaw bone forward in head Z by a
    //                small amount (≈8 mm at weight=1; CC5/iClone uses
    //                a Y-translation but on this 90°-Z-bound rig the
    //                forward direction on the bone is parent +Z).
    //   jawLeft / jawRight → small rotation around head +Y (vertical),
    //                ±10° at weight=1.
    //
    // Rotation is composed `delta * bind` (parent-frame), since the
    // bind's 90°-Z rotation scrambles the bone's local axes.
    if let Some(jaw_e) = face.jaw_root {
        if let Ok(mut t) = transforms.get_mut(jaw_e) {
            let pinned: Option<f32> = std::env::var("CC_FACE_JAW_PIN")
                .ok()
                .and_then(|s| s.parse().ok());
            let jaw_open = pinned.unwrap_or_else(|| w("jawOpen"));
            let max_open = std::env::var("CC_FACE_JAW_OPEN_DEG")
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(45.0)
                .to_radians();
            let axis_str = std::env::var("CC_FACE_JAW_AXIS").unwrap_or_else(|_| "z".to_string());
            let sign: f32 = std::env::var("CC_FACE_JAW_SIGN")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.0);
            let open_axis = match axis_str.as_str() {
                "y" => Vec3::Y,
                "x" => Vec3::X,
                _ => Vec3::Z,
            };
            let jaw_lr = w("jawRight") - w("jawLeft");
            let jaw_lr_max = 10.0_f32.to_radians();
            let q_open = Quat::from_axis_angle(open_axis, sign * jaw_open * max_open);
            let q_lr = Quat::from_rotation_y(jaw_lr * jaw_lr_max);
            t.rotation = q_open * q_lr * face.jaw_root_rest;

            let jaw_fwd = w("jawForward");
            let jaw_fwd_amt = std::env::var("CC_FACE_JAW_FORWARD_M")
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(0.008);
            t.translation = face.jaw_root_rest_t + Vec3::Z * (jaw_fwd * jaw_fwd_amt);
        }
    }

    // ─── Upper jaw (CC_Base_UpperJaw) ────────────────────────────────
    //
    // Subtle. The upper-jaw bone barely moves in real anatomy, but
    // CC5 retargeting uses it for `mouthShrugUpper` (lifts the upper
    // lip) — a small rotation around head +Z by a few degrees.
    if let Some(uj_e) = face.upper_jaw {
        if let Ok(mut t) = transforms.get_mut(uj_e) {
            let shrug = w("mouthShrugUpper");
            let max = 5.0_f32.to_radians();
            let delta = Quat::from_rotation_z(-shrug * max);
            t.rotation = delta * face.upper_jaw_rest;
        }
    }

    // ─── Eyes (CC_Base_L_Eye / CC_Base_R_Eye) ────────────────────────
    //
    // Up/down → rotation around head +X. In/out → head +Y, mirrored
    // per side ("in" for the left eye looks toward the avatar's right,
    // i.e. negative head-Y rotation; "in" for the right eye is the
    // opposite). Composed as `delta * bind` to be axis-independent of
    // each eye's individual bind.
    let max_eye = std::env::var("CC_FACE_EYE_DEG")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(13.0)
        .to_radians();
    // Test/debug pins: bypass A2F eye-look weights with hard-coded
    // values so we can verify the bone rotation math even when the
    // running A2F persona has those channels masked out (e.g. Claire,
    // which marks all `eyeLook*` channels inactive). Range -1..1.
    let pin_pitch = std::env::var("CC_FACE_EYE_PIN_PITCH")
        .ok()
        .and_then(|s| s.parse::<f32>().ok());
    let pin_yaw = std::env::var("CC_FACE_EYE_PIN_YAW")
        .ok()
        .and_then(|s| s.parse::<f32>().ok());
    if let Some(le) = face.l_eye {
        if let Ok(mut t) = transforms.get_mut(le) {
            let up_dn = pin_pitch.unwrap_or_else(|| w("eyeLookUpLeft") - w("eyeLookDownLeft"));
            // For the LEFT eye, "look in" is toward the avatar's right
            // (opposite of "look out"). Both eyes share the yaw pin
            // (positive = both eyes look avatar-right).
            let in_out = pin_yaw.unwrap_or_else(|| w("eyeLookInLeft") - w("eyeLookOutLeft"));
            let delta =
                Quat::from_rotation_x(up_dn * max_eye) * Quat::from_rotation_y(in_out * max_eye);
            t.rotation = delta * face.l_eye_rest;
        }
    }
    if let Some(re) = face.r_eye {
        if let Ok(mut t) = transforms.get_mut(re) {
            let up_dn = pin_pitch.unwrap_or_else(|| w("eyeLookUpRight") - w("eyeLookDownRight"));
            // RIGHT eye mirrors: "look in" toward avatar's left, so the
            // yaw pin flips sign.
            let in_out = pin_yaw
                .map(|y| -y)
                .unwrap_or_else(|| w("eyeLookOutRight") - w("eyeLookInRight"));
            let delta =
                Quat::from_rotation_x(up_dn * max_eye) * Quat::from_rotation_y(in_out * max_eye);
            t.rotation = delta * face.r_eye_rest;
        }
    }

    // ─── Tongue chain (CC_Base_Tongue01/02/03) ───────────────────────
    //
    // ARKit `tongueOut` rotates the tongue bones forward + slightly
    // down. We split the rotation across the three bones for a more
    // organic curl: 50% on Tongue01, 35% on Tongue02, 15% on Tongue03
    // (matches CC5/iClone's default tongueOut retarget weights).
    //
    // Hinge axis = head +Z (same as jaw — the tongue extends out of
    // the mouth in the same forward-down arc when jaw is open).
    let tongue_out = w("tongueOut");
    if tongue_out > 0.001 {
        let tongue_max = std::env::var("CC_FACE_TONGUE_DEG")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(35.0)
            .to_radians();
        let apply_tongue =
            |entity: Entity, rest: Quat, transforms: &mut Query<&mut Transform>, share: f32| {
                if let Ok(mut t) = transforms.get_mut(entity) {
                    let delta = Quat::from_rotation_z(tongue_out * tongue_max * share);
                    t.rotation = delta * rest;
                }
            };
        if let Some(e) = face.tongue1 {
            apply_tongue(e, face.tongue1_rest, &mut transforms, 0.50);
        }
        if let Some(e) = face.tongue2 {
            apply_tongue(e, face.tongue2_rest, &mut transforms, 0.35);
        }
        if let Some(e) = face.tongue3 {
            apply_tongue(e, face.tongue3_rest, &mut transforms, 0.15);
        }
    }
}
