//! Per-hand grab state machine. Pure-Rust FSM (`step_fsm`) is decoupled
//! from Bevy ECS for unit testing. The Bevy `handle_grab_cmds` system
//! collects per-hand inputs, runs the FSM, and applies transitions
//! including `ImpulseJoint` create/destroy.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use super::body_colliders::BodyBoneCollider;
use super::contact_grasp::capture_joint;
use super::props::{Prop, PropRegistry};
use crate::cc_render::renderer::{GrabFailure, GrabOrRelease, Hand, PropId};

#[derive(Debug, Clone, Default)]
pub(crate) enum GrabState {
    #[default]
    Idle,
    Pursuing {
        target: PropId,
        elapsed: f32,
        palm_override: Option<[f32; 3]>,
    },
    Attached {
        target: PropId,
        prop_entity: Entity,
        /// Dedicated child joint; never replaces the prop's permanent joints.
        grip_entity: Entity,
    },
}

#[derive(Component)]
pub(crate) struct ContactGrip;

#[derive(Resource, Default)]
pub(crate) struct GrabFsms {
    pub left: GrabState,
    pub right: GrabState,
}

#[derive(Debug)]
pub(crate) enum FsmTransition {
    NoChange,
    StartPursuit {
        target: PropId,
        palm_override: Option<[f32; 3]>,
    },
    Attach {
        target: PropId,
    },
    DetachIdle,
    Failed {
        reason: String,
        target: PropId,
    },
}

/// Pure FSM step — no Bevy types, fully unit-testable.
pub(crate) fn step_fsm(
    state: &GrabState,
    wrist_pos: Vec3,
    grasp_pos: Option<Vec3>,
    dt: f32,
    new_cmd: Option<&GrabOrRelease>,
    hand: Hand,
    proximity_m: f32,
    timeout_secs: f32,
) -> FsmTransition {
    match new_cmd {
        Some(GrabOrRelease::Grab(g)) if g.hand == hand => {
            return FsmTransition::StartPursuit {
                target: g.target.clone(),
                palm_override: g.palm_offset_override,
            };
        }
        Some(GrabOrRelease::Release(h)) if *h == hand => {
            return FsmTransition::DetachIdle;
        }
        _ => {}
    }
    if matches!(new_cmd,Some(GrabOrRelease::CancelPursuit(h)) if *h==hand)
        && matches!(state, GrabState::Pursuing { .. })
    {
        return FsmTransition::DetachIdle;
    }
    match state {
        GrabState::Idle => FsmTransition::NoChange,
        GrabState::Pursuing {
            target, elapsed, ..
        } => {
            if let Some(grasp) = grasp_pos {
                if (wrist_pos - grasp).length() <= proximity_m {
                    return FsmTransition::Attach {
                        target: target.clone(),
                    };
                }
            }
            if elapsed + dt >= timeout_secs {
                return FsmTransition::Failed {
                    reason: "timeout".into(),
                    target: target.clone(),
                };
            }
            FsmTransition::NoChange
        }
        GrabState::Attached { .. } => FsmTransition::NoChange,
    }
}

