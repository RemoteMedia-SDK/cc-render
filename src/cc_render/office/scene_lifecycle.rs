//! Observed outcome lifecycle only. Never emits poses, paths, constraints or
//! time-indexed body targets. A phase change requires a new model revision.
use super::{controller::OfficeSnapshot, motion_intent::{MotionGoal, SceneOperation}};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase { Arrive, Occupy, Contact, Holding, Settle, Declared }
impl Phase {
    pub(crate) fn as_str(self) -> &'static str {
        match self { Self::Arrive => "arrive", Self::Occupy => "occupy", Self::Contact => "contact", Self::Holding => "holding", Self::Settle => "settle", Self::Declared => "declared" }
    }
}
/// Navigation completes inside the approach region once the pelvis is slower than this.
pub(crate) const ARRIVAL_SPEED_M_S: f32 = 0.6;

#[derive(Debug, PartialEq)]
pub(crate) enum Update { None, PhaseChanged, Completed, Failed(String) }

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SceneLifecycle {
    pub goal: MotionGoal,
    pub phase: Phase,
    pub complete: bool,
    pub superseded: bool,
    pub pending_phase: Option<Phase>,
    /// Declared interaction progress and per-phase results (`interact` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared: Option<super::declared::DeclaredState>,
    #[serde(skip)] awaiting_revision: bool,
    #[serde(skip)] previous: Option<(u64, [f32;3])>,
    #[serde(skip)] stable_since: Option<u64>,
    #[serde(skip)] pursuit_started: Option<u64>,
    #[serde(skip)] required_grips: Vec<(crate::cc_render::Hand, String)>,
}
impl SceneLifecycle {
    pub(crate) fn new(goal: MotionGoal, office: &OfficeSnapshot) -> Result<Self, String> {
        let outcome = goal.scene_outcome().ok_or("scene outcome required")?;
        let object = office.objects.iter().find(|o| o.id == outcome.target_id).ok_or("unknown scene target")?;
        if outcome.operation == SceneOperation::Grasp && office.occupied_hands.iter().any(|(h, id)|
            *h == crate::cc_render::Hand::Right && id != &outcome.target_id) {
            return Err("requested hand already holds another object".into());
        }
        let phase = match outcome.operation {
            SceneOperation::Navigate => Phase::Arrive,
            SceneOperation::Sit if !object.dynamic && office.native_seat_profile.is_some()
                && object.seat_surface_part.and_then(|i| object.parts.get(i)).is_some() => Phase::Contact,
            SceneOperation::Grasp if object.dynamic && (object.grasp.is_some() || object.local_keypoint("grasp").is_some()) => Phase::Contact,
            SceneOperation::Interact => Phase::Declared,
            _ => return Err("unsupported_capability: lifecycle admission not registered for operation".into()),
        };
        let mut required_grips = if outcome.operation == SceneOperation::Grasp { Vec::new() } else { office.occupied_hands.clone() };
        // An interaction may release a grip on its own object (e.g. setting a carried plate down).
        if outcome.operation == SceneOperation::Interact { required_grips.retain(|(_, id)| id != &outcome.target_id); }
        let declared = match &outcome.interaction {
            Some(id) if outcome.operation == SceneOperation::Interact => Some(super::declared::DeclaredState::new(object, id)?),
            _ => None,
        };
        Ok(Self { goal, phase, complete:false, superseded:false, pending_phase:None, declared,
            awaiting_revision:false, previous:None, stable_since:None, pursuit_started:None, required_grips })
    }

    /// Must follow successful scheduling of the new generation. Until then,
    /// old-revision observations cannot establish completion of a new phase.
    pub(crate) fn bind_generation(&mut self, goal: MotionGoal) -> Result<(), String> {
        if !self.awaiting_revision || goal.revision() <= self.goal.revision()
            || goal.session_id() != self.goal.session_id() || goal.scene_outcome() != self.goal.scene_outcome() {
            return Err("invalid lifecycle generation handoff".into());
        }
        self.phase = self.pending_phase.take().ok_or("missing pending lifecycle phase")?;
        if let Some(declared) = self.declared.as_mut() { declared.enter(declared.index + 1); }
        self.goal = goal;
        self.awaiting_revision = false;
        self.previous = None;
        self.stable_since = None;
        Ok(())
    }

