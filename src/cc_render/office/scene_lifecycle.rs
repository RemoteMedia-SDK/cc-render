//! Observed outcome lifecycle only. Never emits poses, paths, constraints or
//! time-indexed body targets. A phase change requires a new model revision.
use super::{controller::OfficeSnapshot, motion_intent::{MotionGoal, SceneOperation}};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase { Arrive, Occupy, Contact, Holding, Settle }
impl Phase {
    pub(crate) fn as_str(self) -> &'static str {
        match self { Self::Arrive => "arrive", Self::Occupy => "occupy", Self::Contact => "contact", Self::Holding => "holding", Self::Settle => "settle" }
    }
}
#[derive(Debug, PartialEq)]
pub(crate) enum Update { None, PhaseChanged, Completed, Failed(String) }

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SceneLifecycle {
    pub goal: MotionGoal,
    pub phase: Phase,
    pub complete: bool,
    pub superseded: bool,
    pub pending_phase: Option<Phase>,
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
            _ => return Err("unsupported_capability: lifecycle admission not registered for operation".into()),
        };
        let required_grips = if outcome.operation == SceneOperation::Grasp { Vec::new() } else { office.occupied_hands.clone() };
        Ok(Self { goal, phase, complete:false, superseded:false, pending_phase:None,
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
        self.goal = goal;
        self.awaiting_revision = false;
        self.previous = None;
        self.stable_since = None;
        Ok(())
    }

    pub(crate) fn awaiting_revision(&self) -> bool { self.awaiting_revision }

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
        let action = match self.phase {
            Phase::Arrive => "A person walks to a destination",
            Phase::Occupy => "A person stops walking and stands still",
            Phase::Contact if self.goal.scene_outcome().is_some_and(|o| o.operation == SceneOperation::Sit) => "A person sits down on a chair",
            Phase::Contact => "A person picks up an object with their right hand and holds it",
            Phase::Settle => "A person sits on a chair",
            Phase::Holding => "A person holds an object in their right hand",
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
        let position = observation.pelvis;
        let stable = match self.phase {
            Phase::Arrive | Phase::Occupy => {
                let region = object.approach_position();
                let inside = ((position[0]-region[0]).powi(2)+(position[2]-region[2]).powi(2)).sqrt() <= 0.25;
                if self.phase == Phase::Arrive && inside {
                    self.pending_phase = Some(Phase::Occupy);
                    self.awaiting_revision = true;
                    return Update::PhaseChanged;
                }
                inside && self.previous.is_some_and(|(old, p)| {
                    let distance = ((position[0]-p[0]).powi(2)+(position[2]-p[2]).powi(2)).sqrt();
                    distance * 1000. / (pts-old) as f32 <= 0.04
                })
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
            Phase::Settle => return Update::Failed("invalid seat lifecycle operation".into()),
            Phase::Contact | Phase::Holding => {
                // Exact requested-object, right-hand physical grip; neither
                // wrist proximity nor a model's claimed contact counts.
                let held = office.occupied_hands.iter().any(|(hand, id)|
                    *hand == crate::cc_render::Hand::Right && id == &outcome.target_id);
                if self.phase == Phase::Contact && held {
                    self.pending_phase = Some(Phase::Holding);
                    self.awaiting_revision = true;
                    return Update::PhaseChanged;
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
        assert_eq!(task.observe(&scene), Update::PhaseChanged);
        assert_eq!(task.phase, Phase::Contact); // Keep old generation policy until handoff.
        assert_eq!(task.pending_phase, Some(Phase::Holding));
        value["generation_revision"] = 2.into();
        task.bind_generation(serde_json::from_value(value).unwrap()).unwrap();
        scene.avatar.native_stream.as_mut().unwrap().generation_revision = 2;
        scene.occupied_hands.clear();
        assert_eq!(task.observe(&scene), Update::Failed("requested physical grip lost".into()));
        assert!(!task.complete);
    }

    #[test]
    fn completion_requires_new_displayed_revision_and_sustained_observations() {
        let mut scene = OfficeController::default().snapshot();
        scene.avatar.ready = true; scene.avatar.position_valid = true;
        scene.avatar.pelvis = scene.objects.iter().find(|o| o.id == "chair").unwrap().approach_position();
        let mut task = SceneLifecycle::new(goal(1), &scene).unwrap();
        assert_eq!(task.observe(&scene), Update::None); // No displayed native evidence.
        scene.avatar.native_stream = Some(StreamIdentity {session_id:"test".into(),
            generation_revision:1,skeleton_id:"smpl22".into(),root_mode:"world".into()});
        assert_eq!(task.observe(&scene), Update::PhaseChanged);
        scene.avatar.displayed_pts_ms = 1000;
        assert_eq!(task.observe(&scene), Update::None); // Not a timer-driven phase.
        task.bind_generation(goal(2)).unwrap();
        assert_eq!(task.observe(&scene), Update::None); // Old pose still displayed.
        scene.avatar.native_stream.as_mut().unwrap().generation_revision = 2;
        for pts in [1000, 1100, 1200, 1300] {
            scene.avatar.displayed_pts_ms = pts;
            assert_eq!(task.observe(&scene), Update::None);
        }
        scene.avatar.displayed_pts_ms = 1400;
        assert_eq!(task.observe(&scene), Update::Completed);
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