/// Bevy system. Drains GrabCmdRx, runs the per-hand FSM, applies transitions.
///
/// NOTE: takes only `ResMut<PropRegistry>` (not Res + ResMut) to avoid a
/// scheduler conflict; reads through `&*prop_registry` where needed.
pub(crate) fn handle_grab_cmds(
    mut commands: Commands,
    time: Res<Time>,
    mut inbox: crate::cc_render::office::systems::OfficeCommandInbox,
    mut fsms: ResMut<GrabFsms>,
    mut prop_registry: ResMut<PropRegistry>,
    body_colliders: Query<(Entity, &BodyBoneCollider, &GlobalTransform)>,
    props: Query<(&Prop, &GlobalTransform, Option<&super::props::HandleExtent>)>,
    physics: Query<&RapierContext>,
    stored_cfg: Res<super::StoredPhysicsConfig>,
) {
    let dt = time.delta_secs();
    let proximity_m = stored_cfg.0.grab.proximity_m;
    let timeout_secs = stored_cfg.0.grab.timeout_secs;

    let cmds = inbox.drain();

    // Wrist world positions + entities (SMPL idx 20 = L_Wrist, 21 = R_Wrist).
    let mut wrist_pos: HashMap<Hand, Vec3> = HashMap::new();
    let mut wrist_bodies: HashMap<Hand, (Entity, GlobalTransform)> = HashMap::new();
    for (e, bc, gt) in body_colliders.iter() {
        let h = match bc.smpl_idx {
            20 => Some(Hand::Left),
            21 => Some(Hand::Right),
            _ => None,
        };
        if let Some(hand) = h {
            wrist_pos.insert(hand, gt.translation());
            wrist_bodies.insert(hand, (e, *gt));
        }
    }

    // Per-hand transition collection (split borrow safe).
    let mut transitions: Vec<(Hand, FsmTransition)> = Vec::new();
    for hand in [Hand::Left, Hand::Right] {
        let state = match hand {
            Hand::Left => &fsms.left,
            Hand::Right => &fsms.right,
        };
        // LATEST-WINS: if multiple cmds for this hand arrive in a single
        // tick, only the most recent is processed. Intermediate cmds are
        // dropped. Safe under the current contract: Kimodo and the LLM
        // emit at most one motion intent / grab-release pair per frame.
        // If this assumption breaks (e.g., 10 Hz pre-flight pursuit anim),
        // process cmds serially via a per-hand inner loop instead.
        let cmd = cmds.iter().rfind(|c| match c {
            GrabOrRelease::Grab(g) => g.hand == hand,
            GrabOrRelease::Release(h) => *h == hand,
            GrabOrRelease::Office(_) => false,
            GrabOrRelease::CancelPursuit(h) => *h == hand,
        });
        let wrist = wrist_pos.get(&hand).copied().unwrap_or(Vec3::ZERO);
        let target = match state {
            GrabState::Pursuing { target, .. } => prop_registry
                .by_id
                .get(target)
                .copied()
                .and_then(|e| props.get(e).ok().map(|(prop, transform, handle)| (e, prop, transform, handle))),
            _ => None,
        };
        let reach = super::contact_grasp::target_position(physics.get_single().ok(),
            wrist_bodies.get(&hand).map(|(entity, _)| (*entity, wrist)), target, proximity_m);
        let trans = step_fsm(
            state,
            wrist,
            reach.map(|(point, _)| point),
            dt,
            cmd,
            hand,
            reach.map_or(proximity_m, |(_, within)| within),
            timeout_secs,
        );
        transitions.push((hand, trans));
    }

    // Apply transitions (immutable phase done; safe to mut-borrow fsms).
    for (hand, trans) in transitions {
        let state = match hand {
            Hand::Left => &mut fsms.left,
            Hand::Right => &mut fsms.right,
        };
        match trans {
            FsmTransition::NoChange => {
                if let GrabState::Pursuing { elapsed, .. } = state {
                    *elapsed += dt;
                }
            }
            FsmTransition::StartPursuit {
                target,
                palm_override,
            } => {
                // If we're swapping out a previously-attached prop, release
                // its joint first to avoid leaking the FixedJoint.
                if let GrabState::Attached { grip_entity, .. } = state {
                    commands.entity(*grip_entity).try_despawn_recursive();
                    tracing::info!(
                        target: "cc_render",
                        "physics: grab swap — hand={:?} released previous prop for new target={}",
                        hand, target,
                    );
                }
                if palm_override.is_some() {
                    *state = GrabState::Idle;
                    prop_registry.pending_failures.push(GrabFailure {
                        hand, target, reason: "contact_grip_rejects_forced_palm_offset".into(),
                    });
                    continue;
                }
                tracing::info!(
                    target: "cc_render",
                    "physics: grab pursuit started — hand={:?} target={} palm_override={:?}",
                    hand, target, palm_override,
                );
                *state = GrabState::Pursuing {
                    target,
                    elapsed: 0.0,
                    palm_override,
                };
            }
            FsmTransition::Attach { target } => {
                let Some(prop_e) = prop_registry.by_id.get(&target).copied() else {
                    *state = GrabState::Idle;
                    prop_registry.pending_failures.push(GrabFailure {
                        hand,
                        target,
                        reason: "prop_not_found".into(),
                    });
                    continue;
                };
                let Some((wrist_e, wrist_world)) = wrist_bodies.get(&hand).copied() else {
                    *state = GrabState::Idle;
                    prop_registry.pending_failures.push(GrabFailure {
                        hand,
                        target,
                        reason: "wrist_not_found".into(),
                    });
                    continue;
                };
                let active_contact = physics.get_single().ok().is_some_and(|context|
                    super::contact_grasp::has_object_contact(context, wrist_e, prop_e));
                let joint = props.get(prop_e).ok().and_then(|(_, object_world, _)| {
                    capture_joint(&wrist_world, object_world, active_contact)
                });
                let Some(joint) = joint else {
                    *state = GrabState::Idle;
                    prop_registry.pending_failures.push(GrabFailure {
                        hand, target, reason: "physical_contact_lost".into(),
                    });
                    continue;
                };
                // Rapier resolves the second endpoint through the entity's
                // parent. A separate child lets permanent hinges coexist with
                // this hand's temporary grip (and another hand's grip).
                let grip_entity = commands
                    .spawn((ContactGrip, ImpulseJoint::new(wrist_e, joint)))
                    .set_parent(prop_e)
                    .id();
                tracing::info!(
                    target: "cc_render",
                    "physics: contact grip attached without repositioning — hand={:?} target={}",
                    hand, target,
                );
                *state = GrabState::Attached {
                    target,
                    prop_entity: prop_e,
                    grip_entity,
                };
            }
            FsmTransition::DetachIdle => {
                if let GrabState::Attached { grip_entity, .. } = *state {
                    commands.entity(grip_entity).try_despawn_recursive();
                    tracing::info!(
                        target: "cc_render",
                        "physics: grab released — hand={:?}", hand,
                    );
                }
                *state = GrabState::Idle;
            }
            FsmTransition::Failed { reason, target } => {
                tracing::warn!(
                    target: "cc_render",
                    "physics: grab failed — hand={:?} target={} reason={}",
                    hand, target, reason,
                );
                prop_registry.pending_failures.push(GrabFailure {
                    hand,
                    target,
                    reason,
                });
                *state = GrabState::Idle;
            }
        }
    }
}