    pub(crate) fn awaiting_revision(&self) -> bool { self.awaiting_revision }

    /// Lifecycle phase name sent with each request: the declared phase id for `interact`.
    pub(crate) fn phase_name(&self) -> &str {
        self.declared.as_ref().map_or(self.phase.as_str(), |d| d.phase.as_str())
    }

    /// Coupling changes queued by declared phases, for the controller.
    pub(crate) fn take_couplings(&mut self) -> Vec<super::interaction::CouplingCommand> {
        self.declared.as_mut().map_or_else(Vec::new, |d| std::mem::take(&mut d.couplings))
    }

    /// Arm physics only when this contact revision is actually displayed.
    pub(crate) fn arm_pursuit(&mut self, office: &OfficeSnapshot) -> Option<String> {
        let observation = &office.avatar;
        if self.goal.scene_outcome()?.operation != SceneOperation::Grasp
            || self.superseded || self.phase != Phase::Contact || self.pursuit_started.is_some() || !observation.ready
            || !observation.position_valid || office.motion_error.is_some()
            || !observation.native_stream.as_ref().is_some_and(|s|
                s.session_id == self.goal.session_id() && s.generation_revision == self.goal.revision()) {
            return None;
        }
        let target = &self.goal.scene_outcome()?.target_id;
        self.pursuit_started = Some(observation.displayed_pts_ms);
        if office.occupied_hands.iter().any(|(h, id)| *h == crate::cc_render::Hand::Right && id == target) {
            return None;
        }
        Some(target.clone())
    }

    pub(crate) fn caption(&self, office: &OfficeSnapshot) -> String {
        if let Some(declared) = &self.declared { return declared.prompt().into(); }
        let action = match self.phase {
            Phase::Arrive => "A person walks to a destination",
            Phase::Occupy => "A person stops walking and stands still",
            Phase::Contact if self.goal.scene_outcome().is_some_and(|o| o.operation == SceneOperation::Sit) => "A person sits down on a chair",
            Phase::Contact => "A person picks up an object with their right hand and holds it",
            Phase::Settle => "A person sits on a chair",
            Phase::Holding => "A person holds an object in their right hand",
            Phase::Declared => unreachable!(),
        };
        let carrying = (matches!(self.phase, Phase::Arrive | Phase::Occupy | Phase::Settle)
            || self.goal.scene_outcome().is_some_and(|o| o.operation == SceneOperation::Sit))
            && office.occupied_hands.iter().any(|(hand, _)| *hand == crate::cc_render::Hand::Right);
        format!("{action}{}.", if carrying { " while holding an object in their right hand" } else { "" })
    }

