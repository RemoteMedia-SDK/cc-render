//! Secondary-motion bones (hair, cloth, breast, tongue jiggle).
//!
//! Auto-detect by regex over bone names; sidecar `chains_override` wins.
//! Breast translation bodies live in world space and follow kinematic anchors
//! with damped springs. Other chains retain their angular pendulum joints.
//! Writeback bounds breast displacement in meters before converting to rig space.

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use regex::Regex;

use super::sidecar::JiggleChainRaw;

/// The anchor follows the animated parent, independently of the driven bone.
/// Translation bodies are unparented: inheriting animation would teleport a
/// dynamic body on every Rapier sync and defeat its inertial response.
#[derive(Component)]
pub(crate) struct JiggleBone {
    pub bone_entity: Entity,
    pub parent_entity: Entity,
    pub translation_limit: f32,
    /// Kept so diagnostics + future per-bone retuning can read the
    /// anchor's authored bind pose without re-querying the hierarchy.
    pub anchor_entity: Entity,
    /// Bone name (cached) for human-readable diagnostic logs. Not used
    /// for runtime logic.
    pub debug_name: String,
    /// Anchor's WORLD rotation captured at spawn (before any animation
    /// has played). Diagnostic only: lets `write_jiggle_bones` show
    /// how much the parent has rotated the anchor in world, so we can
    /// distinguish "parent isn't moving" from "motor too tight".
    pub anchor_world_rest: Quat,
    /// Bone's bind translation in parent's local frame. Used by
    /// translation-mode bones (breast) to write `bind + delta` rather
    /// than overwriting with the dynamic body's local translation.
    pub bone_bind_translation: Vec3,
    /// Rotation chains copy sibling-local rotation; translation chains convert
    /// their bounded world-space offset into the bone parent's local frame.
    pub mode: JiggleMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JiggleMode {
    Rotation,
    Translation,
}

#[derive(Resource, Default)]
pub(crate) struct JiggleSpawned(pub bool);

type BoneEntry = (Entity, String, Transform, Option<Entity>);

fn secondary_candidate(entry: &BoneEntry, patterns: &[Regex], volumetric_tissue: bool)
    -> Option<(Entity, String, Transform, Entity)> {
    let (entity, name, transform, parent) = entry;
    if volumetric_tissue && matches!(name.as_str(), "CC_Base_L_Breast" | "CC_Base_R_Breast") { return None; }
    let parent = (*parent)?;
    patterns.iter().any(|p| p.is_match(name)).then_some((*entity, name.clone(), *transform, parent))
}

/// Walk a list of (entity, bone_name) and return entities whose name
/// matches at least one of the patterns. Pure fn — testable.
pub(crate) fn detect_chains<'a>(
    bones: impl Iterator<Item = (Entity, &'a str)>,
    patterns: &[Regex],
) -> Vec<(Entity, String)> {
    bones
        .filter(|(_, n)| patterns.iter().any(|p| p.is_match(n)))
        .map(|(e, n)| (e, n.to_string()))
        .collect()
}

#[derive(Debug)]
struct BreastTuning {
    mass: f32,
    stiffness: f32,
    damping: f32,
    limit: f32,
}

/// Authored CC5 tuning takes precedence; invalid values use stable defaults.
fn breast_tuning(cc5_physics: Option<&super::cc5_json::Cc5Physics>) -> BreastTuning {
    let cc5_breast_override = cc5_physics
        .and_then(|p| p.breast_tuning.clone())
        .unwrap_or_default();
    let cc5_body_soft = cc5_physics.and_then(|p| p.body_soft_physics().cloned());
    let cc5_breast_radius = cc5_physics.and_then(|p| p.breast_capsule_radius_m());

    // A 4 Hz spring with moderate relative damping settles after each step.
    // The mesh already contains its resting shape; no extra gravitational sag.
    let mass = cc5_breast_override
        .mass
        .or_else(|| cc5_body_soft.as_ref().map(|s| s.mass))
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(2.0);
    let stiffness = cc5_breast_override
        .linear_stiffness
        .or_else(|| {
            cc5_body_soft.as_ref().map(|s| {
                let omega = 2.0 * std::f32::consts::PI * s.stiffness_freq_hz;
                omega * omega * mass
            })
        })
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(1260.0);
    let damping = cc5_breast_override
        .linear_damping
        .or_else(|| cc5_body_soft.as_ref().map(|s| s.damping))
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(30.0);
    let limit = cc5_breast_override
        .translation_limit
        .or_else(|| cc5_breast_radius.map(|r| 0.6 * r))
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(0.03)
        .min(0.05);
    BreastTuning {
        mass,
        stiffness,
        damping,
        limit,
    }
}

pub(crate) fn auto_detect_chains(
    mut commands: Commands,
    bones: Query<
        (Entity, &Name, &Transform, Option<&Parent>),
        (
            Without<super::body_colliders::BodyBoneCollider>,
            Without<JiggleBone>,
        ),
    >,
    parent_globals: Query<&GlobalTransform>,
    sidecar: Option<Res<super::body_colliders::LoadedSidecar>>,
    cc5_physics: Option<Res<super::body_colliders::LoadedCc5Physics>>,
    mut spawned: ResMut<JiggleSpawned>,
    bind: Res<crate::cc_render::bevy_app::pose::CC5BindRotations>,
    tissue: Option<Res<crate::cc_render::bevy_app::tissue::TissueRequest>>,
) {
    if spawned.0 || !bind.captured {
        return;
    }
    spawned.0 = true;

    let raw_patterns: Vec<String> = sidecar
        .as_ref()
        .and_then(|s| s.0.secondary_motion.as_ref())
        .map(|sm| sm.auto_detect_patterns.clone())
        .unwrap_or_else(|| {
            vec![
                r"^CC_Base_Hair_.*".into(),
                r".*Breast.*".into(),
                r"^Skirt_.*".into(),
                r"^CC_Base_Tongue.*".into(),
                // Glutes / buttocks. CC5's standard is `CC_Base_L_GluteMax`
                // / `R_GluteMax` (some rigs add `GluteMed` / `GluteMin`),
                // but `Buttock` shows up in custom rigs. Both patterns are
                // tight enough not to false-match on body bones.
                r".*Glute.*".into(),
                r".*Buttock.*".into(),
            ]
        });
    let patterns_compiled: Vec<Regex> = raw_patterns
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

    // Collect (entity, name, full bind transform, parent_entity).
    // The anchor follows the parent, not the driven bone.
    let entries: Vec<BoneEntry> = bones
        .iter()
        .map(|(e, n, t, p)| (e, n.to_string(), *t, p.map(|x| x.get())))
        .collect();
    let bone_total = entries.len();
    let detected: Vec<(Entity, String, Transform, Entity)> = entries
        .iter()
        .filter_map(|entry| secondary_candidate(entry, &patterns_compiled, tissue.is_some()))
        .collect();

    // CC5 lookups, computed once per call:
    //   - chest_radius_m: peak radius of any authored Spine02 / Hip
    //     capsule, used to scale the pendulum offset proportionally
    //     to the avatar's build (so a child rig and an adult rig
    //     get matching dynamics rather than a hardcoded 5 cm offset).
    //   - hair_soft: CC5's authored hair Mass / Damping / Stiffness
    //     Frequency, mapped onto our motor params for jiggle bones
    //     whose name suggests "hair" (matches the existing
    //     `^CC_Base_Hair_` regex). Realistic per-rig defaults
    //     instead of universal hardcoded numbers.
    let chest_radius_m = cc5_physics.as_ref().and_then(|p| p.0.chest_radius_m());
    let cc5_hair = cc5_physics
        .as_ref()
        .and_then(|p| p.0.hair_soft_physics().cloned());

    let breast = breast_tuning(cc5_physics.as_ref().map(|p| &p.0));
    tracing::info!(target: "cc_render", "physics: breast tuning {breast:?}");
    if let Some(r) = chest_radius_m {
        tracing::info!(
            target: "cc_render",
            "physics: chest radius from CC5 = {:.3} m → pendulum offset will scale to ~{:.3} m",
            r, r * 0.6,
        );
    }
    if let Some(h) = &cc5_hair {
        let omega = (2.0 * std::f32::consts::PI * h.stiffness_freq_hz).powi(2);
        tracing::info!(
            target: "cc_render",
            "physics: hair soft-physics from CC5: mass={:.2}kg damping={:.2} stiffness_freq={:.1}Hz \
             → motor stiffness ω²={:.1}, motor damping={:.2}",
            h.mass, h.damping, h.stiffness_freq_hz, omega, h.damping,
        );
    }

    let detected_count = detected.len();
    for (bone_e, bone_name, bone_bind, parent_e) in &detected {
        let bone_e = *bone_e;
        let parent_e = *parent_e;
        let bone_bind = *bone_bind;
        // Tuning targeting realistic soft-tissue spring dynamics.
        // Real breast / glute tissue measures roughly:
        //   - Natural frequency 1-2 Hz (smaller = faster)
        //   - Damping ratio ζ ≈ 0.3-0.5 (one clear overshoot, small
        //     follow-up, settled in ~1 s)
        //   - Max deflection 3-5 cm ≈ 15-20° at typical bone arm
        //
        // Under rapier's AccelerationBased motor model: ω² = stiffness,
        // and ζ = (motor_damping + body_angular_damping) / (2·ω).
        // Default for non-hair bones: stiffness=40 (ω=1Hz),
        // damping=4 + body damping=1 → ζ ≈ 0.40.
        //
        // Override priority (highest first):
        //   1. Sidecar `chains_override` for this bone
        //   2. CC5 hair Soft Physics, when bone name matches "hair"
        //   3. Hardcoded soft-tissue defaults (40 / 4 / 0.3 / 0.04)
        let lname = bone_name.to_lowercase();
        let is_hair = lname.contains("hair");
        // Breast bones look "chunky" with gravity on: the small
        // pendulum offset (5cm in parent's local -Y) gives a constant
        // m·g·sinθ pull, so the body settles a few degrees off bind
        // and the motor permanently fights it. Disabling gravity on
        // these bodies leaves the motor + anchor acceleration as the
        // only drivers — body sits exactly at bind when still, swings
        // only when the parent (chest) accelerates. Hair / glute /
        // skirt keep gravity on (drape behavior is appropriate there).
        let is_breast = lname.contains("breast");
        let cc5_hair_tuning = cc5_hair.as_ref().filter(|_| is_hair).map(|h| {
            // CC5 stiffness is a frequency (Hz). Convert to motor's
            // ω² form. Mass goes through directly. Damping picked
            // from CC5 Damping; sphere radius left at our default
            // since CC5 doesn't author it for jiggle bones.
            let stiffness = (2.0 * std::f32::consts::PI * h.stiffness_freq_hz).powi(2);
            (stiffness, h.damping, h.mass, 0.04_f32)
        });
        let (stiffness, damping, mass_default, radius) = sidecar
            .as_ref()
            .and_then(|s| s.0.secondary_motion.as_ref())
            .and_then(|sm| {
                sm.chains_override
                    .iter()
                    .find(|c| &c.root_bone == bone_name)
            })
            .map(|c: &JiggleChainRaw| (c.stiffness, c.damping, c.mass_per_link, c.collider_radius))
            .or(cc5_hair_tuning)
            .unwrap_or((40.0, 4.0, 0.3, 0.04));
        // Breast-specific mass override: real soft-tissue mass per side
        // is ~1-2 kg for a large bust; the generic 0.3 kg default
        // (good for hair strands and glute helpers) under-represents
        // the inertia of a large breast — without enough mass the
        // body tracks the anchor's spring force too tightly and
        // the breast looks "bolted on". Sidecar `chains_override`
        // for a specific breast bone still wins.
        let mass = if is_breast
            && sidecar
                .as_ref()
                .and_then(|s| s.0.secondary_motion.as_ref())
                .and_then(|sm| {
                    sm.chains_override
                        .iter()
                        .find(|c| &c.root_bone == bone_name)
                })
                .is_none()
        {
            breast.mass
        } else {
            mass_default
        };

        // Anchors follow the animated parent, never the jiggle-driven bone.
        let kinematic_e = commands
            .spawn((
                Name::new(format!("JiggleAnchor_{}", bone_name)),
                bone_bind,
                RigidBody::KinematicPositionBased,
            ))
            .id();
        commands.entity(parent_e).add_child(kinematic_e);

        // Pendulum offset + angular limits: hang the body 5cm below
        // the joint so gravity creates real torque, but cap the
        // body's rotation to ±LIMIT_RAD per axis so it can't swing
        // past anchor more than that. Without limits, the previous
        // pendulum cut had body equilibrium ruled by gravity, which
        // on bones whose bind isn't world-vertical settled at 170°
        // off — ugly. With limits the body swings within a small
        // envelope around bind: gravity + parent acceleration push
        // it, but it can't fly off.
        //
        // Capture the bone's current world rotation as the "rest"
        // baseline for the diagnostic. After bind capture, the
        // avatar hasn't moved yet so this matches the anchor's
        // initial world pose.
        let anchor_world_rest = parent_globals
            .get(bone_e)
            .map(|gt| gt.compute_transform().rotation)
            .unwrap_or(Quat::IDENTITY);
        let mode = if is_breast {
            JiggleMode::Translation
        } else {
            JiggleMode::Rotation
        };

        let mut body_bind = bone_bind;
        let pendulum_offset = if mode == JiggleMode::Rotation {
            let l = chest_radius_m.map(|r| r * 0.6).unwrap_or(0.05);
            let o = Vec3::new(0.0, -l, 0.0);
            body_bind.translation += o;
            o
        } else {
            // Translation mode: body coincident with anchor.
            Vec3::ZERO
        };

        if mode == JiggleMode::Translation {
            body_bind = parent_globals
                .get(parent_e)
                .expect("detected bone parent has a global transform")
                .mul_transform(body_bind)
                .compute_transform();
        }

        let mut entity_cmds = commands.spawn((
            Name::new(format!("Jiggle_{}", bone_name)),
            JiggleBone {
                bone_entity: bone_e,
                parent_entity: parent_e,
                translation_limit: breast.limit,
                anchor_entity: kinematic_e,
                debug_name: bone_name.clone(),
                anchor_world_rest,
                bone_bind_translation: bone_bind.translation,
                mode,
            },
            body_bind,
            RigidBody::Dynamic,
            Collider::ball(radius),
            CollisionGroups::new(Group::GROUP_3, Group::NONE),
            AdditionalMassProperties::Mass(mass),
            // Breast damping belongs to the joint's relative velocity, so
            // constant-speed walking does not drag the tissue backward.
            Damping {
                linear_damping: if mode == JiggleMode::Translation {
                    0.0
                } else {
                    0.5
                },
                angular_damping: 1.0,
            },
        ));
        if mode == JiggleMode::Rotation {
            entity_cmds.insert(TransformInterpolation::default());
        }
        if mode == JiggleMode::Translation {
            // Colliderless kinematic anchors do not wake sleeping joint bodies.
            entity_cmds.insert((GravityScale(0.0), Velocity::zero(), Sleeping::disabled()));
        }
        let jiggle_e = entity_cmds.id();
        if mode == JiggleMode::Rotation {
            commands.entity(parent_e).add_child(jiggle_e);
        }

        match mode {
            JiggleMode::Rotation => {
                // Per-axis limits on the spherical joint:
                //   X / Z: ±18° forward/back + side swing (the visible
                //                jiggle axes — match real soft-tissue
                //                3-5 cm deflection at typical bone arm).
                //   Y    : ±2°  twist around bone's length axis. Almost
                //                locked — produces ugly mesh spin
                //                when the parent yaws, no bounce
                //                benefit. Bone-frame Y is the "length"
                //                axis in CC5/SMPL rigs.
                const BEND_RAD: f32 = 0.314;
                const TWIST_RAD: f32 = 0.035;
                let twist_stiffness = stiffness * 4.0;
                let twist_damping = damping * 4.0;
                let joint = SphericalJointBuilder::new()
                    .local_anchor1(Vec3::ZERO)
                    .local_anchor2(-pendulum_offset)
                    .limits(JointAxis::AngX, [-BEND_RAD, BEND_RAD])
                    .limits(JointAxis::AngY, [-TWIST_RAD, TWIST_RAD])
                    .limits(JointAxis::AngZ, [-BEND_RAD, BEND_RAD])
                    .motor_position(JointAxis::AngX, 0.0, stiffness, damping)
                    .motor_position(JointAxis::AngY, 0.0, twist_stiffness, twist_damping)
                    .motor_position(JointAxis::AngZ, 0.0, stiffness, damping);
                commands
                    .entity(jiggle_e)
                    .insert(ImpulseJoint::new(kinematic_e, joint));
            }
            JiggleMode::Translation => {
                let translation_limit = breast.limit;
                let lin_stiffness = breast.stiffness;
                // Legacy damping is a rate (1/s). Convert it to a force
                // coefficient and enforce enough damping to avoid ringing.
                let critical = 2.0 * (lin_stiffness * mass).sqrt();
                let lin_damping = (breast.damping * mass).max(0.55 * critical);
                let joint = GenericJointBuilder::new(JointAxesMask::ANG_AXES)
                    .local_anchor1(Vec3::ZERO)
                    .local_anchor2(Vec3::ZERO)
                    .limits(JointAxis::LinX, [-translation_limit, translation_limit])
                    .limits(JointAxis::LinY, [-translation_limit, translation_limit])
                    .limits(JointAxis::LinZ, [-translation_limit, translation_limit])
                    .motor_model(JointAxis::LinX, MotorModel::ForceBased)
                    .motor_model(JointAxis::LinY, MotorModel::ForceBased)
                    .motor_model(JointAxis::LinZ, MotorModel::ForceBased)
                    .motor_position(JointAxis::LinX, 0.0, lin_stiffness, lin_damping)
                    .motor_position(JointAxis::LinY, 0.0, lin_stiffness, lin_damping)
                    .motor_position(JointAxis::LinZ, 0.0, lin_stiffness, lin_damping)
                    .build();
                // bevy_rapier 0.28: GenericJoint has no Into<TypedJoint>
                // impl, so wrap manually.
                let typed = TypedJoint::GenericJoint(joint);
                commands
                    .entity(jiggle_e)
                    .insert(ImpulseJoint::new(kinematic_e, typed));
            }
        }
    }
    if detected_count > 0 {
        // Sample up to 6 matched names so the user can confirm the
        // patterns hit what they expected. Truncate to keep the log
        // line readable on dense rigs (CC5 hair can be 40+ bones).
        let sample: Vec<&str> = detected
            .iter()
            .take(6)
            .map(|(_, n, _, _)| n.as_str())
            .collect();
        tracing::info!(
            target: "cc_render",
            "physics: spawned {} jiggle bodies for secondary motion \
             (out of {} bind-pose bones; sample matches: {:?})",
            detected_count, bone_total, sample,
        );
    } else {
        // Loud warning: bind capture fired AND we ran detection AND
        // matched zero bones. Most likely cause is a non-CC5 rig
        // (Kimodo / SMPL / custom) where bone names don't match the
        // built-in patterns. Show a sample of bone names + the
        // patterns we tried so the user knows what to override via
        // the sidecar's `secondary_motion.auto_detect_patterns`.
        let pattern_strs: Vec<&str> = raw_patterns.iter().map(|s| s.as_str()).collect();
        let bone_sample: Vec<&str> = entries
            .iter()
            .take(12)
            .map(|(_, n, _, _)| n.as_str())
            .collect();
        tracing::warn!(
            target: "cc_render",
            "physics: zero secondary-motion bones matched on this avatar \
             ({} bind-pose bones scanned). \
             Patterns tried: {:?}. Sample bone names: {:?}. \
             Override patterns via the physics sidecar's \
             `secondary_motion.auto_detect_patterns` field.",
            bone_total, pattern_strs, bone_sample,
        );
    }
}

pub(crate) fn write_jiggle_bones(
    jiggle_bodies: Query<(&JiggleBone, &Transform)>,
    bone_globals: Query<&GlobalTransform>,
    mut transforms: Query<&mut Transform, Without<JiggleBone>>,
    mut throttle: Local<u32>,
) {
    // Two-phase: snapshot first, then apply, so the &mut transform
    // borrow can't conflict with the read-only queries.
    struct Snap {
        bone_e: Entity,
        body_rot: Quat,
        delta_local: Vec3,
        displacement_m: f32,
        raw_displacement_m: f32,
        anchor_rot: Quat,
        anchor_world_rot: Quat,
        anchor_world_rest: Quat,
        bone_bind_t: Vec3,
        mode: JiggleMode,
        name: String,
    }
    let snapshots: Vec<Snap> = jiggle_bodies
        .iter()
        .filter_map(|(jb, t_jiggle)| {
            let anchor = *transforms.get(jb.anchor_entity).ok()?;
            let anchor_world_rot = bone_globals
                .get(jb.anchor_entity)
                .ok()
                .map(|gt| gt.compute_transform().rotation)
                .unwrap_or(Quat::IDENTITY);
            let parent = bone_globals.get(jb.parent_entity).ok()?;
            let anchor_world = parent.transform_point(anchor.translation);
            let delta_world =
                bounded_displacement(t_jiggle.translation - anchor_world, jb.translation_limit);
            Some(Snap {
                bone_e: jb.bone_entity,
                body_rot: t_jiggle.rotation,
                delta_local: parent.affine().inverse().transform_vector3(delta_world),
                displacement_m: delta_world.length(),
                raw_displacement_m: (t_jiggle.translation - anchor_world).length(),
                anchor_rot: anchor.rotation,
                anchor_world_rot,
                anchor_world_rest: jb.anchor_world_rest,
                bone_bind_t: jb.bone_bind_translation,
                mode: jb.mode,
                name: jb.debug_name.clone(),
            })
        })
        .collect();

    let mut max_lag_deg = 0.0_f32;
    let mut max_lag_name = String::new();
    let mut max_translation_cm = 0.0_f32;
    let mut max_translation_name = String::new();
    let mut max_anchor_drive_deg = 0.0_f32;
    let mut applied = 0_usize;
    for snap in snapshots {
        if let Ok(mut bone_t) = transforms.get_mut(snap.bone_e) {
            match snap.mode {
                JiggleMode::Rotation => {
                    bone_t.rotation = snap.body_rot;
                    let lag = angle_between_deg(snap.body_rot, snap.anchor_rot);
                    if lag > max_lag_deg {
                        max_lag_deg = lag;
                        max_lag_name = snap.name.clone();
                    }
                }
                JiggleMode::Translation => {
                    bone_t.translation = snap.bone_bind_t + snap.delta_local;
                    bone_t.rotation = snap.anchor_rot;
                    let mag_cm = snap.displacement_m * 100.0;
                    if mag_cm > max_translation_cm {
                        max_translation_cm = mag_cm;
                        max_translation_name = snap.name.clone();
                    }
                    if *throttle % 6 == 0 {
                        tracing::debug!(target: "cc_render",
                            "jiggle_writeback: {} delta_cm={:.3} raw_cm={:.3}", snap.name, mag_cm, snap.raw_displacement_m * 100.0);
                    }
                }
            }
            applied += 1;

            let drive = angle_between_deg(snap.anchor_world_rot, snap.anchor_world_rest);
            if drive > max_anchor_drive_deg {
                max_anchor_drive_deg = drive;
            }
        }
    }
    *throttle = throttle.wrapping_add(1);
    if applied > 0 && *throttle % 30 == 0 {
        tracing::debug!(
            target: "cc_render",
            "jiggle: {} bones | max rot-lag = {:.2}° ({}) | \
             max trans-displacement = {:.2}cm ({}) | \
             max anchor world drive = {:.2}°",
            applied, max_lag_deg, max_lag_name,
            max_translation_cm, max_translation_name,
            max_anchor_drive_deg,
        );
    }
}

/// Solver limits can briefly overshoot on discontinuities. Bound the visible
/// *radial* displacement (not a cube), in world meters, including invalid input.
fn bounded_displacement(delta: Vec3, limit: f32) -> Vec3 {
    if !delta.is_finite() {
        return Vec3::ZERO;
    }
    delta.clamp_length_max(limit)
}

/// Shortest-arc angle (degrees) between two unit quaternions, in
/// [0, 180]. Uses the |dot| trick to fold the q ≡ −q double-cover so
/// near-identity rotations report ~0° rather than ~360°.
fn angle_between_deg(a: Quat, b: Quat) -> f32 {
    let dot = a.dot(b).abs().clamp(0.0, 1.0);
    (2.0 * dot.acos()).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hair_breast_skirt_tongue_glute() {
        let patterns: Vec<Regex> = [
            r"^CC_Base_Hair_.*",
            r".*Breast.*",
            r"^Skirt_.*",
            r"^CC_Base_Tongue.*",
            r".*Glute.*",
            r".*Buttock.*",
        ]
        .iter()
        .map(|p| Regex::new(p).unwrap())
        .collect();

        let bones: Vec<(Entity, &str)> = vec![
            (Entity::from_raw(1), "CC_Base_Hair_Front"),
            (Entity::from_raw(2), "CC_Base_Spine01"),
            (Entity::from_raw(3), "BS_LeftBreast"),
            (Entity::from_raw(4), "Skirt_F_01"),
            (Entity::from_raw(5), "CC_Base_TongueTip"),
            (Entity::from_raw(6), "CC_Base_L_Hand"),
            (Entity::from_raw(7), "CC_Base_L_GluteMax"),
            (Entity::from_raw(8), "CC_Base_R_GluteMed"),
            (Entity::from_raw(9), "CC_Base_R_Buttock"),
            // False-positive guard: should NOT match (no Glute/Buttock substring)
            (Entity::from_raw(10), "CC_Base_L_Thigh"),
        ];

        let detected = detect_chains(bones.into_iter(), &patterns);
        let names: Vec<&str> = detected.iter().map(|(_, n)| n.as_str()).collect();
        assert!(names.contains(&"CC_Base_Hair_Front"));
        assert!(names.contains(&"BS_LeftBreast"));
        assert!(names.contains(&"Skirt_F_01"));
        assert!(names.contains(&"CC_Base_TongueTip"));
        assert!(names.contains(&"CC_Base_L_GluteMax"));
        assert!(names.contains(&"CC_Base_R_GluteMed"));
        assert!(names.contains(&"CC_Base_R_Buttock"));
        assert!(!names.contains(&"CC_Base_Spine01"));
        assert!(!names.contains(&"CC_Base_L_Hand"));
        assert!(!names.contains(&"CC_Base_L_Thigh"));
    }

    #[test]
    fn empty_patterns_match_nothing() {
        let bones: Vec<(Entity, &str)> = vec![(Entity::from_raw(1), "CC_Base_Hair_Front")];
        let detected = detect_chains(bones.into_iter(), &[]);
        assert!(detected.is_empty());
    }

    #[test]
    fn angle_between_handles_double_cover() {
        // q ≡ −q double-cover trap: a near-identity rotation expressed
        // as q with q.w slightly negative was getting reported as ~360°
        // by the previous diagnostic. The |dot|-based form folds both
        // halves of the cover so it reports ~0° in either case.
        let q = Quat::from_rotation_y(0.001);
        let neg_q = Quat::from_xyzw(-q.x, -q.y, -q.z, -q.w);
        let identity = Quat::IDENTITY;
        let d_pos = angle_between_deg(q, identity);
        let d_neg = angle_between_deg(neg_q, identity);
        assert!(d_pos < 0.5, "near-identity should be ~0°, got {}", d_pos);
        assert!(
            d_neg < 0.5,
            "double-cover near-identity should be ~0°, got {}",
            d_neg
        );
        // 90° rotation should stay ~90°, not drift across the wrap.
        let q90 = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let d90 = angle_between_deg(q90, identity);
        assert!(
            (d90 - 90.0).abs() < 0.5,
            "90° rotation should report 90°, got {}",
            d90
        );
    }

    struct Simulation {
        app: App,
        parent: Entity,
        bone: Entity,
    }

    impl Simulation {
        fn new(fps: f32) -> Self {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin));
            app.init_resource::<Assets<Mesh>>();
            app.init_resource::<SceneSpawner>();
            app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default());
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(1.0 / fps),
            ));
            app.insert_resource(TimestepMode::Interpolated {
                dt: 1.0 / 120.0,
                time_scale: 1.0,
                substeps: 4,
            });
            app.init_resource::<JiggleSpawned>();
            app.init_resource::<crate::cc_render::bevy_app::pose::CC5BindRotations>();
            app.add_systems(Update, auto_detect_chains);
            app.add_systems(
                PostUpdate,
                write_jiggle_bones
                    .after(PhysicsSet::Writeback)
                    .before(bevy::transform::TransformSystem::TransformPropagate),
            );
            let parent = app
                .world_mut()
                .spawn(Transform::from_xyz(0.0, 1.4, 0.0))
                .id();
            let bone = app
                .world_mut()
                .spawn((
                    Name::new("CC_Base_L_Breast"),
                    Transform::from_xyz(0.12, 0.0, 0.12),
                ))
                .id();
            app.world_mut().entity_mut(parent).add_child(bone);
            app.update();
            app.world_mut()
                .resource_mut::<crate::cc_render::bevy_app::pose::CC5BindRotations>()
                .captured = true;
            app.update();
            Self { app, parent, bone }
        }

        fn step(&mut self, translation: Vec3, rotation: Quat, scale: Vec3) -> Vec3 {
            *self
                .app
                .world_mut()
                .get_mut::<Transform>(self.parent)
                .unwrap() = Transform {
                translation,
                rotation,
                scale,
            };
            self.app.update();
            let local = self
                .app
                .world()
                .get::<Transform>(self.bone)
                .unwrap()
                .translation;
            rotation * (scale * (local - Vec3::new(0.12, 0.0, 0.12)))
        }
    }

    #[test]
    fn breast_spring_moves_and_settles_at_multiple_frame_rates() {
        for fps in [15.0, 30.0, 60.0, 120.0] {
            let mut sim = Simulation::new(fps);
            let origin = Vec3::new(0.0, 1.4, 0.0);
            for _ in 0..30 {
                assert!(sim.step(origin, Quat::IDENTITY, Vec3::ONE).length() < 0.001);
            }
            let mut peak = 0.0_f32;
            for frame in 0..(fps as usize * 4) {
                let t = frame as f32 / fps;
                let position = origin + Vec3::new(t * 0.5, 0.015 * (t * 12.0).sin(), 0.0);
                let delta = sim.step(position, Quat::from_rotation_y(t), Vec3::ONE);
                assert!(delta.is_finite() && delta.length() <= 0.03001);
                peak = peak.max(delta.length());
            }
            assert!(
                peak > 0.002,
                "secondary motion must remain visible: {fps}fps peak={peak}"
            );
            let stop = sim
                .app
                .world()
                .get::<Transform>(sim.parent)
                .copied()
                .unwrap();
            let mut final_delta = Vec3::ZERO;
            for _ in 0..(fps as usize * 3) {
                final_delta = sim.step(stop.translation, stop.rotation, stop.scale);
            }
            assert!(
                final_delta.length() < 0.001,
                "must settle: {fps}fps {final_delta}"
            );
            let world = sim.app.world_mut();
            let mut bodies = world.query_filtered::<Option<&Parent>, With<JiggleBone>>();
            assert!(
                bodies.iter(world).all(|parent| parent.is_none()),
                "spring mass must not inherit animation"
            );
        }
    }

    #[test]
    fn breast_discontinuities_stay_bounded_in_world_meters() {
        let mut sim = Simulation::new(30.0);
        for frame in 0..120 {
            let rotation = Quat::from_rotation_z(frame as f32 * 0.9);
            let position = Vec3::new(if frame % 2 == 0 { 3.0 } else { -3.0 }, 1.0, 2.0);
            let delta = sim.step(position, rotation, Vec3::new(0.5, 2.0, 1.5));
            assert!(delta.is_finite() && delta.length() <= 0.03001, "{delta}");
        }
        sim.app
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(800),
            ));
        assert!(sim.step(Vec3::Y, Quat::IDENTITY, Vec3::ONE).length() <= 0.03001);
        sim.app
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(1.0 / 30.0),
            ));
        let mut settled = Vec3::ZERO;
        for _ in 0..90 {
            settled = sim.step(Vec3::Y, Quat::IDENTITY, Vec3::ONE);
        }
        assert!(
            settled.length() < 0.001,
            "must recover after teleports and stalls: {settled}"
        );
        assert_eq!(
            bounded_displacement(Vec3::splat(f32::NAN), 0.03),
            Vec3::ZERO
        );
    }
}