/// Historical canonical-frame fixture, not the production contact-grip policy.
#[cfg(test)]
fn grasp_joint(palm: Vec3, calibration: Quat, prop: Option<&Prop>) -> FixedJoint {
    let grasp_rotation = prop
        .and_then(|p| p.grasp_local)
        .map_or(Quat::IDENTITY, |m| {
            Mat4::from_cols_array_2d(&m)
                .to_scale_rotation_translation()
                .1
        });
    // Both joint frames meet in canonical grasp space, matching contact IK:
    // wrist_world * calibration^-1 == prop_world * grasp_local_rotation.
    FixedJointBuilder::new()
        .local_basis1(calibration.inverse())
        .local_basis2(grasp_rotation)
        .local_anchor1(palm)
        .local_anchor2(prop.map_or(Vec3::ZERO, Prop::grasp_offset))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::renderer::{GrabCmd, Hand};

    #[test]
    fn physics_grip_preserves_canonical_frame_during_carry() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<SceneSpawner>();
        app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default());
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ));
        let calibration = Quat::from_euler(EulerRot::XYZ, 0.4, -0.8, 1.2);
        let grip = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
        let offset = Vec3::new(0.075, 0., 0.);
        let prop = Prop {
            id: "mug".into(),
            loading: false,
            grasp_local: Some(Mat4::from_rotation_translation(grip, offset).to_cols_array_2d()),
        };
        let origin = Vec3::Y;
        let wrist = app
            .world_mut()
            .spawn((
                Transform::from_translation(origin).with_rotation(grip * calibration),
                RigidBody::KinematicPositionBased,
            ))
            .id();
        let mug = app
            .world_mut()
            .spawn((
                Transform::from_translation(origin - offset),
                RigidBody::Dynamic,
                Collider::ball(0.02),
                ImpulseJoint::new(wrist, grasp_joint(Vec3::ZERO, calibration, Some(&prop))),
            ))
            .id();
        // Begin aligned, then turn through 90 degrees and settle. A raw CC5
        // wrist basis must never become the prop's world orientation.
        for frame in 0..180 {
            let yaw = (frame as f32 / 120.0).min(1.0) * std::f32::consts::FRAC_PI_2;
            let desired = Quat::from_rotation_y(yaw);
            app.world_mut()
                .get_mut::<Transform>(wrist)
                .unwrap()
                .rotation = desired * grip * calibration;
            app.update();
            if frame > 120 {
                let actual = app.world().get::<Transform>(mug).unwrap();
                assert!(
                    actual.rotation.angle_between(desired) < 0.005,
                    "prop rotated out of canonical grip: {:?}",
                    actual.rotation
                );
                assert!((actual.translation + actual.rotation * offset).distance(origin) < 0.001);
            }
        }
    }

    #[test]
    fn fsm_idle_to_pursuing_on_grab_cmd() {
        let state = GrabState::Idle;
        let cmd = GrabOrRelease::Grab(GrabCmd {
            hand: Hand::Right,
            target: "mug".into(),
            palm_offset_override: None,
        });
        let trans = step_fsm(
            &state,
            Vec3::ZERO,
            None,
            0.016,
            Some(&cmd),
            Hand::Right,
            0.05,
            2.0,
        );
        assert!(matches!(trans, FsmTransition::StartPursuit { ref target, .. } if target == "mug"));
    }

    #[test]
    fn fsm_pursuing_to_attached_on_proximity() {
        let state = GrabState::Pursuing {
            target: "mug".into(),
            elapsed: 0.5,
            palm_override: None,
        };
        let trans = step_fsm(
            &state,
            Vec3::new(0.0, 1.0, 0.0),
            Some(Vec3::new(0.0, 1.02, 0.0)),
            0.016,
            None,
            Hand::Right,
            0.05,
            2.0,
        );
        assert!(matches!(trans, FsmTransition::Attach { ref target } if target == "mug"));
    }

    #[test]
    fn fsm_pursuing_no_proximity_returns_no_change() {
        let state = GrabState::Pursuing {
            target: "mug".into(),
            elapsed: 0.5,
            palm_override: None,
        };
        let trans = step_fsm(
            &state,
            Vec3::new(0.0, 1.0, 0.0),
            Some(Vec3::new(0.0, 2.0, 0.0)),
            0.016,
            None,
            Hand::Right,
            0.05,
            2.0,
        );
        assert!(matches!(trans, FsmTransition::NoChange));
    }

    #[test]
    fn fsm_pursuing_timeout_emits_failed() {
        let state = GrabState::Pursuing {
            target: "mug".into(),
            elapsed: 1.99,
            palm_override: None,
        };
        let trans = step_fsm(
            &state,
            Vec3::new(0.0, 1.0, 0.0),
            Some(Vec3::new(0.0, 2.0, 0.0)),
            0.016,
            None,
            Hand::Right,
            0.05,
            2.0,
        );
        assert!(matches!(trans, FsmTransition::Failed { ref reason, .. } if reason == "timeout"));
    }

    #[test]
    fn fsm_release_detaches() {
        let state = GrabState::Attached {
            target: "mug".into(),
            prop_entity: Entity::from_raw(42),
            grip_entity: Entity::from_raw(43),
        };
        let cmd = GrabOrRelease::Release(Hand::Right);
        let trans = step_fsm(
            &state,
            Vec3::ZERO,
            None,
            0.016,
            Some(&cmd),
            Hand::Right,
            0.05,
            2.0,
        );
        assert!(matches!(trans, FsmTransition::DetachIdle));
    }

    #[test]
    fn fsm_attached_swap_to_new_target() {
        // Grab arriving while Attached should start pursuing the new target.
        // (The Bevy apply layer is responsible for releasing the previous joint.)
        let state = GrabState::Attached {
            target: "mug".into(),
            prop_entity: Entity::from_raw(42),
            grip_entity: Entity::from_raw(43),
        };
        let cmd = GrabOrRelease::Grab(GrabCmd {
            hand: Hand::Right,
            target: "ball".into(),
            palm_offset_override: None,
        });
        let trans = step_fsm(
            &state,
            Vec3::ZERO,
            None,
            0.016,
            Some(&cmd),
            Hand::Right,
            0.05,
            2.0,
        );
        assert!(matches!(
            trans,
            FsmTransition::StartPursuit { ref target, .. } if target == "ball"
        ));
    }

    #[test]
    fn fsm_grab_propagates_palm_override() {
        let state = GrabState::Idle;
        let cmd = GrabOrRelease::Grab(GrabCmd {
            hand: Hand::Right,
            target: "mug".into(),
            palm_offset_override: Some([0.1, 0.2, 0.3]),
        });
        let trans = step_fsm(
            &state,
            Vec3::ZERO,
            None,
            0.016,
            Some(&cmd),
            Hand::Right,
            0.05,
            2.0,
        );
        match trans {
            FsmTransition::StartPursuit { palm_override, .. } => {
                assert_eq!(palm_override, Some([0.1, 0.2, 0.3]));
            }
            other => panic!("expected StartPursuit, got {:?}", other),
        }
    }

    #[test]
    fn fsm_ignores_other_hand_cmds() {
        let state = GrabState::Idle;
        let cmd = GrabOrRelease::Grab(GrabCmd {
            hand: Hand::Left,
            target: "mug".into(),
            palm_offset_override: None,
        });
        let trans = step_fsm(
            &state,
            Vec3::ZERO,
            None,
            0.016,
            Some(&cmd),
            Hand::Right,
            0.05,
            2.0,
        );
        assert!(matches!(trans, FsmTransition::NoChange));
    }

    #[test]
    fn fsm_pursuing_to_attached_with_custom_proximity() {
        // Wrist 8cm from grasp; default 5cm gate → NoChange; 10cm gate → Attach.
        let state = GrabState::Pursuing {
            target: "mug".into(),
            elapsed: 0.5,
            palm_override: None,
        };
        let trans_default = step_fsm(
            &state,
            Vec3::new(0.0, 1.0, 0.0),
            Some(Vec3::new(0.0, 1.08, 0.0)),
            0.016,
            None,
            Hand::Right,
            0.05,
            2.0,
        );
        assert!(matches!(trans_default, FsmTransition::NoChange));

        let trans_loose = step_fsm(
            &state,
            Vec3::new(0.0, 1.0, 0.0),
            Some(Vec3::new(0.0, 1.08, 0.0)),
            0.016,
            None,
            Hand::Right,
            0.10,
            2.0,
        );
        assert!(matches!(trans_loose, FsmTransition::Attach { ref target } if target == "mug"));
    }
}
