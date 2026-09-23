//! Surface traversal policy, kept separate from the ordinary action controller.
use super::*;
use crate::cc_render::office::{
    motion_timeline::{MotionTimeline, RootKeyframe},
    support::{self, StandingSurface, ANKLE_HEIGHT, ROOT_HEIGHT},
};

#[derive(Clone, Debug)]
pub(super) struct SurfaceJump {
    pub support: StandingSurface,
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub cancelled: bool,
}
impl SurfaceJump {
    fn landing_ms(&self, start: u64) -> u64 {
        start + 1000 + support::flight_duration(self.from.into(), self.to.into())
    }
}

impl OfficeController {
    pub(super) fn surface_owns_motion(&self) -> bool {
        self.standing_surface.is_some() || self.active.iter().any(|a| a.jump.is_some())
    }

    pub(super) fn advance_surface_action(&mut self, mut action: Active) -> bool {
        let phase_limit_ms = match action.stage {
            Stage::Approach => 20_000,
            Stage::Align => 5_000,
            _ => 6_000,
        };
        if self.observed.displayed_pts_ms.saturating_sub(action.execute_pts_ms) > phase_limit_ms {
            self.event(&action.request.action_id, Status::Failed,
                "surface transition did not reach observed support/clearance in time");
            return true;
        }
        let (done, changed) = self.tick_jump(&mut action);
        if done {
            if action.request.verb == Verb::JumpDown {
                self.surface_recovery_at_ms = Some(self.observed.displayed_pts_ms);
            }
            let status = if action.jump.as_ref().unwrap().cancelled {
                Status::Cancelled
            } else {
                Status::Completed
            };
            self.event(&action.request.action_id, status, "observed landing");
        } else {
            self.active.push(action);
        }
        changed
    }

    pub(super) fn prepare_surface_action(
        &self,
        r: &ActionRequest,
    ) -> Result<Option<SurfaceJump>, String> {
        if self.standing_surface.is_some()
            && !matches!(
                r.verb,
                Verb::JumpDown | Verb::Wave | Verb::Smile | Verb::StandUp
            )
        {
            return Err("use jump_down before a floor action".into());
        }
        if !matches!(r.verb, Verb::JumpOnto | Verb::JumpDown) {
            return Ok(None);
        }
        if self.seated.is_some() || !self.held.is_empty() || !self.timeline_layers.is_empty() {
            return Err("jump requires standing with free hands and no timeline edits".into());
        }
        let down = r.verb == Verb::JumpDown;
        let support = if down {
            let support = self
                .standing_surface
                .clone()
                .ok_or("avatar is not standing on a surface")?;
            if r.target.as_ref().is_some_and(|id| id != &support.object_id) {
                return Err("jump_down target is not the current support".into());
            }
            if !support::stance_clear(
                support.floor_position.into(),
                &support::boxes(&self.objects),
            ) {
                return Err("floor landing area is obstructed".into());
            }
            support
        } else {
            support::discover(
                &self.objects,
                r.target.as_deref().ok_or("jump_onto needs a target")?,
            )?
        };
        if !down {
            let floor = support.floor_position;
            super::super::locomotion::approach_path(
                &self.navigation_scene(),
                Vec2::new(self.observed.pelvis[0], self.observed.pelvis[2]),
                Vec2::new(floor[0], floor[2]),
                Some(Vec2::new(support.heading.sin(), support.heading.cos())),
                super::super::locomotion::arrival_angle(),
            )
            .map_err(|error| format!("jump takeoff is unreachable: {error}"))?;
        }
        let (from, to) = if down {
            (support.position, support.floor_position)
        } else {
            (support.floor_position, support.position)
        };
        Ok(Some(SurfaceJump {
            support,
            from,
            to,
            cancelled: false,
        }))
    }

    pub(super) fn defer_jump_cancel(&mut self, id: &str) -> bool {
        let Some(action) = self
            .active
            .iter_mut()
            .find(|a| a.request.action_id == id && a.stage == Stage::Execute)
        else {
            return false;
        };
        let Some(jump) = action.jump.as_mut() else {
            return false;
        };
        jump.cancelled = true;
        true
    }

