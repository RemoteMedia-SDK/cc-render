//! Closed goal-only protocol. No body channels or arbitrary extension maps.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "Wire")]
pub struct MotionGoal {
    #[serde(flatten)]
    wire: Wire,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    kind: Kind,
    version: u8,
    session_id: String,
    generation_revision: u64,
    pts_ms: u64,
    seed: u64,
    goal: Goal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Kind {
    #[serde(rename = "motion_goal")]
    MotionGoal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticGoal {
    description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneOperation { Navigate, Grasp, Sit }

/// Walking speed hint for planned navigation (0.6, 1.0 or 1.4 m/s).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pace { Slow, Normal, Brisk }

/// Longest accepted caller `style` (plain-text manner of movement).
pub const MAX_STYLE_CHARS: usize = 160;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneOutcome {
    pub operation: SceneOperation,
    pub target_id: String,
    /// Optional manner of movement, used only as the prompt for navigation phases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace: Option<Pace>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Goal {
    Semantic(SemanticGoal),
    Scene(SceneOutcome),
}

impl TryFrom<Wire> for MotionGoal {
    type Error = String;
    fn try_from(wire: Wire) -> Result<Self, Self::Error> {
        let valid_goal = match (&wire.goal, wire.version) {
            (Goal::Semantic(goal), 2) => !goal.description.trim().is_empty() && goal.description.chars().count() <= 2048,
            (Goal::Scene(goal), 3) => !goal.target_id.trim().is_empty() && goal.target_id.chars().count() <= 128
                && (goal.operation == SceneOperation::Navigate || (goal.style.is_none() && goal.pace.is_none()))
                && goal.style.as_ref().is_none_or(|style| !style.trim().is_empty()
                    && style.chars().count() <= MAX_STYLE_CHARS && !style.chars().any(char::is_control)),
            _ => false,
        };
        if !valid_goal
            || !(1..=128).contains(&wire.session_id.chars().count())
            || [wire.generation_revision, wire.pts_ms, wire.seed]
                .iter()
                .any(|v| *v > (1_u64 << 53) - 1)
        {
            return Err("invalid closed motion_goal version/payload".into());
        }
        Ok(Self { wire })
    }
}

impl MotionGoal {
    pub fn scene_outcome(&self) -> Option<&SceneOutcome> {
        match &self.wire.goal { Goal::Scene(goal) => Some(goal), _ => None }
    }

    pub fn description(&self) -> Option<&str> {
        match &self.wire.goal { Goal::Semantic(goal) => Some(&goal.description), _ => None }
    }

    pub fn session_id(&self) -> &str { &self.wire.session_id }
    pub fn pts_ms(&self) -> u64 { self.wire.pts_ms }

    pub(crate) fn next_revision(&self) -> Result<Self, String> {
        if self.revision() >= (1_u64 << 53) - 1 { return Err("generation revision exhausted".into()); }
        let mut next = self.clone();
        next.wire.generation_revision += 1;
        Ok(next)
    }

    pub fn revision(&self) -> u64 {
        self.wire.generation_revision
    }

    pub fn new(
        session_id: String,
        revision: u64,
        pts_ms: u64,
        seed: u64,
        description: String,
    ) -> Result<Self, String> {
        Wire {
            kind: Kind::MotionGoal,
            version: 2,
            session_id,
            generation_revision: revision,
            pts_ms,
            seed,
            goal: Goal::Semantic(SemanticGoal { description }),
        }
        .try_into()
    }
}

/// Application-owned semantic requests, independent of the retired body planner.
/// One in-flight request prevents render ticks from queuing stale goal revisions.
// Core40 worker prefetches only when the generated horizon is at most 600ms ahead.
const PREFETCH_MS: u64 = 600;
const EMPTY_RETRY_MS: u64 = 1_000;
const MAX_EMPTY_ACKS_AT_CLOCK: u8 = 3;
const NO_PROGRESS_WATCHDOG_MS: u64 = 120_000;

#[derive(Default)]
pub struct NativeMotionDriver {
    goal: Option<MotionGoal>,
    in_flight: Option<MotionGoal>,
    in_flight_had_frames: bool,
    generated_until_pts_ms: Option<u64>,
    last_request_at_ms: u64,
    empty_acks_at_clock: u8,
    last_displayed_pts_ms: Option<u64>,
    last_progress_at_ms: Option<u64>,
    has_committed_output: bool,
    failed: bool,
}

impl NativeMotionDriver {
    /// Observed request ownership for UI/LLM producers, never a pose plan.
    pub fn current_goal(&self) -> Option<&MotionGoal> {
        self.goal.as_ref()
    }

    pub(crate) fn in_flight(&self) -> bool { self.in_flight.is_some() }
    pub(crate) fn generated_until(&self) -> Option<u64> { self.generated_until_pts_ms }

    pub(crate) fn validate_goal(&self, goal: &MotionGoal) -> Result<(), String> {
        if self.failed {
            return Err("motion stream failed; restart the session".into());
        }
        if let Some(old) = &self.goal {
            if goal.wire.session_id != old.wire.session_id
                || goal.wire.seed != old.wire.seed
                || goal.revision() < old.revision()
                || (goal.revision() == old.revision()
                    && goal.wire.goal != old.wire.goal)
            {
                return Err("immutable motion session/seed or stale goal revision".into());
            }
        }
        Ok(())
    }

    pub fn submit(&mut self, goal: MotionGoal) -> Result<(), String> {
        self.validate_goal(&goal)?;
        if self.goal.as_ref().is_none_or(|old| old.revision() != goal.revision()) {
            self.generated_until_pts_ms = None;
            self.empty_acks_at_clock = 0;
        }
        self.goal = Some(goal);
        Ok(())
    }

    fn observe_clock(&mut self, displayed_pts_ms: u64, now_ms: u64) {
        if self.last_displayed_pts_ms.is_none_or(|last| displayed_pts_ms > last) {
            self.last_displayed_pts_ms = Some(displayed_pts_ms);
            self.last_progress_at_ms = Some(now_ms);
            self.empty_acks_at_clock = 0;
        }
    }

    /// Called on each renderer tick; a stalled clock or lost ACK terminates explicitly.
    /// `now_ms` is monotonic elapsed time, independent of the displayed pose clock.
    pub fn watchdog(&mut self, displayed_pts_ms: u64, now_ms: u64) -> Option<String> {
        if self.failed || self.goal.is_none() || self.last_progress_at_ms.is_none() {
            return None;
        }
        self.observe_clock(displayed_pts_ms, now_ms);
        if now_ms.saturating_sub(self.last_progress_at_ms.unwrap()) < NO_PROGRESS_WATCHDOG_MS {
            return None;
        }
        let detail = format!("native motion stalled: displayed_pts_ms={displayed_pts_ms}, generated_until_pts_ms={:?}, in_flight={}, empty_acks_at_clock={}",
            self.generated_until_pts_ms, self.in_flight.is_some(), self.empty_acks_at_clock);
        self.fail();
        Some(detail)
    }

    /// Only accepted native frames for the current in-flight request raise the horizon.
    pub fn observe_frame(&mut self, session: &str, revision: u64, pts_ms: u64) {
        if self.in_flight.as_ref().is_some_and(|g| g.wire.session_id == session && g.revision() == revision) {
            self.in_flight_had_frames = true;
            if self.goal.as_ref().is_some_and(|current| current.revision() == revision) {
                self.generated_until_pts_ms = Some(self.generated_until_pts_ms.unwrap_or(0).max(pts_ms));
            }
        }
    }

    pub fn next_request(&mut self, displayed_pts_ms: u64, now_ms: u64) -> Option<MotionGoal> {
        if self.failed || self.in_flight.is_some() { return None; }
        let mut goal = self.goal.clone()?;
        self.observe_clock(displayed_pts_ms, now_ms);
        // Scene conditioning is slower than stock inference. This is a bounded
        // delivery lead, never a change to model timing or frame content.
        // 2s generation allowance plus the existing 2.4s revision-handoff lead.
        let prefetch_ms = if goal.scene_outcome().is_some() { 4400 } else { PREFETCH_MS };
        if (self.has_committed_output && displayed_pts_ms < 200)
            || self.generated_until_pts_ms.is_some_and(|end| end > displayed_pts_ms.saturating_add(prefetch_ms))
            || self.empty_acks_at_clock >= MAX_EMPTY_ACKS_AT_CLOCK
            || (self.empty_acks_at_clock > 0 && now_ms.saturating_sub(self.last_request_at_ms) < EMPTY_RETRY_MS)
        {
            return None;
        }
        goal.wire.pts_ms = displayed_pts_ms;
        self.in_flight_had_frames = false;
        self.last_request_at_ms = now_ms;
        self.in_flight = Some(goal.clone());
        Some(goal)
    }

    pub fn acknowledge(&mut self, session: &str, revision: u64, pts_ms: u64) {
        if self.in_flight.as_ref().is_some_and(|g| {
            g.wire.session_id == session && g.revision() == revision && g.wire.pts_ms == pts_ms
        }) {
            // An old revision's matching ACK releases its slot but cannot set the new horizon.
            if self.in_flight_had_frames { self.has_committed_output = true; }
            if self.goal.as_ref().is_some_and(|goal| goal.revision() == revision) {
                if self.in_flight_had_frames { self.empty_acks_at_clock = 0; }
                else { self.empty_acks_at_clock = self.empty_acks_at_clock.saturating_add(1); }
            }
            self.in_flight = None;
            self.in_flight_had_frames = false;
        }
    }

    pub fn fail(&mut self) -> bool {
        let first = !self.failed;
        self.failed = true;
        self.in_flight = None;
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_v3_is_closed_and_separate_from_semantic_v2() {
        let mut value = request();
        value["version"] = 3.into();
        value["goal"] = serde_json::json!({"operation":"navigate", "target_id":"desk"});
        let goal: MotionGoal = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(goal.scene_outcome().unwrap().target_id, "desk");
        assert!(goal.description().is_none());
        assert_eq!(serde_json::to_value(goal).unwrap(), value);
        let mut old = value.clone(); old["version"] = 2.into();
        assert!(serde_json::from_value::<MotionGoal>(old).is_err());
        for key in ["scene_context", "root_pos", "description", "motion_mask"] {
            let mut invalid = value.clone(); invalid["goal"][key] = serde_json::json!([]);
            assert!(serde_json::from_value::<MotionGoal>(invalid).is_err());
        }
        value["goal"]["operation"] = "teleport".into();
        assert!(serde_json::from_value::<MotionGoal>(value).is_err());
    }

    #[test]
    fn scene_v3_accepts_optional_style_and_pace_only_within_limits() {
        let mut value = request();
        value["version"] = 3.into();
        value["goal"] = serde_json::json!({"operation":"navigate", "target_id":"bed",
            "style":"A person waddles like a duck with bent knees and feet turned out.", "pace":"slow"});
        let goal: MotionGoal = serde_json::from_value(value.clone()).unwrap();
        let outcome = goal.scene_outcome().unwrap();
        assert_eq!(outcome.pace, Some(Pace::Slow));
        assert_eq!(serde_json::to_value(goal).unwrap(), value, "style and pace are forwarded unchanged");
        for (key, bad) in [("style", serde_json::json!("")), ("style", serde_json::json!("x".repeat(MAX_STYLE_CHARS + 1))),
                           ("style", serde_json::json!("walk\nnow")), ("pace", serde_json::json!("sprint")),
                           ("style", serde_json::json!(3))] {
            let mut invalid = value.clone(); invalid["goal"][key] = bad;
            assert!(serde_json::from_value::<MotionGoal>(invalid).is_err(), "{key}");
        }
        let mut grasp = value.clone(); grasp["goal"]["operation"] = "grasp".into();
        assert!(serde_json::from_value::<MotionGoal>(grasp).is_err(), "style/pace apply only to navigate");
    }

    #[test]
    fn scene_target_change_requires_a_new_revision() {
        let mut value = request(); value["version"] = 3.into();
        value["goal"] = serde_json::json!({"operation":"navigate", "target_id":"desk"});
        let mut driver = NativeMotionDriver::default();
        driver.submit(serde_json::from_value(value.clone()).unwrap()).unwrap();
        value["goal"]["target_id"] = "chair".into();
        assert!(driver.submit(serde_json::from_value(value.clone()).unwrap()).is_err());
        value["generation_revision"] = 2.into();
        driver.submit(serde_json::from_value(value).unwrap()).unwrap();
        assert_eq!(driver.current_goal().unwrap().scene_outcome().unwrap().target_id, "chair");
    }

    fn request() -> serde_json::Value {
        serde_json::json!({"kind":"motion_goal", "version":2, "session_id":"test",
            "generation_revision":1, "pts_ms":0, "seed":42,
            "goal":{"description":"A person kneels and rises."}})
    }

    #[test]
    fn native_driver_waits_for_matching_ack_and_uses_displayed_clock() {
        let mut driver = NativeMotionDriver::default();
        assert!(driver.next_request(0, 0).is_none());
        driver.submit(serde_json::from_value(request()).unwrap()).unwrap();
        let first = driver.next_request(0, 0).unwrap();
        assert_eq!(first.wire.pts_ms, 0);
        assert!(driver.next_request(100, 10).is_none());
        let mut rise = request();
        rise["generation_revision"] = 2.into();
        rise["goal"]["description"] = "A person stands up from a squat.".into();
        driver.submit(serde_json::from_value(rise).unwrap()).unwrap();
        driver.acknowledge("other", 1, 0);
        assert!(driver.next_request(200, 20).is_none());
        driver.observe_frame("test", 1, 2000);
        driver.acknowledge("test", 1, 0);
        assert!(driver.next_request(0, 30).is_none()); // Wait for one displayed native token.
        assert!(driver.next_request(150, 40).is_none());
        let changed = driver.next_request(250, 50).unwrap();
        assert_eq!(changed.wire.pts_ms, 250);
        assert_eq!(changed.wire.generation_revision, 2);
        assert_eq!(changed.description(), Some("A person stands up from a squat."));
        let mut changed_seed = request();
        changed_seed["seed"] = 43.into();
        assert!(driver.submit(serde_json::from_value(changed_seed).unwrap()).is_err());
    }

    #[test]
    fn fixed_clock_empty_acks_are_bounded_and_report_watchdog() {
        let mut driver = NativeMotionDriver::default();
        driver.submit(serde_json::from_value(request()).unwrap()).unwrap();
        let mut sent = 0;
        for now in (0..120_000).step_by(50) {
            if let Some(goal) = driver.next_request(0, now) {
                sent += 1;
                driver.acknowledge("test", 1, goal.wire.pts_ms);
            }
            assert!(driver.watchdog(0, now).is_none());
        }
        assert!(sent <= 3, "fixed clock sent {sent} requests");
        assert!(driver.watchdog(0, 120_000).unwrap().contains("stalled"));
        assert!(driver.next_request(0, 121_000).is_none());
        assert!(!driver.fail()); // one-shot terminal state
    }

    #[test]
    fn fixed_clock_after_full_horizon_does_not_poll_empty_acks() {
        let mut driver = NativeMotionDriver::default();
        driver.submit(serde_json::from_value(request()).unwrap()).unwrap();
        driver.next_request(0, 0).unwrap();
        for pts in (50..=2000).step_by(50) { driver.observe_frame("test", 1, pts); }
        driver.acknowledge("test", 1, 0);
        let mut requests = 1;
        for now in (50..110_000).step_by(50) {
            // The displayed clock advances ordinarily to the 600ms prefetch threshold,
            // then stops while the worker repeatedly acknowledges empty output.
            let displayed = (now / 50 * 50).min(1400);
            if let Some(goal) = driver.next_request(displayed, now) {
                requests += 1;
                driver.acknowledge("test", 1, goal.wire.pts_ms);
            }
            assert!(driver.watchdog(displayed, now).is_none());
        }
        assert!(requests <= 4, "full horizon followed by empty ACKs sent {requests} requests");
        assert_eq!(driver.watchdog(1400, 121_400).is_some(), true);
        assert!(driver.submit(serde_json::from_value(request()).unwrap()).is_err());
        let mut fresh = NativeMotionDriver::default();
        fresh.submit(serde_json::from_value(request()).unwrap()).unwrap();
        assert!(fresh.next_request(0, 0).is_some());
    }

    #[test]
    fn missing_ack_and_true_starvation_both_timeout_without_clock_fabrication() {
        let mut lost = NativeMotionDriver::default();
        lost.submit(serde_json::from_value(request()).unwrap()).unwrap();
        lost.next_request(0, 0).unwrap();
        assert!(lost.watchdog(0, 119_999).is_none());
        let message = lost.watchdog(0, 120_000).unwrap();
        assert!(message.contains("in_flight=true"));
        lost.acknowledge("test", 1, 0);
        assert!(lost.next_request(0, 120_001).is_none());

        let mut starved = NativeMotionDriver::default();
        starved.submit(serde_json::from_value(request()).unwrap()).unwrap();
        let first = starved.next_request(0, 0).unwrap();
        starved.observe_frame("test", 1, 2000);
        starved.acknowledge("test", 1, first.wire.pts_ms);
        // Once real playback reaches the horizon edge, only three empty retries.
        let mut count = 0;
        for now in (1000..15_000).step_by(50) {
            if let Some(goal) = starved.next_request(2000, now) {
                count += 1;
                starved.acknowledge("test", 1, goal.wire.pts_ms);
            }
        }
        assert_eq!(count, 3);
        assert!(starved.watchdog(2000, 122_000).unwrap().contains("stalled"));
    }

    #[test]
    fn horizon_prefetch_revision_and_stale_acks_respect_clock() {
        let mut driver = NativeMotionDriver::default();
        driver.submit(serde_json::from_value(request()).unwrap()).unwrap();
        let first = driver.next_request(0, 0).unwrap();
        for pts in (50..=2000).step_by(50) { driver.observe_frame("test", 1, pts); }
        driver.acknowledge("other", 1, 0);
        assert!(driver.next_request(1400, 100).is_none()); // still in flight
        driver.acknowledge("test", 1, first.wire.pts_ms);
        assert!(driver.next_request(1399, 200).is_none()); // 600ms prefetch threshold
        let second = driver.next_request(1400, 300).unwrap();
        assert_eq!(second.wire.pts_ms, 1400);
        driver.acknowledge("test", 1, 0); // late ACK cannot unlock
        driver.acknowledge("test", 2, 1400); // wrong revision cannot unlock
        assert!(driver.next_request(1500, 400).is_none());
        for pts in (2050..=4000).step_by(50) { driver.observe_frame("test", 1, pts); }
        driver.acknowledge("test", 1, 1400);
        assert!(driver.next_request(1500, 500).is_none());
        assert!(driver.next_request(3399, 550).is_none());
        let mut revision = request();
        revision["generation_revision"] = 2.into();
        driver.submit(serde_json::from_value(revision).unwrap()).unwrap();
        let changed = driver.next_request(3399, 600).unwrap(); // same displayed clock, new revision
        assert_eq!(changed.wire.generation_revision, 2);
        assert_eq!(changed.wire.pts_ms, 3399);
        driver.observe_frame("test", 1, 6000); // old revision cannot increase new horizon
        driver.acknowledge("test", 1, 3399);
        assert!(driver.next_request(3399, 700).is_none());
        driver.observe_frame("test", 2, 5400);
        driver.acknowledge("test", 2, 3399);
        assert!(driver.next_request(3399, 800).is_none());
        assert_eq!(driver.next_request(4800, 900).unwrap().wire.pts_ms, 4800);
    }

    #[test]
    fn scene_prefetch_preserves_handoff_lead_after_generation_latency() {
        let mut driver = NativeMotionDriver::default();
        let goal = serde_json::json!({"kind":"motion_goal","version":3,"session_id":"test",
            "generation_revision":1,"pts_ms":0,"seed":42,"goal":{"operation":"sit","target_id":"chair"}});
        driver.submit(serde_json::from_value(goal).unwrap()).unwrap();
        driver.next_request(0, 0).unwrap();
        for pts in (50..=6000).step_by(50) { driver.observe_frame("test", 1, pts); }
        driver.acknowledge("test", 1, 0);
        assert!(driver.next_request(1599, 0).is_none());
        assert!(driver.next_request(1600, 1).is_some());
        // After a 2s generation, a full suffix leaves >2.2s for a safe handoff.
        for pts in (6050..=8000).step_by(50) { driver.observe_frame("test", 1, pts); }
        driver.acknowledge("test", 1, 1600);
        assert!(super::super::scene_delivery::SceneDelivery::source_anchor(3600, driver.generated_until().unwrap()).is_some());
    }

    #[test]
    fn semantic_goal_round_trips() {
        let goal: MotionGoal = serde_json::from_value(request()).unwrap();
        assert_eq!(serde_json::to_value(goal).unwrap(), request());
    }

    #[test]
    fn prescriptions_and_legacy_protocol_fail_closed() {
        for key in [
            "timeline",
            "root_waypoints",
            "joints",
            "heading",
            "observed_root",
            "allow_override",
        ] {
            let mut value = request();
            value[key] = serde_json::json!([]);
            assert!(
                serde_json::from_value::<MotionGoal>(value).is_err(),
                "{key}"
            );
        }
        for (key, value) in [
            ("version", serde_json::json!(1)),
            ("kind", serde_json::json!("motion_request")),
            ("seed", serde_json::json!(true)),
            ("goal", serde_json::json!({"description":" "})),
            (
                "goal",
                serde_json::json!({"description":"walk", "root":[0,1,0]}),
            ),
        ] {
            let mut wire = request();
            wire[key] = value;
            assert!(serde_json::from_value::<MotionGoal>(wire).is_err());
        }
    }
}