    pub(crate) fn observe(&mut self, office: &OfficeSnapshot) -> Update {
        if let Some(error) = &office.motion_error { return Update::Failed(error.clone()); }
        let observation = &office.avatar;
        if self.superseded || self.awaiting_revision || !observation.ready || !observation.position_valid
            || !observation.native_stream.as_ref().is_some_and(|s|
                s.session_id == self.goal.session_id() && s.generation_revision == self.goal.revision()) {
            return Update::None;
        }
        if self.required_grips.iter().any(|grip| !office.occupied_hands.contains(grip)) {
            return Update::Failed("carried physical grip lost".into());
        }
        if self.complete {
            let outcome = self.goal.scene_outcome().unwrap();
            if outcome.operation == SceneOperation::Interact { return Update::None; }
            if outcome.operation == SceneOperation::Sit && !seat_contact(&self.goal, observation) {
                return Update::Failed("seated geometric contact lost".into());
            }
            if outcome.operation == SceneOperation::Grasp && !office.occupied_hands.iter().any(|(hand,id)|
                *hand == crate::cc_render::Hand::Right && id == &outcome.target_id) {
                return Update::Failed("requested physical grip lost".into());
            }
            return Update::None;
        }
        let pts = observation.displayed_pts_ms;
        if self.previous.is_some_and(|(old, _)| pts <= old) { return Update::None; }
        let outcome = self.goal.scene_outcome().unwrap();
        let Some(object) = office.objects.iter().find(|o| o.id == outcome.target_id) else {
            return Update::Failed("scene target disappeared".into());
        };
        if let Some(declared) = self.declared.as_mut() {
            use super::declared::Step;
            return match declared.observe(office) {
                Step::Running => Update::None,
                Step::PhaseDone => {
                    self.pending_phase = Some(Phase::Declared);
                    self.awaiting_revision = true;
                    Update::PhaseChanged
                }
                Step::Done => { self.complete = true; Update::Completed }
                Step::Failed(error) => Update::Failed(error),
            };
        }
        let position = observation.pelvis;
        let stable = match self.phase {
            Phase::Arrive | Phase::Occupy => {
                // Arrival completes the goal while she is still easing in, so the next
                // step can follow without a separate stand-still revision.
                let region = object.approach_position();
                let inside = ((position[0]-region[0]).powi(2)+(position[2]-region[2]).powi(2)).sqrt() <= 0.25;
                let easing = self.previous.is_some_and(|(old, p)| {
                    let distance = ((position[0]-p[0]).powi(2)+(position[2]-p[2]).powi(2)).sqrt();
                    distance * 1000. / (pts-old) as f32 <= ARRIVAL_SPEED_M_S
                });
                if inside && easing {
                    self.complete = true;
                    return Update::Completed;
                }
                false
            }
            Phase::Contact | Phase::Settle if outcome.operation == SceneOperation::Sit => {
                let contact = seat_contact(&self.goal, observation);
                if self.phase == Phase::Contact && contact {
                    self.pending_phase = Some(Phase::Settle);
                    self.awaiting_revision = true;
                    return Update::PhaseChanged;
                }
                contact && self.previous.is_some_and(|(old, p)| {
                    let distance = position.iter().zip(p).map(|(a,b)| (a-b).powi(2)).sum::<f32>().sqrt();
                    distance * 1000. / (pts-old) as f32 <= 0.04
                })
            }
            Phase::Settle | Phase::Declared => return Update::Failed("invalid seat lifecycle operation".into()),
            Phase::Contact | Phase::Holding => {
                // Exact requested-object, right-hand physical grip; neither
                // wrist proximity nor a model's claimed contact counts.
                let held = office.occupied_hands.iter().any(|(hand, id)|
                    *hand == crate::cc_render::Hand::Right && id == &outcome.target_id);
                if self.phase == Phase::Contact && held {
                    // The physical grip is the outcome; holding on is checked from here on.
                    self.complete = true;
                    return Update::Completed;
                }
                if self.phase == Phase::Contact && self.pursuit_started.is_some_and(|at| pts.saturating_sub(at) >= 2200) {
                    return Update::Failed("requested contact grip was not acquired".into());
                }
                if self.phase == Phase::Holding && !held {
                    return Update::Failed("requested physical grip lost".into());
                }
                held
            }
        };
        self.previous = Some((pts, position));
        if !stable { self.stable_since = None; return Update::None; }
        if pts.saturating_sub(*self.stable_since.get_or_insert(pts)) < 300 { return Update::None; }
        self.complete = true;
        Update::Completed
    }
}

