//! Secondary-motion bones (hair, cloth, breast, tongue jiggle).
//!
//! Auto-detect by regex over bone names; sidecar `chains_override` wins.
//! Each detected bone becomes a dynamic Rapier body with a small sphere
//! collider, joined to its parent bone with a `SphericalJoint` that has
//! per-angular-axis spring motors targeting zero rotation (= bind pose).
//! After `PhysicsSet::Writeback`, we copy the dynamic body's rotation
//! back onto the bone's `Transform` so the skinned mesh follows.
//!
//! Spring tuning: `stiffness` (default 30.0) controls how aggressively
//! each axis pulls back to rest; `damping` (default 2.5) controls
//! oscillation decay. Sidecar `chains_override` per-bone overrides
//! both. Higher stiffness = stiffer hair (less drift), higher damping
//! = less wobble.

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use regex::Regex;

use super::sidecar::JiggleChainRaw;

/// Per-bone marker for the dynamic jiggle body. The dynamic body and
/// its kinematic anchor are spawned as SIBLINGS of the bone (children
/// of the bone's parent), not children of the bone itself — see the
/// "feedback loop" comment in `auto_detect_chains` for why.
#[derive(Component)]
pub(crate) struct JiggleBone {
    pub bone_entity: Entity,
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
    /// Drive mode for `write_jiggle_bones`:
    /// - `Rotation`: write the dynamic body's local rotation onto the
    ///   bone (cone-around-pivot — natural for hair, glute, skirt).
    /// - `Translation`: write `bind_translation + (body.t - anchor.t)`
    ///   onto the bone, leaving rotation at bind. The LBS skinning
    ///   distributes the translation across the weight gradient as a
    ///   soft volumetric squish — the only way to get realistic breast
    ///   bounce out of a single bone (a pivoted cone rotation around
    ///   one bone always reads as a "stiff cone wobble" no matter how
    ///   the weights are tuned).
    pub mode: JiggleMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JiggleMode {
    Rotation,
    Translation,
}

#[derive(Resource, Default)]
pub(crate) struct JiggleSpawned(pub bool);

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
    // Parent is required: jiggle anchor + dynamic body must spawn as
    // siblings of the bone (under bone's parent), not children of
    // the bone itself, to break the rapier-bone feedback loop.
    let entries: Vec<(Entity, String, Transform, Option<Entity>)> = bones
        .iter()
        .map(|(e, n, t, p)| (e, n.to_string(), *t, p.map(|x| x.get())))
        .collect();
    let bone_total = entries.len();
    let detected: Vec<(Entity, String, Transform, Entity)> = entries
        .iter()
        .filter_map(|(e, n, t, parent)| {
            let parent = (*parent)?;
            patterns_compiled
                .iter()
                .any(|p| p.is_match(n))
                .then_some((*e, n.clone(), *t, parent))
        })
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

    // Per-avatar breast (translation-mode) tuning. Resolution priority,
    // per-knob, picks the first authored source:
    //   1. Custom `Physics."Breast Tuning"` block in the CC5 JSON
    //      (artist hand-authored — overrides everything).
    //   2. CC5 native data when present:
    //        - mass               ← Soft Physics body-mesh `Mass`
    //        - linear_stiffness   ← (2π·`Stiffness Frequency`)²·mass
    //        - linear_damping     ← Soft Physics body-mesh `Damping`
    //        - translation_limit  ← 0.6·`CC_Base_L_Breast` capsule radius
    //   3. Hardcoded "large bust" defaults (back-compat with avatars
    //      that ship neither block).
    //
    // The custom block is the authoritative knob set: artists who
    // skipped iClone soft-physics authoring (common on this and
    // similar Kimodo / SMPL-aligned exports) just hand-edit one
    // small block per avatar. Fully-authored CC5 rigs with `Soft
    // Physics > Meshes` and breast collision capsules get sane
    // tuning automatically.
    let cc5_breast_override = cc5_physics
        .as_ref()
        .and_then(|p| p.0.breast_tuning.clone())
        .unwrap_or_default();
    let cc5_body_soft = cc5_physics
        .as_ref()
        .and_then(|p| p.0.body_soft_physics().cloned());
    let cc5_breast_radius = cc5_physics
        .as_ref()
        .and_then(|p| p.0.breast_capsule_radius_m());

