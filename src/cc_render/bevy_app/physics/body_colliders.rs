//! Per-SMPL-22 bone kinematic colliders. Sized from `CC5BindRotations`
//! at avatar-load time; their transforms are copied from the bone's
//! `GlobalTransform` each frame in `PostUpdate` after pose application.

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use super::cc5_json::Cc5Shape;
use super::sidecar::{BodyColliderOverride, SidecarRaw};
use crate::cc_render::bevy_app::pose::{CC5BindRotations, SMPL22_TO_CC5};

/// Centimeter-to-meter conversion for CC5 native physics JSON. CC5
/// authors all coordinates in cm at world bind pose.
const CC5_CM_TO_M: f32 = 0.01;

/// Marks the kinematic body for the given SMPL-22 bone index.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct BodyBoneCollider {
    /// SMPL-22 bone index (0=Pelvis ... 20=L_Wrist, 21=R_Wrist).
    /// Read by the grab FSM to find wrist entities (smpl_idx == 20 = L_Wrist, 21 = R_Wrist).
    pub smpl_idx: usize,
}

/// Marker so `auto_fit_capsules` only runs once per session.
#[derive(Resource, Default)]
pub(crate) struct BodyCollidersSpawned(pub bool);

/// Compute capsule (radius, half_height, center_world) for a bone given
/// its world-space joint position and tip position. Radius is a fixed
/// fraction of bone length, clamped to a sane min/max.
pub(crate) fn capsule_from_bone(
    joint_world: Vec3,
    tip_world: Vec3,
    radius_ratio: f32,
    radius_min: f32,
    radius_max: f32,
) -> (f32, f32, Vec3) {
    let dir = tip_world - joint_world;
    let len = dir.length();
    let half_h = (len * 0.5).max(0.01);
    let r = (len * radius_ratio).clamp(radius_min, radius_max);
    let center = joint_world + dir * 0.5;
    (r, half_h, center)
}

/// One-shot system: when `CC5BindRotations` becomes populated and we
/// haven't spawned yet, build per-bone kinematic bodies. Sidecar
/// `body_colliders_override` wins (Some replaces, None disables).
///
/// Joint world position comes from the bone entity's `GlobalTransform`
/// (via `bind.entities[i]`). Tip world position uses the SMPL primary
/// child's `GlobalTransform` when present; for leaves, falls back to
/// `joint + cc5_bone_axis_world * 0.05` (5cm leaf-collider stub).
pub(crate) fn auto_fit_capsules(
    mut commands: Commands,
    bind: Res<CC5BindRotations>,
    sidecar: Option<Res<LoadedSidecar>>,
    cc5_physics: Option<Res<LoadedCc5Physics>>,
    mut spawned: ResMut<BodyCollidersSpawned>,
    transforms: Query<&GlobalTransform>,
) {
    use crate::cc_render::bevy_app::pose::SMPL_PRIMARY_CHILD;
    if spawned.0 || !bind.captured {
        return;
    }
    spawned.0 = true;

    let overrides = sidecar
        .as_ref()
        .map(|s| s.0.body_colliders_override.clone())
        .unwrap_or_default();

    for (smpl_idx, (smpl_name, cc5_name)) in SMPL22_TO_CC5.iter().enumerate() {
        // Bone entity comes from CC5BindRotations.entities.
        let Some(bone_entity) = bind.entities[smpl_idx] else {
            continue;
        };
        let Ok(bone_gt) = transforms.get(bone_entity) else {
            continue;
        };
        let joint = bone_gt.translation();

        // Tip = primary child's GlobalTransform, or fallback for leaves.
        let tip = match SMPL_PRIMARY_CHILD[smpl_idx] {
            Some(child_idx) => bind.entities[child_idx]
                .and_then(|e| transforms.get(e).ok())
                .map(|gt| gt.translation())
                .unwrap_or_else(|| joint + bind.cc5_bone_axis_world[smpl_idx] * 0.05),
            None => joint + bind.cc5_bone_axis_world[smpl_idx] * 0.05,
        };

        // Priority chain (highest -> lowest):
        //   1. sidecar `body_colliders_override` (Some(Some) replaces, Some(None) disables)
        //   2. CC5 native physics JSON (artist-authored multi-shape rig)
        //   3. auto-fit single capsule from bind data
        let user = overrides.get(*smpl_name);
        let (collider, friction_coef) = match user {
            Some(None) => continue, // sidecar explicitly disables this bone
            Some(Some(BodyColliderOverride::Capsule {
                radius,
                half_height,
            })) => (Collider::capsule_y(*half_height, *radius), 0.4),
            Some(Some(BodyColliderOverride::Sphere { radius })) => (Collider::ball(*radius), 0.4),
            None => {
                // Try CC5 native JSON first.
                if let Some(cc5_built) = cc5_physics
                    .as_ref()
                    .and_then(|c| c.0.by_bone.get(*cc5_name))
                    .and_then(|shapes| build_cc5_compound(shapes, bone_gt, smpl_name, cc5_name))
                {
                    cc5_built
                } else {
                    let (r, h, _center) = capsule_from_bone(joint, tip, 0.18, 0.02, 0.12);
                    (Collider::capsule_y(h, r), 0.4)
                }
            }
        };

        // Spawn child kinematic body parented to the bone so its
        // GlobalTransform tracks the bone automatically. Bevy 0.15:
        // Transform alone (no TransformBundle).
        let child = commands
            .spawn((
                Name::new(format!("BodyCollider_{}", smpl_name)),
                BodyBoneCollider { smpl_idx },
                Transform::default(),
                RigidBody::KinematicPositionBased,
                collider,
                Friction::coefficient(friction_coef),
            ))
            .id();
        commands.entity(bone_entity).add_child(child);
    }
    tracing::info!(
        target: "cc_render",
        "physics: spawned body colliders for SMPL-22 bones"
    );
}