fn seat_contact(goal: &MotionGoal, observation: &super::controller::Observation) -> bool {
    let Some(outcome) = goal.scene_outcome() else { return false; };
    observation.native_seat.as_ref().is_some_and(|e|
        e.session_id == goal.session_id() && e.generation_revision == goal.revision()
        && e.pts_ms == observation.displayed_pts_ms && e.target_id == outcome.target_id
        && (e.collision_warning || e.maximum_penetration_m <= 0.001)
        && e.patch_distances_m.iter().all(|d| *d <= 0.015))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::{controller::OfficeController, timeline::StreamIdentity};
    fn goal(revision: u64) -> MotionGoal {
        serde_json::from_value(serde_json::json!({"kind":"motion_goal","version":3,
            "session_id":"test","generation_revision":revision,"pts_ms":0,"seed":42,
            "goal":{"operation":"navigate","target_id":"chair"}})).unwrap()
    }
    fn interact(revision: u64, target: &str, interaction: &str) -> MotionGoal {
        let mut value = serde_json::to_value(goal(revision)).unwrap();
        value["goal"] = serde_json::json!({"operation":"interact","target_id":target,"interaction":interaction});
        serde_json::from_value(value).unwrap()
    }

    fn displayed(scene: &mut OfficeSnapshot, revision: u64, pts: u64) {
        scene.avatar.ready = true; scene.avatar.position_valid = true;
        scene.avatar.native_stream = Some(StreamIdentity { session_id:"test".into(), generation_revision:revision,
            skeleton_id:"smpl22".into(), root_mode:"world".into() });
        scene.avatar.displayed_pts_ms = pts;
    }

    #[test]
    fn interact_goals_name_a_declared_interaction() {
        let scene = OfficeController::default().snapshot();
        let mut value = serde_json::to_value(goal(1)).unwrap();
        value["goal"] = serde_json::json!({"operation":"interact","target_id":"broom"});
        assert!(serde_json::from_value::<MotionGoal>(value.clone()).is_err(), "interaction is required");
        value["goal"] = serde_json::json!({"operation":"navigate","target_id":"broom","interaction":"pick_up"});
        assert!(serde_json::from_value::<MotionGoal>(value.clone()).is_err(), "only interact names one");
        value["goal"] = serde_json::json!({"operation":"interact","target_id":"broom","interaction":"pick up!"});
        assert!(serde_json::from_value::<MotionGoal>(value).is_err());
        let error = SceneLifecycle::new(interact(1, "broom", "juggle"), &scene).unwrap_err();
        assert!(error.contains("unsupported_capability"), "{error}");
        assert!(SceneLifecycle::new(interact(1, "chair", "pick_up"), &scene).is_err());
    }

    #[test]
    fn the_hands_own_approach_picks_which_grip_each_takes() {
        let mut scene = OfficeController::default().snapshot();
        let mut task = SceneLifecycle::new(interact(1, "broom", "pick_up"), &scene).unwrap();
        scene.avatar.pelvis = [-1.6, 0.9, 1.9];
        scene.avatar.heading = 0.;
        displayed(&mut scene, 1, 0);
        task.observe(&scene);
        displayed(&mut scene, 1, 700);
        assert_eq!(task.observe(&scene), Update::PhaseChanged);
        task.bind_generation(interact(2, "broom", "pick_up")).unwrap();
        assert_eq!(task.phase_name(), "reach");
        // Her hands first come in the declared way round (right high), near but not on the grips...
        scene.avatar.wrists = [[-1.45, 0.75, 2.2], [-1.75, 1.15, 2.2]];
        displayed(&mut scene, 2, 750);
        assert_eq!(task.observe(&scene), Update::None);
        // ...then the LEFT hand takes the high grip and the right the low one: crossed from the
        // declaration, as the motion service's corrections chose. That pairing is accepted.
        scene.avatar.wrists = [[-1.62, 1.15, 2.4], [-1.58, 0.75, 2.4]];
        let mut update = Update::None;
        for pts in [800, 850, 900] {
            displayed(&mut scene, 2, pts);
            update = task.observe(&scene);
        }
        assert_eq!(update, Update::PhaseChanged, "the crossed pairing is accepted, not failed");
        assert_eq!(task.declared.as_ref().unwrap().hands_swapped, Some(true), "reported: the hands crossed the declaration");
        match task.take_couplings().as_slice() {
            [super::super::interaction::CouplingCommand::Attach { anchors, .. }] => {
                // The upper grip went to the left hand, the lower to the right.
                assert_eq!(anchors[0].0, crate::cc_render::Hand::Left);
                assert!((anchors[0].1[1] - 0.49).abs() < 1e-3);
                assert_eq!(anchors[1].0, crate::cc_render::Hand::Right);
                assert!((anchors[1].1[1] - 0.09).abs() < 1e-3);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn broom_pick_up_runs_declared_phases_on_observed_joints_and_attaches_both_hands() {
        let mut scene = OfficeController::default().snapshot();
        let mut task = SceneLifecycle::new(interact(1, "broom", "pick_up"), &scene).unwrap();
        assert_eq!((task.phase_name(), task.caption(&scene).as_str()),
                   ("approach", "A person walks over to a broom standing by the wall."));
        // Approach: the pelvis must reach the stand zone, face the broom and stay there.
        scene.avatar.pelvis = [-1.6, 0.9, 1.9];
        scene.avatar.heading = std::f32::consts::FRAC_PI_2; // there, but facing along the wall
        displayed(&mut scene, 1, 0);
        assert_eq!(task.observe(&scene), Update::None);
        displayed(&mut scene, 1, 600);
        assert_eq!(task.observe(&scene), Update::None);
        scene.avatar.heading = 0.2; // facing the broom (+Z) within 30 degrees
        displayed(&mut scene, 1, 700);
        assert_eq!(task.observe(&scene), Update::None);
        displayed(&mut scene, 1, 1300);
        assert_eq!(task.observe(&scene), Update::PhaseChanged);
        task.bind_generation(interact(2, "broom", "pick_up")).unwrap();
        assert_eq!(task.phase_name(), "reach");
        // Reach: only the right wrist on the upper grip is not enough.
        scene.avatar.wrists = [[-1.0, 1.0, 2.0], [-1.6, 1.15, 2.4]];
        for pts in [700, 750, 800, 850] {
            displayed(&mut scene, 2, pts);
            assert_eq!(task.observe(&scene), Update::None);
        }
        scene.avatar.wrists[0] = [-1.55, 0.75, 2.4];
        let mut update = Update::None;
        for pts in [900, 950, 1000] {
            displayed(&mut scene, 2, pts);
            update = task.observe(&scene);
        }
        assert_eq!(update, Update::PhaseChanged);
        match task.take_couplings().as_slice() {
            [super::super::interaction::CouplingCommand::Attach { object, anchors, .. }] => {
                assert_eq!(object, "broom");
                assert_eq!(anchors[0].0, crate::cc_render::Hand::Right);
                assert_eq!(anchors[1].0, crate::cc_render::Hand::Left);
                assert!((anchors[0].1[1] - 0.49).abs() < 1e-3, "grip latched at the wrist height on the handle");
            }
            other => panic!("expected one attach, got {other:?}"),
        }
        task.bind_generation(interact(3, "broom", "pick_up")).unwrap();
        assert_eq!(task.phase_name(), "lift");
        // Lift: no relation; the motion plays out with the broom in both hands.
        scene.avatar.wrists = [[-1.2, 0.9, 2.0], [-1.6, 1.15, 2.0]];
        let mut update = Update::None;
        for pts in (1100..=3700).step_by(100) {
            displayed(&mut scene, 3, pts);
            update = task.observe(&scene);
            if update != Update::None { break; }
        }
        assert_eq!(update, Update::Completed);
        let results = &task.declared.as_ref().unwrap().results;
        assert_eq!(results.iter().map(|r| (r.phase.as_str(), r.status)).collect::<Vec<_>>(),
                   [("approach", "passed"), ("reach", "passed"), ("lift", "passed")]);
    }

    #[test]
    fn declared_phases_fail_on_timeout_with_per_phase_results() {
        let mut scene = OfficeController::default().snapshot();
        let mut task = SceneLifecycle::new(interact(1, "broom", "pick_up"), &scene).unwrap();
        scene.avatar.pelvis = [3., 0.9, -3.]; // never reaches the broom
        let mut update = Update::None;
        for pts in (0..=41_000).step_by(500) {
            displayed(&mut scene, 1, pts);
            update = task.observe(&scene);
            if update != Update::None { break; }
        }
        assert!(matches!(&update, Update::Failed(e) if e.contains("phase approach timed out")), "{update:?}");
        let result = &task.declared.as_ref().unwrap().results[0];
        assert_eq!((result.phase.as_str(), result.status), ("approach", "failed"));
    }

    #[test]
    fn stepping_up_authorizes_support_at_phase_start_and_needs_both_feet_on_top() {
        let mut scene = OfficeController::default().snapshot();
        let mut task = SceneLifecycle::new(interact(1, "step_stool", "step_up"), &scene).unwrap();
        assert!(task.take_couplings().is_empty());
        scene.avatar.pelvis = [-2.4, 0.9, 0.42];
        displayed(&mut scene, 1, 0);
        task.observe(&scene);
        displayed(&mut scene, 1, 600);
        assert_eq!(task.observe(&scene), Update::PhaseChanged);
        task.bind_generation(interact(2, "step_stool", "step_up")).unwrap();
        assert_eq!(task.take_couplings(),
                   [super::super::interaction::CouplingCommand::SupportOn { object: "step_stool".into() }]);
        // One foot up, one on the floor: not supported.
        scene.avatar.feet = [[-2.45, 0.33, 1.0], [-2.35, 0.08, 0.6]];
        for pts in (700..1500).step_by(50) { displayed(&mut scene, 2, pts); assert_eq!(task.observe(&scene), Update::None); }
        scene.avatar.feet[1] = [-2.35, 0.33, 1.05];
        let mut update = Update::None;
        for pts in (1500..2200).step_by(50) {
            displayed(&mut scene, 2, pts);
            update = task.observe(&scene);
            if update != Update::None { break; }
        }
        assert_eq!(update, Update::Completed);
        assert!(task.take_couplings().is_empty(), "support_on is not repeated when met");
    }

    #[test]
    fn grasp_requires_exact_physical_hand_and_object_and_detects_loss() {
        let mut scene = OfficeController::default().snapshot();
        let target = scene.objects.iter().find(|o| o.dynamic && o.grasp.is_some()).unwrap().id.clone();
        let mut value = serde_json::to_value(goal(1)).unwrap();
        value["goal"] = serde_json::json!({"operation":"grasp","target_id":target});
        let request: MotionGoal = serde_json::from_value(value.clone()).unwrap();
        let mut task = SceneLifecycle::new(request, &scene).unwrap();
        scene.avatar.ready = true; scene.avatar.position_valid = true;
        scene.avatar.native_stream = Some(StreamIdentity {session_id:"test".into(),
            generation_revision:1,skeleton_id:"smpl22".into(),root_mode:"world".into()});
        scene.avatar.native_stream.as_mut().unwrap().generation_revision = 0;
        assert!(task.arm_pursuit(&scene).is_none());
        scene.avatar.native_stream.as_mut().unwrap().generation_revision = 1;
        assert_eq!(task.arm_pursuit(&scene), Some(target.clone()));
        assert!(task.arm_pursuit(&scene).is_none());
        scene.occupied_hands = vec![(crate::cc_render::Hand::Left, target.clone()),
                                   (crate::cc_render::Hand::Right, "other".into())];
        assert_eq!(task.observe(&scene), Update::None);
        scene.avatar.displayed_pts_ms = 50;
        scene.occupied_hands = vec![(crate::cc_render::Hand::Right, target)];
        // The grip is the outcome: no separate holding revision before the next step.
        assert_eq!(task.observe(&scene), Update::Completed);
        assert!(task.complete && task.pending_phase.is_none());
        scene.avatar.displayed_pts_ms = 100;
        scene.occupied_hands.clear();
        assert_eq!(task.observe(&scene), Update::Failed("requested physical grip lost".into()));
    }

    #[test]
    fn navigation_completes_on_displayed_arrival_while_easing_in() {
        let mut scene = OfficeController::default().snapshot();
        scene.avatar.ready = true; scene.avatar.position_valid = true;
        let arrival = scene.objects.iter().find(|o| o.id == "chair").unwrap().approach_position();
        scene.avatar.pelvis = [arrival[0], arrival[1], arrival[2] + 0.6];
        let mut task = SceneLifecycle::new(goal(1), &scene).unwrap();
        assert_eq!(task.observe(&scene), Update::None); // No displayed native evidence.
        scene.avatar.native_stream = Some(StreamIdentity {session_id:"test".into(),
            generation_revision:1,skeleton_id:"smpl22".into(),root_mode:"world".into()});
        assert_eq!(task.observe(&scene), Update::None); // Outside the approach region.
        // Striding through the region at 1.2 m/s does not count as arriving.
        scene.avatar.pelvis = [arrival[0], arrival[1], arrival[2] + 0.12];
        scene.avatar.displayed_pts_ms = 400;
        assert_eq!(task.observe(&scene), Update::None);
        // Easing in (0.4 m/s) inside the region completes, with no stand-still revision.
        scene.avatar.pelvis = [arrival[0], arrival[1], arrival[2] + 0.04];
        scene.avatar.displayed_pts_ms = 600;
        assert_eq!(task.observe(&scene), Update::Completed);
        assert!(task.complete && task.pending_phase.is_none());
    }

    #[test]
    fn sitting_requires_displayed_identity_bound_contact_and_retains_carried_grip() {
        let mut scene = OfficeController::default().snapshot();
        scene.native_seat_profile = Some("profile".into());
        scene.occupied_hands = vec![(crate::cc_render::Hand::Right, "mug".into())];
        let mut value = serde_json::to_value(goal(1)).unwrap();
        value["goal"]["operation"] = "sit".into();
        let mut task = SceneLifecycle::new(serde_json::from_value(value.clone()).unwrap(), &scene).unwrap();
        scene.avatar.ready = true; scene.avatar.position_valid = true;
        scene.avatar.native_stream = Some(StreamIdentity { session_id:"test".into(),generation_revision:1,
            skeleton_id:"smpl22".into(),root_mode:"world".into() });
        scene.avatar.native_seat = Some(super::super::native_seat::SeatObservation {
            session_id:"test".into(),generation_revision:2,target_id:"chair".into(),pts_ms:0,
            patch_distances_m:[0.001,0.001],maximum_penetration_m:0.,collision_warning:false });
        assert!(task.arm_pursuit(&scene).is_none());
        assert_eq!(task.observe(&scene), Update::None);
        scene.avatar.displayed_pts_ms = 50;
        let e = scene.avatar.native_seat.as_mut().unwrap(); e.generation_revision = 1; e.pts_ms = 50;
        assert_eq!(task.observe(&scene), Update::PhaseChanged);
        value["generation_revision"] = 2.into();
        task.bind_generation(serde_json::from_value(value).unwrap()).unwrap();
        scene.avatar.displayed_pts_ms = 150;
        assert_eq!(task.observe(&scene), Update::None);
        scene.avatar.native_stream.as_mut().unwrap().generation_revision = 2;
        assert_eq!(task.observe(&scene), Update::None); // Old geometric evidence cannot complete.
        for pts in [200,300,400,500] {
            scene.avatar.displayed_pts_ms = pts;
            let e = scene.avatar.native_seat.as_mut().unwrap(); e.generation_revision = 2; e.pts_ms = pts;
            assert_eq!(task.observe(&scene), if pts == 500 { Update::Completed } else { Update::None });
        }
        scene.avatar.displayed_pts_ms = 525;
        let evidence = scene.avatar.native_seat.as_mut().unwrap();
        evidence.pts_ms = 525;
        evidence.maximum_penetration_m = 0.004;
        evidence.collision_warning = true;
        assert_eq!(task.observe(&scene), Update::None); // Overlap warning cannot revoke completion.
        scene.occupied_hands.clear();
        assert_eq!(task.observe(&scene), Update::Failed("carried physical grip lost".into()));
        scene.occupied_hands = vec![(crate::cc_render::Hand::Right, "mug".into())];
        scene.avatar.displayed_pts_ms = 550;
        let evidence = scene.avatar.native_seat.as_mut().unwrap();
        evidence.pts_ms = 550;
        evidence.patch_distances_m[0] = 0.02;
        assert_eq!(task.observe(&scene), Update::Failed("seated geometric contact lost".into()));
    }
}
