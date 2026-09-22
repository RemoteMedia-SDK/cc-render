//! Renderer-clock timeline sent to ARDY, separate from the pose playback buffer.
use super::controller::{JointTarget, MotionRequest};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootKeyframe {
    pub pts_ms: i64,
    pub position: [f32; 3],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootTrack {
    pub keyframes: Vec<RootKeyframe>,
    pub interpolation: String,
    pub constrain_height: bool,
    pub heading: Option<f32>,
    /// Follow the XZ route tangent unless an explicit heading overrides it.
    #[serde(default)]
    pub face_path: bool,
}
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
                interpolation: if motion.ease_root { "smooth" } else { "linear" }.into(),
                constrain_height: motion.constrain_root_height,
                heading: motion.heading,
                face_path: false,
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
                || !matches!(root.interpolation.as_str(), "linear" | "smooth")
                || root
                    .keyframes
                    .iter()
                    .any(|k| k.pts_ms <= now as i64 || !k.position.iter().all(|v| v.is_finite()))
                || root
                    .keyframes
                    .windows(2)
                    .any(|w| w[0].pts_ms >= w[1].pts_ms)
                || root.heading.is_some_and(|h| !h.is_finite())
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
            if !matches!(key["kind"].as_str(), Some("full_body" | "end_effectors"))
                || key["pts_ms"].as_u64().is_none_or(|t| t <= now)
            {
                return Err("pose edits must target future frames".into());
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
    if key["kind"] == "end_effectors" {
        let joints = key["joints"].as_array().ok_or_else(invalid)?;
        if joints.is_empty()
            || joints.iter().any(|j| {
                !matches!(
                    j.as_str(),
                    Some("Hips" | "LeftHand" | "RightHand" | "LeftFoot" | "RightFoot")
                )
            })
        {
            return Err(invalid());
        }
    }
    if let Some(target) = key.get("hand_target") {
        if !matches!(target["joint"].as_str(), Some("LeftHand" | "RightHand"))
            || !finite_vector(&target["position"], 3)
            || target.get("rotation_xyzw").is_some_and(|q| {
                !finite_vector(q, 4)
                    || q.as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_f64().unwrap().powi(2))
                        .sum::<f64>()
                        < 1e-12
            })
        {
            return Err(invalid());
        }
    }
    if let Some(preset) = key.get("preset") {
        if !matches!(preset.as_str(), Some("seated" | "standing" | "bend"))
            || !finite_vector(&key["root_position"], 3)
        {
            return Err(invalid());
        }
    } else {
        for (name, width) in [("positions", 3), ("rotations_xyzw", 4)] {
            let rows = key[name].as_array().ok_or_else(invalid)?;
            if rows.len() != 27
                || rows.iter().any(|r| {
                    !finite_vector(r, width)
                        || (width == 4
                            && r.as_array()
                                .unwrap()
                                .iter()
                                .map(|v| v.as_f64().unwrap().powi(2))
                                .sum::<f64>()
                                < 1e-12)
                })
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
fn finite_vector(value: &serde_json::Value, width: usize) -> bool {
    value.as_array().is_some_and(|v| {
        v.len() == width && v.iter().all(|x| x.as_f64().is_some_and(f64::is_finite))
    })
}