    /// Returns (completed, needs a new generation revision). Flight and landing
    /// use one timeline; no phase switch, IK lock, root warp or physics impulse.
    pub(super) fn tick_jump(&mut self, action: &mut Active) -> (bool, bool) {
        let jump = action.jump.as_ref().unwrap();
        let observed = &self.observed;
        if action.stage == Stage::Approach {
            let goal = jump.from;
            if Vec2::new(observed.pelvis[0] - goal[0], observed.pelvis[2] - goal[2]).length() > 0.16
            {
                return (false, false);
            }
            action.stage = Stage::Align;
            action.execute_pts_ms = self.motion_start().0;
            action.execute_origin = observed.pelvis;
            action.execute_heading = observed.heading;
            return (false, true);
        }
        if action.stage == Stage::Align {
            let aligned = (observed.heading - jump.support.heading).cos() > 0.18_f32.cos()
                && observed.torso_up[1] > 0.93
                && observed.displayed_pts_ms
                    >= action.execute_pts_ms
                        + heading_turn_ms(action.execute_heading, jump.support.heading)
                        + 500;
            if !aligned {
                action.stable_since_ms = None;
                return (false, false);
            }
            let facing_since = *action
                .stable_since_ms
                .get_or_insert(observed.displayed_pts_ms);
            if observed.displayed_pts_ms.saturating_sub(facing_since) < 300 {
                return (false, false);
            }
            action.stable_since_ms = None;
            action.stage = Stage::Execute;
            action.execute_pts_ms = self.motion_start().0;
            action.jump.as_mut().unwrap().from = [observed.pelvis[0], 0., observed.pelvis[2]];
            return (false, true);
        }
        action.moved |= Vec2::new(
            observed.pelvis[0] - jump.from[0],
            observed.pelvis[2] - jump.from[2],
        )
        .length()
            > 0.2
            || observed
                .feet
                .iter()
                .any(|foot| foot[1] > jump.from[1] + ANKLE_HEIGHT + 0.12);
        let landed = observed.displayed_pts_ms >= jump.landing_ms(action.execute_pts_ms)
            && action.moved
            && observed.torso_up[1] > 0.90
            && (observed.pelvis[1] - jump.to[1] - ROOT_HEIGHT).abs() < 0.22
            && observed.feet.iter().all(|foot| {
                (foot[1] - jump.to[1] - ANKLE_HEIGHT).abs() < 0.16
                    && Vec2::new(foot[0] - jump.to[0], foot[2] - jump.to[2]).length() < 0.38
            })
            && (action.request.verb == Verb::JumpDown
                || support::feet_inside_patch(&observed.feet, &jump.support, &self.objects));
        if !landed {
            action.stable_since_ms = None;
            return (false, false);
        }
        let settled = observed.displayed_pts_ms.saturating_sub(
            *action
                .stable_since_ms
                .get_or_insert(observed.displayed_pts_ms),
        ) >= 300;
        if settled {
            self.standing_surface =
                (action.request.verb == Verb::JumpOnto).then(|| jump.support.clone());
        }
        (settled, settled)
    }