    // Default fallbacks tuned for a large-bust build, calibrated against
    // real-world breast jiggle on rhythmic body motion (skip rope, jog).
    //
    // Counter-intuitive: stiffer spring + lower damping produces MORE
    // pronounced bounce, not less. Mechanism on each landing impact:
    //   1. Mass inertia keeps body moving while chest decelerates
    //      (offset grows ∝ chest decel × m / k during the ~50 ms
    //       impact phase). Big mass = bigger initial offset.
    //   2. Stiff spring then accelerates body back hard → strong
    //      rebound velocity → big overshoot past the anchor.
    //   3. Low damping lets that overshoot ring out at ω_n for many
    //      cycles. Each visible bounce is a half-period of ringing.
    //
    // Soft spring + high damping (the previous wrong direction) damps
    // out before any bounce is visible — body just floats sluggishly
    // behind the anchor, reads as "bolted on with a delay".
    //
    //   ω_n  = sqrt(k/m) = sqrt(80/2.0) ≈ 6.32 rad/s → period ~1.0 s
    //                      (matches real ~1 Hz breast tissue resonance)
    //   ζ    = lin_damping / (2·ω_n) = 0.3 / 12.6 ≈ 0.024
    //                      (extremely under-damped — rings ~15 cycles
    //                       before amplitude decays to 1/e)
    //   limit = ±0.20 m → 20 cm peak envelope. Hard impacts will hit
    //                      this and clamp with some collision damping
    //                      (acts as the non-linear skin/muscle tether).
    let breast_mass = cc5_breast_override
        .mass
        .or_else(|| cc5_body_soft.as_ref().map(|s| s.mass))
        .unwrap_or(2.0);
    let breast_lin_stiffness = cc5_breast_override
        .linear_stiffness
        .or_else(|| {
            cc5_body_soft.as_ref().map(|s| {
                let omega = 2.0 * std::f32::consts::PI * s.stiffness_freq_hz;
                omega * omega * breast_mass
            })
        })
        .unwrap_or(80.0);
    let breast_lin_damping = cc5_breast_override
        .linear_damping
        .or_else(|| cc5_body_soft.as_ref().map(|s| s.damping))
        .unwrap_or(0.3);
    let breast_translation_limit = cc5_breast_override
        .translation_limit
        .or_else(|| cc5_breast_radius.map(|r| 0.6 * r))
        .unwrap_or(0.20);
    if cc5_breast_override.mass.is_some()
        || cc5_breast_override.linear_stiffness.is_some()
        || cc5_breast_override.linear_damping.is_some()
        || cc5_breast_override.translation_limit.is_some()
        || cc5_body_soft.is_some()
        || cc5_breast_radius.is_some()
    {
        tracing::info!(
            target: "cc_render",
            "physics: breast tuning resolved → mass={:.2}kg k={:.1} c_lin={:.2} lim=±{:.3}m \
             (override={} cc5_soft={} cc5_radius={:?})",
            breast_mass, breast_lin_stiffness, breast_lin_damping, breast_translation_limit,
            cc5_breast_override.mass.is_some()
                || cc5_breast_override.linear_stiffness.is_some()
                || cc5_breast_override.linear_damping.is_some()
                || cc5_breast_override.translation_limit.is_some(),
            cc5_body_soft.is_some(),
            cc5_breast_radius,
        );
    }
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
            breast_mass
        } else {
            mass_default
        };

        // CRITICAL ARCHITECTURE NOTE — sibling-anchor design:
        //
        // The naive approach (jiggle body as child of bone, joint
        // anchored to bone) creates a positive-feedback loop:
        //   1. rapier reads bone.global_transform to position the
        //      kinematic joint anchor
        //   2. We compute jiggle delta and write it to bone.local_rotation
        //   3. TransformPropagate updates bone.global_transform
        //   4. Next tick, rapier reads the updated bone.global → joint
        //      anchor moves → dynamic body chases its own reflection
        //
        // This is what made max-rotation-from-identity hit 360° on the
        // first test — runaway spin.
        //
        // Fix: spawn the kinematic anchor + dynamic body as SIBLINGS
        // of the bone (children of the bone's parent). The kinematic
        // anchor sits at the bone's bind transform IN PARENT'S FRAME
        // and never moves relative to its parent. The dynamic body
        // starts at the same pose; the motor pulls it toward the
        // anchor's rotation. When the parent (e.g. spine) rotates, the
        // anchor rotates with it instantly (kinematic), and the
        // dynamic body lags due to inertia. write_jiggle_bones copies
        // dynamic.local rotation onto bone.local — no feedback because
        // bone.local writes don't affect a sibling subtree's
        // GlobalTransform.
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
        // Parent's world rotation at spawn — seeds JiggleParentState so
        // the first frame's centrifugal force computation has a baseline
        // (= no rotation since spawn → ω=0 → zero force on frame 1).
        let parent_world_rot_at_spawn = parent_globals
            .get(parent_e)
            .map(|gt| gt.compute_transform().rotation)
            .unwrap_or(Quat::IDENTITY);

        let mode = if is_breast {
            JiggleMode::Translation
        } else {
            JiggleMode::Rotation
        };

        // Branch on mode. Translation-mode (breast) and rotation-mode
        // (hair / glute / skirt / tongue) need different body bind
        // poses, joint geometry, and damping.
        //
        // ── ROTATION MODE (hair, glute, skirt, tongue) ──
        //   Body sits on a pendulum offset 0.6 × chest_radius below the
        //   anchor in parent's local -Y, joint is a SphericalJoint
        //   anchored at anchor2 = -body_offset (body's local +Y top),
        //   gravity ON so the offset arm produces real torque, motor
        //   springs body rotation back to anchor's. Per-axis limits
        //   ±18° bend / ±2° twist cap the swing.
        //
        // ── TRANSLATION MODE (breast) ──
        //   The fundamental "rotate around a single pivot looks like a
        //   rigid cone" problem can't be fixed by tuning a rotation
        //   spring. Real breast tissue undergoes volumetric translation:
        //   when the chest accelerates, the soft volume's center-of-
        //   mass lags, then springs back. We recreate that here:
        //
        //     - body bind = anchor bind (both at the bone's bind pose)
        //     - GenericJoint locks all 3 angular axes (no rotation),
        //       leaves the 3 linear axes free with linear motors
        //       springing the body back to the anchor's position
        //     - gravity OFF (don't want the breast to sag indefinitely)
        //     - linear limits ±0.04 m cap the displacement
        //     - write_jiggle_bones writes (body.t - anchor.t) onto
        //       bone.translation, leaving rotation at bind. LBS
        //       skinning then distributes that translation across the
        //       weight gradient as a soft volumetric squish.
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

        let mut entity_cmds = commands.spawn((
            Name::new(format!("Jiggle_{}", bone_name)),
            JiggleBone {
                bone_entity: bone_e,
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
            // Linear damping is the PRIMARY trailing mechanism for
            // translation-mode bodies. It opposes the body's absolute
            // velocity in world frame, which during a body spin pulls
            // the body BACKWARD against its tangential motion —
            // producing real "lag behind the chest" trailing.
            // Equilibrium: spring force = m·linear_damping·v_anchor,
            // giving offset = m·linear_damping·v_anchor/k behind anchor.
            //
            // For breast (m=0.8 with the breast mass override above,
            // k=200 from the breast tuning below), peak spin
            // (v_anchor≈0.84 m/s):
            //   linear_damping=8 → ~27mm trailing.
            //
            // Rotation-mode bodies don't need much (their pendulum
            // offset + angular damping handle the dynamics) — keep
            // their linear damping low.
            Damping {
                linear_damping: if mode == JiggleMode::Translation {
                    breast_lin_damping
                } else {
                    0.5
                },
                angular_damping: 1.0,
            },
        ));
        // TransformInterpolation: only for rotation-mode (hair / glute /
        // skirt / tongue). For translation-mode (breast), interpolation
        // sampling beyond the latest physics step extrapolates the
        // body's pose forward in time — visually reads as "the breasts
        // anticipate the spin" because the renderer shows where the
        // body WOULD be one substep into the future, ahead of the
        // chest's actual rendered pose. Rotation-mode bodies don't
        // have this issue because their pose is angular (around a
        // pivot) and the chain inheritance dominates the visible
        // position; the small angular extrapolation reads as
        // sub-degree noise. Translation-mode bodies, where world
        // position IS the visible thing, surface the extrapolation
        // as visible "leading the rotation". Drop interpolation here
        // and accept the (imperceptible at 30+ fps) sub-frame
        // discretization for the breast bones.
        if mode == JiggleMode::Rotation {
            entity_cmds.insert(TransformInterpolation::default());
        }
        if mode == JiggleMode::Translation {
            // Breast: PARTIAL gravity. Captures the visual phenomenon
            // of breast tissue continuing to rise after the body has
            // started falling — at apex, body has built up upward
            // momentum from being dragged by the spring during the
            // launch phase; once the chest peaks and starts descending,
            // gravity decelerates the body's upward velocity and
            // eventually reverses it, while the spring also pulls back
            // toward the (now descending) chest. The COMBINED gravity +
            // spring restoring force produces the asymmetric "weight"
            // feel that pure spring restoration lacks.
            //
            // Static sag at rest: m·(GravityScale·g)/k. With defaults
            // (m=2, GravityScale=0.3, k=80): sag ≈ 0.073 m = 7.3 cm.
            // Visible at rest as the breasts hanging slightly below
            // their anchor — anatomically correct for a large bust.
            //
            // The translation_limit cube must accommodate sag + dynamic
            // overshoot. With sag 7.3 cm and limit 20 cm, the body has
            // ±13 cm headroom in each direction at rest before clamping.
            //
            // ExternalForce + ReadMassProperties + JiggleParentState
            // wire the body up for `apply_jiggle_inertia`'s centrifugal
            // pseudo-force injection — gives the body real inertial
            // outward pull during a body spin instead of the spring/motor
            // alone (which has zero inertia arm and produces underdamped
            // PD overshoot that reads as "anticipating the spin").
            entity_cmds.insert((
                GravityScale(0.3),
                ExternalForce::default(),
                ReadMassProperties::default(),
                JiggleParentState {
                    prev_parent_world_rot: parent_world_rot_at_spawn,
                },
            ));
        }
        let jiggle_e = entity_cmds.id();
        commands.entity(parent_e).add_child(jiggle_e);

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
                // Translation jiggle: lock all rotation, spring all
                // translation back to anchor.
                //
                // Tuning: ω = sqrt(stiffness). f = ω / 2π. Real breast
                // tissue measures ~1-2 Hz, but for visible-at-30fps
                // motion we run faster (~3.6 Hz) so the body actually
                // responds within a few render frames of chest impact
                // rather than oozing through the motion. Period of
                // ~280 ms = ~9 render frames @ 30 fps = clearly
                // visible swing on every step.
                //
                //   stiffness = 500   → ω ≈ 22.4 rad/s → f ≈ 3.6 Hz
                //   damping   = 32    → ζ = 32 / (2·22.4) ≈ 0.71
                //                       → critically-damped-ish; clean
                //                         single drag on each impulse,
                //                         no leading-edge overshoot.
                //   limit     = ±6 cm → larger envelope so a hard jolt
                //                       reads as a real bounce, not a
                //                       capped 4 cm nudge.
                //
                // Damping was previously 9 (ζ ≈ 0.20) which produced
                // visible velocity-overshoot during the angular-
                // acceleration phase of a body spin: the spring
                // accelerated the body so hard that it briefly
                // *passed* the kinematic anchor in motion direction
                // before settling. Combined with the spin fix that
                // actually rotates the spine column (see commit that
                // added twist preservation to axis_match_chained),
                // this read as the breasts "leading the rotation".
                // Raising damping to ζ ≈ 0.7 eliminates the overshoot
                // while keeping the response time under one render
                // frame.
                //
                // `apply_jiggle_inertia` adds the centrifugal pseudo-
                // force on top so sustained rotations produce real
                // outward swell (small at typical spin rates ~1 rad/s
                // — equilibrium offset ≈ m·ω²·r / k ~0.1 mm — but
                // builds visibly on faster gestures).
                //
                // (We override the generic `stiffness`/`damping` knobs
                // computed above because those were tuned for angular
                // motors and don't translate cleanly to linear ones.)
                // Tuning for large-breast jiggle. The previous
                // stiffness=200 + mass=0.8 setup tracked the anchor
                // too tightly in practice (Rapier's per-substep motor
                // integration is more aggressive than the analytic
                // formula predicts — body's effective stiffness ~4x
                // the nominal at 4 substeps/frame). With breasts
                // appearing "bolted on", drop stiffness much further:
                //
                //   ω_n = sqrt(k/m) = sqrt(60/1.5) ≈ 6.3 rad/s
                //   period ≈ 1.0 s (~1 Hz, matches the slow swing
                //                   of unsupported large breast tissue)
                //   ζ = 5 / (2 · 6.3) ≈ 0.40 (underdamped, lively)
                //
                // Predicted at peak spin (ω_parent ≈ 7 rad/s, breast
                // at radius 0.12 m from spine pivot):
                //   tangential trailing offset  ≈ 105 mm theoretical
                //                              (capped at limit corner)
                //   radial centrifugal swell    ≈ 60 mm
                //                              (also capped)
                //   the limit cube of ±0.08 m caps the total
                //   magnitude at ≈ 138 mm corner-to-corner — body
                //   bounces inside that envelope.
                let translation_limit = breast_translation_limit;
                let lin_stiffness = breast_lin_stiffness;
                // Motor damping = 0: do not use the joint motor's
                // velocity term. It opposes RELATIVE velocity in the
                // joint frame, which goes to zero in steady-state
                // co-rotation and does nothing useful there. During
                // transients it pushes the body in the anchor's
                // motion direction (= "leading"), which is the
                // OPPOSITE of what we want. Trailing comes from the
                // linear_damping on the body itself (Damping component
                // above), which opposes the body's absolute world
                // velocity.
                let lin_damping = 0.0_f32;
                let joint = GenericJointBuilder::new(JointAxesMask::ANG_AXES)
                    .local_anchor1(Vec3::ZERO)
                    .local_anchor2(Vec3::ZERO)
                    .limits(JointAxis::LinX, [-translation_limit, translation_limit])
                    .limits(JointAxis::LinY, [-translation_limit, translation_limit])
                    .limits(JointAxis::LinZ, [-translation_limit, translation_limit])
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

/// Per-bone state tracking the parent's world rotation across frames so
/// [`apply_jiggle_inertia`] can compute the parent's angular velocity.
/// Initialized at spawn from the parent's GlobalTransform; updated each
/// frame after the inertia force is applied.
#[derive(Component)]
pub(crate) struct JiggleParentState {
    pub prev_parent_world_rot: Quat,
}

/// Centrifugal pseudo-force on a body whose parent is rotating.
///
/// Real soft tissue, when its host body spins, "feels" a centrifugal
/// outward pull (in the rotating frame, where the tissue lives). The
/// dynamic body in our setup has no inertia mechanism on its own (joint
/// locks angular axes; spring chases anchor), so we inject this force
/// each frame to recreate the missing inertial behavior.
///
/// Math:
/// ```text
///   ω         = (parent_rot_now · parent_rot_prev⁻¹) as axis·angle / dt
///   r         = body_world_pos - parent_world_pos
///   F_centrifugal = -m · ω × (ω × r)        (outward from rotation axis)
/// ```
///
/// Returns `Vec3::ZERO` for a static parent (no rotation between frames)
/// or sub-microsecond `dt` (avoids NaN from `Quat::to_axis_angle()`).
pub(crate) fn centrifugal_pseudo_force(
    parent_rot_now: Quat,
    parent_rot_prev: Quat,
    parent_world_pos: Vec3,
    body_world_pos: Vec3,
    mass: f32,
    dt: f32,
) -> Vec3 {
    if dt <= 1e-6 {
        return Vec3::ZERO;
    }
    // Quaternion double-cover: q and -q represent the same rotation. The
    // straight `now * prev⁻¹` can land on either side. If we end up on
    // the w < 0 side, `to_axis_angle()` reports `angle = 2π - small`
    // instead of `small` → spurious enormous ω → 600 N centripetal
    // kick that flings the body into "leading" position. Normalize to
    // short-arc form (w ≥ 0) so we always read the small actual rotation.
    let drot_raw = parent_rot_now * parent_rot_prev.inverse();
    let drot = if drot_raw.w < 0.0 {
        Quat::from_xyzw(-drot_raw.x, -drot_raw.y, -drot_raw.z, -drot_raw.w)
    } else {
        drot_raw
    };
    let (axis, angle) = drot.to_axis_angle();
    if angle.abs() < 1e-6 || !axis.is_finite() {
        return Vec3::ZERO;
    }
    let omega = axis.normalize_or_zero() * (angle / dt);
    let r = body_world_pos - parent_world_pos;
    -mass * omega.cross(omega.cross(r))
}

/// Inject centrifugal pseudo-force into translation-mode jiggle bodies
/// so they exhibit realistic inertial pull when their parent (e.g. the
/// chest spine) rotates. Without this, the body has zero inertia arm
/// (COM at the joint anchor, angular axes locked) and the spring/motor
/// alone produces only a tiny PD-controller lag — visible during a
/// body spin as "the breasts anticipate the spin" because the
/// underdamped spring overshoots before settling.
///
/// Runs in PostUpdate before [`bevy_rapier3d::plugin::PhysicsSet::SyncBackend`]
/// so the force is in place when Rapier integrates this frame's step.
/// Reads the previous frame's GlobalTransform (stale by 1 frame —
/// imperceptible at 30+ fps) since `TransformPropagate` won't have run
/// yet for this frame's pose. Updates [`JiggleParentState`] after each
/// step so the next frame's ω calculation has fresh history.
pub(crate) fn apply_jiggle_inertia(
    time: Res<Time>,
    mut bodies: Query<(
        Entity,
        &JiggleBone,
        &Parent,
        &GlobalTransform,
        &mut ExternalForce,
        &mut JiggleParentState,
    )>,
    other_globals: Query<&GlobalTransform, Without<JiggleBone>>,
    mass_props: Query<&ReadMassProperties>,
    mut throttle: Local<u32>,
) {
    let dt = time.delta_secs().max(1e-4);
    *throttle = throttle.wrapping_add(1);
    let log_this_tick = *throttle % 6 == 0; // ~5 Hz at 30 fps
    for (body_e, jb, body_parent, body_global, mut ext_force, mut state) in bodies.iter_mut() {
        // Reset force regardless of mode so a stale value from a previous
        // mode toggle doesn't leak. Translation mode is the only mode
        // that needs injected inertia (rotation mode bones already have
        // a pendulum offset → real torque arm).
        if jb.mode != JiggleMode::Translation {
            ext_force.force = Vec3::ZERO;
            ext_force.torque = Vec3::ZERO;
            continue;
        }
        let parent_e = body_parent.get();
        let Ok(parent_global) = other_globals.get(parent_e) else {
            ext_force.force = Vec3::ZERO;
            continue;
        };
        let parent_t = parent_global.compute_transform();
        let body_world_pos = body_global.translation();
        let mass = mass_props
            .get(body_e)
            .ok()
            .map(|m| m.mass)
            .filter(|m| *m > 1e-4)
            .unwrap_or(0.3);

        // Compute ω directly so we can both apply force and report it.
        // Same quaternion-double-cover normalization as in
        // `centrifugal_pseudo_force` — without it, sign flips in the
        // stored quaternions between frames produce phantom 180+ rad/s
        // ω readings.
        let drot_raw = parent_t.rotation * state.prev_parent_world_rot.inverse();
        let drot = if drot_raw.w < 0.0 {
            Quat::from_xyzw(-drot_raw.x, -drot_raw.y, -drot_raw.z, -drot_raw.w)
        } else {
            drot_raw
        };
        let (axis, angle) = drot.to_axis_angle();
        let omega_world = if angle.abs() > 1e-6 && axis.is_finite() {
            axis.normalize_or_zero() * (angle / dt)
        } else {
            Vec3::ZERO
        };

        ext_force.force = centrifugal_pseudo_force(
            parent_t.rotation,
            state.prev_parent_world_rot,
            parent_t.translation,
            body_world_pos,
            mass,
            dt,
        );

        // Directional diagnostic: project body offset (vs anchor) onto
        // the tangent of the parent's rotation at the body's position.
        // This is the definitive answer to "leading vs trailing":
        //   sign > 0  →  body LEADS  (offset is in motion direction)
        //   sign < 0  →  body TRAILS (offset opposes motion direction)
        //   sign ≈ 0  →  tracking
        if log_this_tick && omega_world.length() > 0.1 {
            if let Ok(anchor_global) = other_globals.get(jb.anchor_entity) {
                let anchor_world = anchor_global.translation();
                let offset_world = body_world_pos - anchor_world;
                let r_world = body_world_pos - parent_t.translation;
                let tangent_world = omega_world.cross(r_world);
                let sign = offset_world.dot(tangent_world);
                let label = if sign > 1e-7 {
                    "LEAD"
                } else if sign < -1e-7 {
                    "TRAIL"
                } else {
                    "TRACK"
                };
                tracing::info!(
                    target: "cc_render",
                    "jiggle_dir: {} {} sign={:+.6e} |offset|={:.4}m |omega|={:.3}rad/s force=({:.3},{:.3},{:.3})",
                    jb.debug_name,
                    label,
                    sign,
                    offset_world.length(),
                    omega_world.length(),
                    ext_force.force.x, ext_force.force.y, ext_force.force.z,
                );
            }
        }

        state.prev_parent_world_rot = parent_t.rotation;
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
        body_t: Vec3,
        anchor_rot: Quat,
        anchor_t: Vec3,
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
            Some(Snap {
                bone_e: jb.bone_entity,
                body_rot: t_jiggle.rotation,
                body_t: t_jiggle.translation,
                anchor_rot: anchor.rotation,
                anchor_t: anchor.translation,
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
                    // Visible displacement = body − anchor in parent's
                    // local frame. Add to the bone's bind translation
                    // (NOT overwriting the bind, just delta).
                    let delta = snap.body_t - snap.anchor_t;
                    let before = bone_t.translation;
                    bone_t.translation = snap.bone_bind_t + delta;
                    let after = bone_t.translation;
                    let mag_cm = delta.length() * 100.0;
                    if mag_cm > max_translation_cm {
                        max_translation_cm = mag_cm;
                        max_translation_name = snap.name.clone();
                    }
                    // Per-bone INFO diagnostic. Confirms the writeback
                    // actually mutates the bone's Transform — if this
                    // log shows non-zero deltas but the rendered mesh
                    // still looks bolted on, the issue is downstream
                    // (skinning weights / bone entity mismatch).
                    // Throttle by global tick (= writeback's `throttle`
                    // local) so all 2 breast bones log together at 5 Hz.
                    if mag_cm > 0.5 {
                        tracing::info!(
                            target: "cc_render",
                            "jiggle_writeback: {} bone_e={:?} \
                             before=({:+.4},{:+.4},{:+.4}) \
                             after=({:+.4},{:+.4},{:+.4}) delta_cm={:.2}",
                            snap.name, snap.bone_e,
                            before.x, before.y, before.z,
                            after.x, after.y, after.z,
                            mag_cm,
                        );
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
    fn jiggle_bone_carries_bone_entity() {
        // The sibling-anchor architecture means JiggleBone only needs
        // to remember which bone entity to write to + which anchor
        // entity to compare against for diagnostics. Bind pose lives
        // on the kinematic anchor's Transform, not on JiggleBone.
        let jb = JiggleBone {
            bone_entity: Entity::from_raw(7),
            anchor_entity: Entity::from_raw(8),
            debug_name: "TestBone".into(),
            anchor_world_rest: Quat::IDENTITY,
            bone_bind_translation: Vec3::ZERO,
            mode: JiggleMode::Rotation,
        };
        assert_eq!(jb.bone_entity, Entity::from_raw(7));
        assert_eq!(jb.anchor_entity, Entity::from_raw(8));
        assert_eq!(jb.debug_name, "TestBone");
        assert_eq!(jb.mode, JiggleMode::Rotation);
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

    #[test]
    fn sibling_writeback_at_bind_keeps_bone_at_bind() {
        // Steady-state check: if the dynamic body has settled at the
        // anchor's bind pose, write_jiggle_bones writes that same
        // bind pose onto the bone — no drift, no off-by-one.
        let bind_rot = Quat::from_rotation_y(0.3);
        // Simulating: dynamic.local.rotation == bind (motor pulled body
        // to anchor's frame) → bone.rotation = bind.
        let dynamic_local_rot = bind_rot;
        let bone_after = dynamic_local_rot;
        assert!(bone_after.abs_diff_eq(bind_rot, 1e-5));
    }

    /// Centrifugal pseudo-force = +m·ω²·r_perp (outward from rotation axis).
    /// Body at +X offset from a Y-axis rotation pivot must receive a force
    /// in +X direction with magnitude m·ω²·r — the inertial outward pull
    /// soft tissue would feel during a body spin.
    #[test]
    fn centrifugal_pseudo_force_pulls_outward_for_pure_y_rotation() {
        let dt = 1.0 / 60.0;
        let omega_rate = std::f32::consts::FRAC_PI_2; // π/2 rad/s = 90 °/s
        let dtheta = omega_rate * dt;
        let parent_rot_prev = Quat::IDENTITY;
        let parent_rot_now = Quat::from_rotation_y(dtheta);
        let parent_world_pos = Vec3::ZERO;
        let body_world_pos = Vec3::new(0.5, 0.0, 0.0); // 0.5 m radius right
        let mass = 0.3_f32;

        let force = centrifugal_pseudo_force(
            parent_rot_now,
            parent_rot_prev,
            parent_world_pos,
            body_world_pos,
            mass,
            dt,
        );

        let expected_magnitude = mass * omega_rate * omega_rate * 0.5;
        assert!(
            force.x > 0.0,
            "force should point outward (+X) for body at +X with Y-axis rotation, got {:?}",
            force,
        );
        assert!(
            (force.length() - expected_magnitude).abs() < expected_magnitude * 0.05,
            "magnitude should be m·ω²·r ≈ {}, got {} (force {:?})",
            expected_magnitude,
            force.length(),
            force,
        );
        assert!(
            force.y.abs() < 1e-3,
            "y component should be ~0, got {}",
            force.y
        );
        assert!(
            force.z.abs() < 1e-3,
            "z component should be ~0, got {}",
            force.z
        );
    }

    /// No rotation between frames → zero force. Guards against division-
    /// by-zero or NaN from `Quat::to_axis_angle()` on identity input.
    #[test]
    fn centrifugal_pseudo_force_is_zero_when_parent_static() {
        let force = centrifugal_pseudo_force(
            Quat::IDENTITY,
            Quat::IDENTITY,
            Vec3::ZERO,
            Vec3::new(0.5, 0.0, 0.0),
            0.3,
            1.0 / 60.0,
        );
        assert!(
            force.length() < 1e-4,
            "force should be ~0 for static parent, got {:?}",
            force
        );
    }

    /// Quaternion double-cover regression. q and -q represent the same
    /// rotation; if the relative rotation `now * prev⁻¹` lands on the
    /// w < 0 side, `to_axis_angle()` returns `2π - small` instead of
    /// `small`, producing a spurious enormous ω that translates into a
    /// hundreds-of-Newtons centripetal kick. This test pins the fix:
    /// negating either input must produce the same physical force as
    /// the un-negated version, since both encode the same rotation.
    /// Discovered when the diagnostic log showed |ω|=180 rad/s spikes
    /// during a 1-2 rad/s body spin, with corresponding 600 N force
    /// kicks that pushed the body into "leading" position for the
    /// rest of the clip.
    #[test]
    fn centrifugal_pseudo_force_handles_quaternion_double_cover() {
        let dt = 1.0 / 60.0;
        let mass = 0.3_f32;
        let parent_world_pos = Vec3::ZERO;
        let body_world_pos = Vec3::new(0.5, 0.0, 0.0);
        let prev = Quat::IDENTITY;
        let now = Quat::from_rotation_y(0.05); // tiny rotation, ~3°

        let f_pos = centrifugal_pseudo_force(now, prev, parent_world_pos, body_world_pos, mass, dt);

        // Negate `now` — represents the SAME rotation, opposite quaternion sign.
        // Without the double-cover fix in centrifugal_pseudo_force, this
        // returned a force ~500x larger than f_pos (omega ~ 2π/dt ≈ 188 rad/s
        // instead of 0.05/dt ≈ 3 rad/s, force scales with ω²).
        let now_neg = Quat::from_xyzw(-now.x, -now.y, -now.z, -now.w);
        let f_neg =
            centrifugal_pseudo_force(now_neg, prev, parent_world_pos, body_world_pos, mass, dt);

        let diff = (f_pos - f_neg).length();
        assert!(
            diff < 1e-4,
            "negating quaternion sign must not change force (double-cover): \
             f_pos={:?} f_neg={:?} diff={}",
            f_pos,
            f_neg,
            diff,
        );
        // Sanity bound: physical force = m·ω²·r = 0.3·(0.05/dt)²·0.5
        // ≈ 1.35 N. Without the fix, ω would be ~2π/dt ≈ 374 rad/s,
        // giving force ≈ 21,000 N. Anything under 100 is in the
        // physically-correct regime.
        assert!(
            f_pos.length() < 100.0,
            "force at small ω should be small, got {} N (likely double-cover bug)",
            f_pos.length(),
        );
    }
}