/// Build a `Collider::compound` from CC5 native physics JSON shapes,
/// converting world-at-bind cm to bone-local meters via the inverse of
/// the bone's bind world transform. Returns `None` when no valid (bone-
/// active, supported) shapes remain.
///
/// Per-shape `Friction`/`Elasticity` from CC5 are aggregated to a single
/// per-bone Friction value (returned alongside the collider): we use the
/// FIRST active shape's friction. This is a deliberate simplification:
/// `Friction` in bevy_rapier3d is a single component per parent entity,
/// while compound colliders report contacts per sub-shape. Averaging
/// would silently misrepresent stiffer vs softer sub-shapes; instead we
/// take the lead shape's value (typically the largest / first-authored
/// shape per bone).
fn build_cc5_compound(
    shapes: &[Cc5Shape],
    bone_gt: &GlobalTransform,
    smpl_name: &str,
    cc5_name: &str,
) -> Option<(Collider, f32)> {
    if shapes.is_empty() {
        return None;
    }
    // Bind world -> bone local: invert the bone's bind world transform.
    let bone_inv = bone_gt.compute_matrix().inverse();
    let mut sub_shapes: Vec<(Vect, Rot, Collider)> = Vec::with_capacity(shapes.len());
    let mut friction_coef: f32 = 0.4;
    let mut friction_set = false;

    for shape in shapes {
        match shape {
            Cc5Shape::Capsule {
                bone_active,
                world_translate,
                world_rotation_q,
                radius,
                capsule_length,
                friction,
                ..
            } => {
                if !*bone_active {
                    continue;
                }
                let world_t = Vec3::new(
                    world_translate[0] * CC5_CM_TO_M,
                    world_translate[1] * CC5_CM_TO_M,
                    world_translate[2] * CC5_CM_TO_M,
                );
                let world_r = Quat::from_xyzw(
                    world_rotation_q[0],
                    world_rotation_q[1],
                    world_rotation_q[2],
                    world_rotation_q[3],
                )
                .normalize();
                let world_mat = Mat4::from_rotation_translation(world_r, world_t);
                let local_mat = bone_inv * world_mat;
                let (_scale, local_rot, local_t) = local_mat.to_scale_rotation_translation();
                // CC5 "Capsule Length" is the cylinder length BETWEEN
                // hemisphere centers. capsule_y wants half-height (also
                // between centers) plus radius — both in meters.
                let half_h = (*capsule_length * CC5_CM_TO_M) * 0.5;
                let r = *radius * CC5_CM_TO_M;
                sub_shapes.push((local_t, local_rot, Collider::capsule_y(half_h, r)));
                if !friction_set {
                    friction_coef = *friction;
                    friction_set = true;
                }
            }
            Cc5Shape::Box {
                bone_active,
                world_translate,
                world_rotation_q,
                extents,
                friction,
                ..
            } => {
                if !*bone_active {
                    continue;
                }
                let world_t = Vec3::new(
                    world_translate[0] * CC5_CM_TO_M,
                    world_translate[1] * CC5_CM_TO_M,
                    world_translate[2] * CC5_CM_TO_M,
                );
                let world_r = Quat::from_xyzw(
                    world_rotation_q[0],
                    world_rotation_q[1],
                    world_rotation_q[2],
                    world_rotation_q[3],
                )
                .normalize();
                let world_mat = Mat4::from_rotation_translation(world_r, world_t);
                let local_mat = bone_inv * world_mat;
                let (_scale, local_rot, local_t) = local_mat.to_scale_rotation_translation();
                let collider = match extents {
                    Some(e) => {
                        // Extents are full size in cm; cuboid wants
                        // half-extents in m -> multiply by 0.005.
                        Collider::cuboid(e[0] * 0.005, e[1] * 0.005, e[2] * 0.005)
                    }
                    None => {
                        tracing::warn!(
                            target: "cc_render",
                            "physics: CC5 Box for bone {} ({}) has no Extents; \
                             falling back to 5cm sphere placeholder",
                            smpl_name, cc5_name,
                        );
                        Collider::ball(0.05)
                    }
                };
                sub_shapes.push((local_t, local_rot, collider));
                if !friction_set {
                    friction_coef = *friction;
                    friction_set = true;
                }
            }
        }
    }

    if sub_shapes.is_empty() {
        return None;
    }
    tracing::debug!(
        target: "cc_render",
        "physics: built CC5 compound for {} ({}): {} sub-shapes, friction={:.3}",
        smpl_name, cc5_name, sub_shapes.len(), friction_coef,
    );
    Some((Collider::compound(sub_shapes), friction_coef))
}