    pub(super) fn apply_surface_motion(&mut self, timeline: &mut MotionTimeline) {
        if let Some(action) = self.active.iter().find(|a| a.jump.is_some()) {
            let jump = action.jump.as_ref().unwrap();
            if action.stage == Stage::Approach {
                return;
            }
            timeline.root.heading = Some(jump.support.heading);
            timeline.root.face_path = false;
            timeline.root.heading_interval_ms = 0;
            timeline.contacts.clear();
            timeline.pose_keyframes.clear();
            self.motion.joints.clear();
            if action.stage == Stage::Align {
                timeline.root.heading_transition =
                    Some(super::super::motion_timeline::HeadingTransition {
                        start_ms: action.execute_pts_ms,
                        duration_ms: heading_turn_ms(action.execute_heading, jump.support.heading),
                        from: action.execute_heading,
                    });
                timeline.root.constrain_height = true;
                timeline.root.keyframes = vec![RootKeyframe {
                    pts_ms: action.execute_pts_ms as i64,
                    position: [jump.from[0], ROOT_HEIGHT, jump.from[2]],
                }];
                {
                    // A bent walking history can otherwise linger at takeoff
                    // for tens of seconds. Sample the upright destination
                    // from ARDY, not from authored body rotations, and let
                    // ARDY generate the whole standing transition.
                    let start = action.execute_pts_ms;
                    timeline.prompts = vec![super::super::motion_timeline::PromptSpan {
                        start_ms: start, end_ms: None,
                        text: "A person comes to a relaxed upright stand with both feet planted, facing the low platform.".into(),
                    }];
                    timeline.pose_keyframes = vec![
                        serde_json::json!({"kind":"generated_posture", "posture":"standing",
                            "pts_ms":start + 1500, "visible_from_ms":start,
                            "heading":jump.support.heading}),
                        serde_json::json!({"kind":"observed_support", "pts_ms":start + 1900}),
                    ];
                }
                return;
            }
            let start = action.execute_pts_ms;
            let land = jump.landing_ms(start);
            let root = |point: [f32; 3]| [point[0], point[1] + ROOT_HEIGHT, point[2]];
            timeline.root.position_mode = "keyframes".into();
            timeline.root.constrain_height = false;
            timeline.root.heading_transition = None;
            timeline.root.keyframes = [
                (start, jump.from),
                (start + 1000, jump.from),
                (land, jump.to),
                (land + 4000, jump.to),
            ]
            .into_iter()
            .map(|(pts_ms, position)| RootKeyframe {
                pts_ms: pts_ms as i64,
                position: root(position),
            })
            .collect();
            timeline.pose_keyframes = vec![
                support::foot_key(jump.from, jump.support.heading, start, Some(start + 1000)),
                support::foot_key(jump.to, jump.support.heading, land, None),
            ];
            let (settle_delay, support_delay) = if action.request.verb == Verb::JumpOnto {
                (150, 550)
            } else {
                (650, 1050)
            };
            timeline.pose_keyframes.extend([
                serde_json::json!({"kind":"generated_posture", "posture":"standing",
                    "pts_ms":land + settle_delay, "visible_from_ms":start + 800,
                    "heading":jump.support.heading}),
                serde_json::json!({"kind":"observed_support", "pts_ms":land + support_delay}),
            ]);
            let text = if action.request.verb == Verb::JumpOnto {
                "A person stands facing a low platform, bends their knees and jumps forward up onto it with both feet, lands on both feet and stands upright on top."
            } else {
                "A person stands upright on a low platform, bends their knees and jumps backward down to the floor, lands on both feet and stands upright."
            };
            timeline.prompts = vec![
                super::super::motion_timeline::PromptSpan {
                    start_ms: start, end_ms: Some(land), text: text.into(),
                },
                super::super::motion_timeline::PromptSpan {
                    start_ms: land, end_ms: None,
                    text: if action.request.verb == Verb::JumpOnto {
                        "A person stands upright on the platform with both feet planted, shoulders relaxed and head up."
                    } else {
                        "A person stands upright on the floor with both feet planted, shoulders relaxed and head up."
                    }.into(),
                },
            ];
            self.motion.prompt = text.into();
            self.motion.constrain_root_height = false;
            self.motion.heading = timeline.root.heading;
        } else if let Some(surface) = &self.standing_surface {
            if !timeline.pose_keyframes.iter().any(|key| key["kind"] == "observed_support") {
                timeline.pose_keyframes.push(support::foot_key(
                    surface.position,
                    surface.heading,
                    self.motion_start().0,
                    None,
                ));
            }
        }
    }

