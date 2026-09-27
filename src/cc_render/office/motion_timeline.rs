//! Renderer-clock timeline sent to ARDY, separate from the pose playback buffer.
use super::controller::{JointTarget, MotionRequest};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootKeyframe {
    pub pts_ms: i64,
    pub position: [f32; 3],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadingTransition {
    pub start_ms: u64,
    pub duration_ms: u64,
    pub from: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootTrack {
    pub keyframes: Vec<RootKeyframe>,
    pub interpolation: String,
    /// Dense preserves walking behavior; keyframes leaves flight unconstrained.
    #[serde(default = "dense_position_mode")]
    pub position_mode: String,
    pub constrain_height: bool,
    pub heading: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_transition: Option<HeadingTransition>,
    /// Follow the XZ route tangent unless an explicit heading overrides it.
    #[serde(default)]
    pub face_path: bool,
    /// Zero keeps legacy dense guidance; positive intervals guide facing sparsely.
    #[serde(default)]
    pub heading_interval_ms: u64,
}
fn dense_position_mode() -> String { "dense".into() }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptSpan {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MotionTimeline {
    pub version: u8,
    pub skeleton_id: String,
    pub root: RootTrack,
    pub prompts: Vec<PromptSpan>,
    pub contacts: Vec<JointTarget>,
    pub pose_keyframes: Vec<serde_json::Value>,
}
impl MotionTimeline {
    /// Resolve local action timing to absolute time once per generation revision.
    /// A walking request contains no future sitting language or contact track.
    pub fn from_motion(motion: &MotionRequest) -> Self {
        let mut keys = vec![RootKeyframe {
            pts_ms: motion.pts_ms as i64,
            position: motion.observed_root,
        }];
        if motion.root_waypoints.first().is_some_and(|p| p[3] <= 0.) {
            keys.clear();
        }
        for p in &motion.root_waypoints {
            let key = RootKeyframe {
                pts_ms: motion.pts_ms as i64 + (p[3] * 1000.).round() as i64,
                position: [p[0], p[1], p[2]],
            };
            if keys
                .last()
                .is_some_and(|previous| previous.pts_ms == key.pts_ms)
            {
                keys.pop();
            }
            keys.push(key);
        }
        Self {
            version: 1,
            skeleton_id: "cskel27".into(),
            root: RootTrack {
                keyframes: keys,
                position_mode: dense_position_mode(),
                interpolation: if motion.ease_root { "smooth" } else { "linear" }.into(),
                constrain_height: motion.constrain_root_height,
                heading: motion.heading,
                heading_transition: None,
                face_path: false,
                heading_interval_ms: 0,
            },
            prompts: vec![PromptSpan {
                start_ms: motion.pts_ms,
                end_ms: None,
                text: motion.prompt.clone(),
            }],
            contacts: motion.joints.clone(),
            pose_keyframes: Vec::new(),
        }
    }
}

/// Replace named future tracks atomically. Omitting a channel preserves the
/// controller's channel; `layer: null` removes an earlier edit.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineLayer {
    pub root: Option<RootTrack>,
    pub prompts: Option<Vec<PromptSpan>>,
    #[serde(default)]
    pub pose_keyframes: Vec<serde_json::Value>,
    #[serde(default)]
    pub contacts: Vec<JointTarget>,
}
impl TimelineLayer {
    pub fn validate(&self, now: u64) -> Result<(), String> {
        if self.pose_keyframes.len() > 128 || self.contacts.len() > 32 {
            return Err("timeline edit too large".into());
        }
        if let Some(root) = &self.root {
            if root.keyframes.is_empty()
                || root.keyframes.len() > 256
                || !matches!(root.position_mode.as_str(), "dense" | "keyframes")
                || !matches!(root.interpolation.as_str(), "linear" | "smooth")
                || root
                    .keyframes
                    .iter()
                    .any(|k| k.pts_ms <= now as i64 || !k.position.iter().all(|v| v.is_finite()))
                || root
                    .keyframes
                    .windows(2)
                    .any(|w| w[0].pts_ms >= w[1].pts_ms)
                || (root.heading_interval_ms != 0 && !(50..=10000).contains(&root.heading_interval_ms))
                || root.heading.is_some_and(|h| !h.is_finite())
                || root.heading_transition.as_ref().is_some_and(|t| {
                    (root.heading.is_none() && !root.face_path) || !t.from.is_finite() || t.duration_ms == 0
                })
            {
                return Err("invalid root track".into());
            }
        }
        if let Some(prompts) = &self.prompts {
            if prompts.is_empty()
                || prompts.len() > 128
                || prompts.iter().any(|p| {
                    p.text.trim().is_empty() || p.end_ms.is_some_and(|end| end <= p.start_ms)
                })
                || prompts
                    .windows(2)
                    .any(|w| w[0].end_ms != Some(w[1].start_ms))
                || prompts[0].start_ms > now
                || prompts.last().unwrap().end_ms.is_some()
            {
                return Err("prompt spans must continuously cover now and the future".into());
            }
        }
        for key in &self.pose_keyframes {
            // Public timeline edits describe intent and geometry, never an
            // authored body. Internal legacy presets are not a public API.
            if key["kind"] == "full_body" {
                return Err("full-body edits require model-generated posture intent".into());
            }
            if !matches!(key["kind"].as_str(), Some("end_effectors" | "generated_posture" | "observed_support" | "model_pose"))
                || key["pts_ms"].as_u64().is_none_or(|t| t <= now)
            {
                return Err("pose edits must target future frames".into());
            }
        }
        let destinations: Vec<_> = self.pose_keyframes.iter()
            .filter(|k| k["kind"] == "generated_posture").collect();
        let supports: Vec<_> = self.pose_keyframes.iter()
            .filter(|k| k["kind"] == "observed_support").collect();
        if !destinations.is_empty() || !supports.is_empty() {
            // Both markers are a single atomic, model-led posture transition.
            // They cannot be submitted separately: the worker must never infer
            // support from a frame which was not itself generated and committed.
            if destinations.len() != 1 || supports.len() != 1
                || supports[0]["pts_ms"].as_u64() <= destinations[0]["pts_ms"].as_u64()
                || self.root.as_ref().is_none_or(|r| r.position_mode != "keyframes"
                    || r.constrain_height || r.heading.is_some() || r.face_path)
                || self.prompts.is_none()
                || !self.contacts.is_empty()
            {
                return Err("model-led posture requires a later observed support, free root height and continuous prompts".into());
            }
        }
        for key in &self.pose_keyframes {
            validate_pose(key)?;
        }
        for contact in &self.contacts {
            if !contact
                .position
                .iter()
                .chain(contact.rotation_xyzw.iter())
                .all(|v| v.is_finite())
                || contact.rotation_xyzw.iter().map(|v| v * v).sum::<f32>() < 1e-6
                || contact
                    .heading_reference
                    .is_some_and(|h| !h.is_finite() || !contact.rotation_only)
            {
                return Err("invalid contact target".into());
            }
        }
        Ok(())
    }
    pub fn apply(&self, timeline: &mut MotionTimeline) {
        if let Some(root) = &self.root {
            timeline.root = root.clone();
        }
        if let Some(prompts) = &self.prompts {
            timeline.prompts = prompts.clone();
        }
        timeline.pose_keyframes.extend(self.pose_keyframes.clone());
        timeline.contacts.extend(self.contacts.clone());
    }
}

fn validate_pose(key: &serde_json::Value) -> Result<(), String> {
    let invalid = || "invalid Core pose keyframe".to_string();
    let start = key["pts_ms"].as_u64().ok_or_else(invalid)?;
    if key["kind"] == "model_pose" {
        if !matches!(key["posture"].as_str(), Some("standing" | "seated" | "bend"))
            || !finite_vector(&key["root_position"], 3)
            || key["heading"].as_f64().is_none_or(|v| !v.is_finite())
            || key.as_object().is_none_or(|o| o.keys().any(|field| !matches!(field.as_str(),
                "kind" | "posture" | "root_position" | "heading" | "pts_ms" |
                "visible_from_ms" | "end_ms" | "hold" | "free_hands" | "carry_hands")))
        {
            return Err(invalid());
        }
        return Ok(());
    }
    if key["kind"] == "generated_posture" {
        if !matches!(key["posture"].as_str(), Some("kneeling" | "standing"))
            || key["heading"].as_f64().is_none_or(|h| !h.is_finite())
            || key["visible_from_ms"].as_u64().is_none_or(|t| t > start)
            || key.as_object().is_none_or(|o| o.keys().any(|k|
                !matches!(k.as_str(), "kind" | "posture" | "pts_ms" | "visible_from_ms" | "heading")))
        {
            return Err(invalid());
        }
        return Ok(());
    }
    if key["kind"] == "observed_support" {
        if key.as_object().is_none_or(|o| o.len() != 2 || !o.contains_key("kind")) {
            return Err(invalid());
        }
        return Ok(());
    }
    if key
        .get("end_ms")
        .is_some_and(|v| v.as_u64().is_none_or(|end| end <= start))
    {
        return Err(invalid());
    }
    if let Some(points) = key.get("keypoints") {
        let points = points.as_array().ok_or_else(invalid)?;
        let mut names = std::collections::HashSet::new();
        if key["kind"] != "end_effectors"
            || ["preset", "positions", "rotations_xyzw", "legacy_authored", "source", "joints"]
                .iter().any(|field| key.get(*field).is_some())
            || points.is_empty()
            || points.iter().any(|p| {
                !matches!(
                    p["joint"].as_str(),
                    Some("Hips" | "LeftHand" | "RightHand" | "LeftFoot" | "RightFoot")
                ) || !names.insert(p["joint"].as_str())
                    || !finite_vector(&p["position"], 3)
                    || p.get("rotation_xyzw").is_some_and(|q| {
                        !finite_vector(q, 4)
                            || q.as_array()
                                .unwrap()
                                .iter()
                                .map(|v| v.as_f64().unwrap().powi(2))
                                .sum::<f64>()
                                < 1e-12
                    })
            })
        {
            return Err(invalid());
        }
        return Ok(());
    }
    Err("end-effector edits must use sparse keypoints, not a body pose".into())
}
fn finite_vector(value: &serde_json::Value, width: usize) -> bool {
    value.as_array().is_some_and(|v| {
        v.len() == width && v.iter().all(|x| x.as_f64().is_some_and(f64::is_finite))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_timeline_edits_cannot_submit_internal_legacy_body_presets() {
        let layer = TimelineLayer {
            pose_keyframes: vec![json!({
                "kind":"full_body", "preset":"standing", "legacy_authored":true,
                "pts_ms":1100, "root_position":[0.0, 0.95, 0.0]
            })],
            ..Default::default()
        };
        assert!(layer.validate(1000).unwrap_err().contains("model-generated"));
        let mut sparse = TimelineLayer {
            pose_keyframes: vec![json!({
                "kind":"end_effectors", "pts_ms":1100,
                "keypoints":[{"joint":"RightFoot", "position":[0.0, 0.08, 0.0]}]
            })],
            ..Default::default()
        };
        assert!(sparse.validate(1000).is_ok());
        sparse.pose_keyframes[0]["rotations_xyzw"] = json!([[0.0, 0.0, 0.0, 1.0]]);
        assert!(sparse.validate(1000).is_err());
    }

    #[test]
    fn model_led_kneel_is_atomic_and_never_accepts_authored_rotations() {
        let mut layer: TimelineLayer = serde_json::from_value(json!({
            "root": {"keyframes": [{"pts_ms": 1050, "position": [0., 0.95, 0.]},
                                   {"pts_ms": 2600, "position": [0., 0.35, 0.]}],
                     "interpolation":"smooth", "position_mode":"keyframes",
                     "constrain_height":false, "heading":null},
            "prompts": [{"start_ms":1000, "end_ms":null,
                         "text":"A woman kneels down onto both knees on the floor."}],
            "pose_keyframes": [
                {"kind":"generated_posture", "posture":"kneeling", "pts_ms":2200,
                 "visible_from_ms":1000, "heading":0.0},
                {"kind":"observed_support", "pts_ms":2700}
            ]
        })).unwrap();
        assert!(layer.validate(1000).is_ok());
        layer.pose_keyframes[0]["rotations_xyzw"] = json!([[0., 0., 0., 1.]]);
        assert!(layer.validate(1000).is_err());
        layer.pose_keyframes[0].as_object_mut().unwrap().remove("rotations_xyzw");
        layer.pose_keyframes.pop();
        assert!(layer.validate(1000).is_err());
        layer.pose_keyframes.push(json!({"kind":"observed_support", "pts_ms":1900}));
        assert!(layer.validate(1000).is_err());
        layer.pose_keyframes[1]["pts_ms"] = json!(2700);
        layer.root.as_mut().unwrap().constrain_height = true;
        assert!(layer.validate(1000).is_err());
    }
}