// Note: an earlier draft included a `sync_kinematic` PostUpdate hook
// here as a placeholder for axis-aligned capsule re-orient. It was a
// no-op (parented children inherit the bone's GlobalTransform via Bevy)
// and contributed only scheduler overhead + a misleading parents/bodies
// query. Removed — re-add only when there's an actual re-orient need.

#[derive(Resource, Clone, Debug)]
pub(crate) struct LoadedSidecar(pub SidecarRaw);

#[derive(Resource, Clone, Debug)]
pub(crate) struct LoadedCc5Physics(pub super::cc5_json::Cc5Physics);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_fit_for_known_bone() {
        let joint = Vec3::new(0.0, 1.0, 0.0);
        let tip = Vec3::new(0.0, 1.4, 0.0); // 40cm bone
        let (r, h, c) = capsule_from_bone(joint, tip, 0.18, 0.02, 0.12);
        assert!((h - 0.20).abs() < 1e-5, "half_height = {}", h);
        assert!((r - (0.40 * 0.18)).abs() < 1e-5, "radius = {}", r);
        assert_eq!(c, Vec3::new(0.0, 1.2, 0.0));
    }

    #[test]
    fn capsule_radius_clamped_to_min() {
        let joint = Vec3::ZERO;
        let tip = Vec3::new(0.0, 0.05, 0.0); // 5cm bone, tiny
        let (r, _, _) = capsule_from_bone(joint, tip, 0.18, 0.02, 0.12);
        assert!(
            (r - 0.02).abs() < 1e-5,
            "radius should clamp to min, got {}",
            r
        );
    }

    #[test]
    fn capsule_radius_clamped_to_max() {
        let joint = Vec3::ZERO;
        let tip = Vec3::new(0.0, 2.0, 0.0); // 2m bone, huge
        let (r, _, _) = capsule_from_bone(joint, tip, 0.18, 0.02, 0.12);
        assert!(
            (r - 0.12).abs() < 1e-5,
            "radius should clamp to max, got {}",
            r
        );
    }
}
