//! A semantic continuation after observed navigation arrival. Duration follows
//! displayed native time, never wall time, generated-ahead time, or edited poses.
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub(super) struct TimedPrompt {
    description: String,
    requested_duration_ms: u64,
    status: &'static str,
    started_pts_ms: Option<u64>,
    stop_requested_pts_ms: Option<u64>,
    ended_pts_ms: Option<u64>,
    error: Option<String>,
    prompt_revision: Option<u64>,
    #[serde(skip)]
    dirty: bool,
}

impl TimedPrompt {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let Ok(description) = std::env::var("CC_CONDO_AFTER_NAV_PROMPT") else { return Ok(None) };
        let seconds = std::env::var("CC_CONDO_PROMPT_SECONDS").map_or(Ok(30.), |s| s.parse::<f64>())?;
        Ok(Some(Self::new(description, seconds)?))
    }

    pub fn new(description: String, seconds: f64) -> anyhow::Result<Self> {
        anyhow::ensure!(!description.trim().is_empty() && description.chars().count() <= 2048,
            "follow-up prompt must contain 1..2048 characters");
        anyhow::ensure!(seconds.is_finite() && (1.0..=120.0).contains(&seconds),
            "prompt duration must be within 1..120 seconds");
        Ok(Self { description, requested_duration_ms:(seconds * 1000.).round() as u64,
            status:"waiting_for_arrival", started_pts_ms:None, stop_requested_pts_ms:None,
            ended_pts_ms:None, error:None, prompt_revision:None, dirty:true })
    }

    pub fn start(&mut self, revision: u64, pts: u64) -> (u64, u64, String) {
        self.prompt_revision = Some(revision);
        self.status = "waiting_for_prompt_display";
        self.dirty = true;
        (revision, pts, self.description.clone())
    }

    pub fn complete(&self) -> bool { self.status == "complete" }

    pub fn save(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        if self.dirty {
            std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
            self.dirty = false;
        }
        Ok(())
    }

    pub fn observe(&mut self, event: &Value, session: &str, target: &str) -> Option<(u64, u64, String)> {
        if matches!(self.status, "complete" | "failed") { return None; }
        if event["kind"] == "motion_error" || event["kind"] == "worker_error" {
            self.error = Some(event["error"].to_string());
            self.status = "failed";
            self.dirty = true;
            return None;
        }
        if self.status == "waiting_for_arrival" {
            let goal = &event["goal"];
            if event["kind"] == "motion_goal_status" && event["status"] == "completed"
                && goal["session_id"] == session && goal["generation_revision"] == 2
                && goal["goal"]["operation"] == "navigate" && goal["goal"]["target_id"] == target {
                let pts = event["displayed_pts_ms"].as_u64()?;
                return Some(self.start(3, pts));
            }
            return None;
        }
        if event["kind"] != "scene_state" { return None; }
        let office = &event["state"]["office"];
        let avatar = &office["avatar"];
        if avatar["native_stream"]["session_id"] != session || avatar["ready"] != true { return None; }
        let pts = avatar["displayed_pts_ms"].as_u64()?;
        let revision = avatar["native_stream"]["generation_revision"].as_u64()?;
        let prompt_revision = self.prompt_revision?;
        if revision == prompt_revision && self.stop_requested_pts_ms.is_none() {
            let start = *self.started_pts_ms.get_or_insert_with(|| {
                self.status = "running"; self.dirty = true; pts
            });
            if pts.saturating_sub(start) >= self.requested_duration_ms {
                self.stop_requested_pts_ms = Some(pts);
                self.status = "waiting_for_end_display";
                self.dirty = true;
                return Some((prompt_revision + 1, pts, "A person stands relaxed under the shower.".into()));
            }
        } else if revision == prompt_revision + 1 && self.stop_requested_pts_ms.is_some() {
            self.ended_pts_ms = Some(pts);
            self.status = "complete";
            self.dirty = true;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn arrived() -> Value {
        json!({"kind":"motion_goal_status","status":"completed","displayed_pts_ms":5000,
            "goal":{"session_id":"test","generation_revision":2,
                "goal":{"operation":"navigate","target_id":"shower_screen"}}})
    }
    fn displayed(session: &str, revision: u64, pts: u64) -> Value {
        json!({"kind":"scene_state","state":{"office":{"avatar":{"ready":true,"displayed_pts_ms":pts,
            "native_stream":{"session_id":session,"generation_revision":revision}}}}})
    }
    #[test]
    fn continuation_requires_matching_arrival_and_times_only_displayed_prompt() {
        let mut prompt = TimedPrompt::new("Wash in place.".into(), 30.).unwrap();
        assert!(prompt.observe(&arrived(), "foreign", "shower_screen").is_none());
        assert!(prompt.observe(&arrived(), "test", "other").is_none());
        assert_eq!(prompt.observe(&arrived(), "test", "shower_screen").unwrap().0, 3);
        assert!(prompt.observe(&arrived(), "test", "shower_screen").is_none());
        for event in [displayed("test",2,50000), displayed("foreign",3,50000)] {
            assert!(prompt.observe(&event,"test","shower_screen").is_none());
        }
        assert!(prompt.started_pts_ms.is_none());
        for pts in [6000, 6000, 35999] {
            assert!(prompt.observe(&displayed("test",3,pts),"test","shower_screen").is_none());
        }
        assert_eq!(prompt.observe(&displayed("test",3,36000),"test","shower_screen").unwrap().0,4);
        assert!(prompt.observe(&displayed("test",3,37000),"test","shower_screen").is_none());
        prompt.observe(&displayed("test",4,36600),"test","shower_screen");
        assert_eq!(prompt.ended_pts_ms,Some(36600));
        assert_eq!(prompt.status,"complete");
    }
    #[test]
    fn errors_do_not_trigger_a_replacement_or_claim_completed_duration() {
        for seconds in [f64::NAN, f64::INFINITY, 0., 121.] {
            assert!(TimedPrompt::new("Wash.".into(),seconds).is_err());
        }
        let mut prompt = TimedPrompt::new("Wash.".into(),30.).unwrap();
        prompt.observe(&arrived(),"test","shower_screen");
        prompt.observe(&json!({"kind":"motion_error","error":"rejected"}),"test","shower_screen");
        assert!(prompt.observe(&displayed("test",3,99999),"test","shower_screen").is_none());
        assert_eq!(prompt.status,"failed");
        assert!(prompt.ended_pts_ms.is_none());
    }
}