    /// During traversal accept brief target-furniture clipping, but keep a 3D
    /// swept torso envelope against unrelated furniture and walls. While resting,
    /// only collider parts below the feet count as support, not the backrest.
    pub(super) fn permits_surface_root(&self, position: Vec3) -> Option<bool> {
        let flight = self
            .active
            .iter()
            .find(|a| a.jump.is_some() && a.stage == Stage::Execute)
            .and_then(|a| a.jump.as_ref());
        let surface = flight
            .map(|j| &j.support)
            .or(self.standing_surface.as_ref())?;
        let start = Vec3::from(self.observed.pelvis);
        let floor = if flight.is_some() {
            0.
        } else {
            surface.position[1]
        };
        if !position.is_finite() || position.y < floor + 0.4 || position.y > floor + 2.8 {
            return Some(false);
        }
        let colliders = support::boxes(&self.objects);
        Some(
            !colliders
                .iter()
                .filter(|b| !self.held.iter().any(|(_, id)| id == &b.owner))
                .any(|b| {
                    if b.owner == surface.object_id
                        && (flight.is_some() || b.center.y + b.half.y <= floor + 0.12)
                    {
                        return false;
                    }
                    // Three overlapping swept spheres cover a standing torso envelope.
                    [-0.45, 0., 0.45].iter().any(|offset| {
                        b.intersects(
                            start + Vec3::Y * *offset,
                            position + Vec3::Y * *offset,
                            0.23,
                        )
                    })
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> SceneState {
        SceneState::default()
    }
    fn submit(c: &mut OfficeController, id: &str, verb: Verb, target: Option<&str>) {
        let command = serde_json::from_value(serde_json::json!({
            "kind":"perform_action", "action_id":id, "verb":verb, "target":target,
        }))
        .unwrap();
        c.command(command, &world());
    }
    fn at_takeoff() -> OfficeController {
        let mut c = OfficeController::default();
        let surface = support::discover(&c.objects, "living_sofa").unwrap();
        c.observed.ready = true;
        c.observed.position_valid = true;
        c.observed.heading = surface.heading;
        c.observed.pelvis = [
            surface.floor_position[0],
            ROOT_HEIGHT,
            surface.floor_position[2],
        ];
        c.observed.torso_up = [0., 1., 0.];
        submit(&mut c, "up", Verb::JumpOnto, Some("living_sofa"));
        c
    }
    fn jump_ready() -> OfficeController {
        let mut c = at_takeoff();
        c.observed.displayed_pts_ms = 1500;
        c.advance(&world());
        assert_eq!(c.active[0].stage, Stage::Align);
        c.observed.displayed_pts_ms = 1800;
        c.advance(&world());
        assert_eq!(c.active[0].stage, Stage::Execute);
        c
    }
    fn land(c: &mut OfficeController) {
        let a = &c.active[0];
        let jump = a.jump.as_ref().unwrap();
        let point = jump.to;
        let stamp = jump.landing_ms(a.execute_pts_ms);
        c.active[0].moved = true;
        c.observed.displayed_pts_ms = stamp;
        c.observed.pelvis = [point[0], point[1] + ROOT_HEIGHT, point[2]];
        c.observed.feet = [
            [point[0] + 0.12, point[1] + ANKLE_HEIGHT, point[2]],
            [point[0] - 0.12, point[1] + ANKLE_HEIGHT, point[2]],
        ];
        c.advance(&world());
        assert_eq!(
            c.active.len(),
            1,
            "contact requires sustained observed landing"
        );
        c.observed.displayed_pts_ms += 350;
        c.advance(&world());
    }
    #[test]
    fn final_approach_faces_landing_instead_of_arriving_sideways() {
        let mut c = OfficeController::default();
        c.observed.ready = true;
        c.observed.pelvis = [0., ROOT_HEIGHT, 0.];
        submit(&mut c, "up", Verb::JumpOnto, Some("living_sofa"));
        assert_eq!(c.active[0].stage, Stage::Approach);
        let route = &c.motion.timeline.as_ref().unwrap().root.keyframes;
        let last = route[route.len() - 1].position;
        let previous = route[route.len() - 2].position;
        let arrival = Vec2::new(last[0] - previous[0], last[2] - previous[2]).normalize();
        let facing = c.active[0].approach_direction(None).unwrap();
        assert!(
            arrival.dot(facing) >= super::super::super::locomotion::arrival_angle().cos() - 1e-4
        );
    }

    #[test]
    fn flight_prompt_changes_to_upright_pose_at_observed_landing_window() {
        let c = jump_ready();
        let timeline = c.motion.timeline.as_ref().unwrap();
        let jump = c.active[0].jump.as_ref().unwrap();
        let land = jump.landing_ms(c.active[0].execute_pts_ms);
        assert_eq!(timeline.prompts[0].end_ms, Some(land));
        assert_eq!(timeline.prompts[1].start_ms, land);
        assert!(timeline.prompts[1].text.contains("head up"));
        assert!(timeline.pose_keyframes.iter().any(|key| key["kind"] == "generated_posture"
            && key["posture"] == "standing" && key["pts_ms"] == land + 150));
        assert!(!timeline.pose_keyframes.iter().any(|key| key["kind"] == "full_body"));
    }

    #[test]
    fn elevated_idle_guides_whole_generated_pose_before_history_foot_support() {
        let mut c = jump_ready();
        land(&mut c);
        assert!(c.standing_surface.is_some());
        let timeline = c.motion.timeline.as_ref().unwrap();
        assert_eq!(timeline.root.position_mode, "keyframes");
        assert!(!timeline.root.constrain_height);
        assert!(timeline.root.keyframes[0].position[1] > 1.5);
        assert_eq!(timeline.pose_keyframes.iter().filter(|key|
            key["kind"] == "generated_posture" && key["posture"] == "standing").count(), 1);
        assert_eq!(timeline.pose_keyframes.iter().filter(|key|
            key["kind"] == "observed_support").count(), 1);
        assert!(!timeline.pose_keyframes.iter().any(|key| key["kind"] == "full_body"));
        assert!(!timeline.pose_keyframes.iter().any(|key| key["kind"] == "end_effectors"));
    }

    #[test]
    fn bent_takeoff_uses_model_generated_standing_destination() {
        let mut c = at_takeoff();
        c.observed.torso_up = [0.5, 0.85, 0.];
        c.rebuild(&world());
        let keys = &c.motion.timeline.as_ref().unwrap().pose_keyframes;
        assert!(keys.iter().any(|key| key["kind"] == "generated_posture"
            && key["posture"] == "standing"));
        assert!(keys.iter().any(|key| key["kind"] == "observed_support"));
        assert!(!keys.iter().any(|key| key["kind"] == "full_body"));
    }

    #[test]
    fn unsafe_jump_alignment_fails_instead_of_hanging() {
        let mut c = at_takeoff();
        assert_eq!(c.active[0].stage, Stage::Align);
        c.observed.torso_up = [0.5, 0.85, 0.];
        c.observed.displayed_pts_ms = c.active[0].execute_pts_ms + 5_050;
        c.advance(&world());
        assert!(c.active.is_empty());
        assert_eq!(c.events.back().unwrap().status, Status::Failed);
        assert!(c.events.back().unwrap().detail.contains("support/clearance"));
    }

    #[test]
    fn jump_waits_for_sustained_facing_and_resets_if_avatar_turns_away() {
        let mut c = at_takeoff();
        for (stamp, heading) in [
            (3000, 1.57),
            (3050, 0.),
            (3100, 1.57),
            (3300, 0.),
            (3500, 0.),
        ] {
            c.observed.displayed_pts_ms = stamp;
            c.observed.heading = heading;
            c.advance(&world());
            assert_eq!(c.active[0].stage, Stage::Align);
        }
        c.observed.displayed_pts_ms = 3600;
        c.advance(&world());
        assert_eq!(c.active[0].stage, Stage::Execute);
        assert_eq!(c.active[0].execute_pts_ms, 3600);
        assert!(c.active[0].stable_since_ms.is_none());
    }

    #[test]
    fn future_landing_is_sparse_and_not_an_early_renderer_foot_lock() {
        let mut c = jump_ready();
        let timeline = c.motion.timeline.as_ref().unwrap();
        assert_eq!(timeline.root.position_mode, "keyframes");
        assert!(!timeline.root.constrain_height);
        assert!(timeline.contacts.is_empty() && c.motion.joints.is_empty());
        assert_eq!(timeline.pose_keyframes.len(), 4);
        assert_eq!(timeline.pose_keyframes[1]["pts_ms"], 3650);
        assert_eq!(timeline.pose_keyframes[2]["kind"], "generated_posture");
        assert_eq!(timeline.pose_keyframes[3]["kind"], "observed_support");
        c.observed.displayed_pts_ms = 6000;
        c.advance(&world());
        assert_eq!(
            c.active.len(),
            1,
            "generation time alone must not complete a jump"
        );
    }
    #[test]
    fn observed_landing_retains_elevation_then_jump_down_releases_it() {
        let mut c = jump_ready();
        land(&mut c);
        assert!(c.active.is_empty());
        assert_eq!(c.snapshot().posture, "standing_on_surface");
        assert!(c.motion.timeline.as_ref().unwrap().root.keyframes[0].position[1] > 1.5);
        assert!(
            c.motion
                .timeline
                .as_ref()
                .unwrap()
                .pose_keyframes
                .iter()
                .all(|key| key["preset"] != "standing"),
            "floor-based full-body preset cannot hold elevated feet"
        );
        assert!(c.permits_root(c.observed.pelvis.into()));
        submit(&mut c, "walk", Verb::Walk, Some("desk"));
        assert_eq!(c.events.back().unwrap().status, Status::Failed);
        submit(&mut c, "down", Verb::JumpDown, None);
        land(&mut c);
        assert!(c.standing_surface.is_none());
        assert_eq!(c.events.back().unwrap().status, Status::Completed);
        let keys = &c.motion.timeline.as_ref().unwrap().pose_keyframes;
        assert!(keys.iter().any(|key| key["kind"] == "generated_posture"
            && key["posture"] == "standing"));
        assert!(keys.iter().any(|key| key["kind"] == "observed_support"));
        assert!(!keys.iter().any(|key| key["preset"] == "standing"));
    }
    #[test]
    fn cancellation_finishes_landing_and_unrelated_obstacles_still_block() {
        let mut c = jump_ready();
        let jump = c.active[0].jump.as_ref().unwrap();
        let target = Vec3::from(jump.to) + Vec3::Y * ROOT_HEIGHT;
        assert!(c.permits_root(target));
        assert!(!c.permits_root(Vec3::new(30., 1.5, 30.)));
        c.command(
            OfficeCommand::CancelAction {
                action_id: "up".into(),
            },
            &world(),
        );
        assert_eq!(c.active.len(), 1);
        land(&mut c);
        assert_eq!(c.events.back().unwrap().status, Status::Cancelled);
        assert!(c.standing_surface.is_some());
    }
}
