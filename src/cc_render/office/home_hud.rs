//! Display-only home task overlay. A goal producer reports its own needs,
//! queue and step progress; nothing here schedules goals, reads observations
//! or reaches body, skeleton, physics or object state.
use serde::{Deserialize, Serialize};

const MAX_NEEDS: usize = 8;
const MAX_STEPS: usize = 16;
const MAX_QUEUE: usize = 8;
const MAX_LABEL: usize = 64;
const MAX_DETAIL: usize = 160;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    kind: Kind,
    version: u8,
    hud: Option<HomeHud>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Kind {
    #[serde(rename = "home_hud")]
    HomeHud,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeHud {
    pub needs: Vec<NeedMeter>,
    #[serde(default)]
    pub current: Option<CurrentTask>,
    #[serde(default)]
    pub queue: Vec<String>,
    pub autonomy: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeedMeter {
    pub id: String,
    pub label: String,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentTask {
    pub label: String,
    pub steps: Vec<HudStep>,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudStep {
    pub label: String,
    pub status: StepStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus { Pending, Running, Done, Failed, Blocked }

/// Parse `{"kind":"home_hud","version":1,"hud":{..}|null}`; `null` hides the panel.
pub fn parse(value: &serde_json::Value) -> Result<Option<HomeHud>, String> {
    let wire: Wire = serde_json::from_value(value.clone()).map_err(|e| format!("invalid home_hud: {e}"))?;
    if wire.version != 1 {
        return Err("invalid home_hud: unsupported version".into());
    }
    if let Some(hud) = &wire.hud {
        hud.validate()?;
    }
    Ok(wire.hud)
}

impl HomeHud {
    fn validate(&self) -> Result<(), String> {
        let label = |s: &str, max: usize| !s.trim().is_empty() && s.chars().count() <= max;
        let valid = self.needs.len() <= MAX_NEEDS
            && self.queue.len() <= MAX_QUEUE
            && self.needs.iter().all(|n| label(&n.id, MAX_LABEL) && label(&n.label, MAX_LABEL)
                && n.value.is_finite() && (0. ..=1.).contains(&n.value))
            && self.queue.iter().all(|q| label(q, MAX_LABEL))
            && self.current.as_ref().is_none_or(|c| label(&c.label, MAX_LABEL)
                && c.steps.len() <= MAX_STEPS
                && c.steps.iter().all(|s| label(&s.label, MAX_LABEL))
                && c.detail.as_deref().is_none_or(|d| label(d, MAX_DETAIL)));
        if valid { Ok(()) } else { Err("invalid home_hud: out-of-range value or label".into()) }
    }
}

#[cfg(test)]
pub(crate) fn fixture() -> serde_json::Value {
    serde_json::json!({"kind":"home_hud","version":1,"hud":{
        "needs":[{"id":"hunger","label":"Hunger","value":0.35},{"id":"energy","label":"Energy","value":0.8}],
        "current":{"label":"Set the table","steps":[
            {"label":"Walk to the sink","status":"done"},
            {"label":"Pick up the plate","status":"running"},
            {"label":"Put it on the table","status":"blocked"}],
            "detail":"place: unsupported_capability"},
        "queue":["Watch TV"],"autonomy":true}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_schema_accepts_display_state_and_null_clear() {
        let hud = parse(&fixture()).unwrap().unwrap();
        assert_eq!(hud.current.unwrap().steps[2].status, StepStatus::Blocked);
        assert_eq!(parse(&serde_json::json!({"kind":"home_hud","version":1,"hud":null})).unwrap(), None);
    }

    #[test]
    fn rejects_unknown_fields_motion_channels_and_out_of_range_values() {
        for (pointer, value) in [
            ("/hud/root_waypoints", serde_json::json!([[0, 0, 0]])),
            ("/hud/current/steps/0/target_position", serde_json::json!([0, 0, 0])),
            ("/hud/needs/0/value", serde_json::json!(1.5)),
            ("/hud/needs/0/label", serde_json::json!("")),
            ("/hud/current/steps/0/status", serde_json::json!("skipped")),
            ("/version", serde_json::json!(2)),
            ("/kind", serde_json::json!("motion_goal")),
        ] {
            let mut value_json = fixture();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            let target = if parent.is_empty() { &mut value_json } else { value_json.pointer_mut(parent).unwrap() };
            target[key] = value;
            assert!(parse(&value_json).is_err(), "{pointer} must be rejected");
        }
        let mut long = fixture();
        long["hud"]["queue"] = serde_json::json!(vec!["Nap"; MAX_QUEUE + 1]);
        assert!(parse(&long).is_err());
    }
}
