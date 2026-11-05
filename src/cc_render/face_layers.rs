//! The face from two producers at once.
//!
//! - **Live**: `live: true` envelopes (`LiveLinkFaceNode`,
//!   `GameInputFaceNode`, `FaceMixNode`). Applied the moment they arrive.
//! - **Speech**: the playback-clocked lip-sync queue (`Audio2FaceLipSyncNode`
//!   frames released as their audio plays).
//!
//! While speech plays it owns the articulation shapes: jaw, mouth, tongue
//! and cheek puff. The live face keeps the eyes, lids, brows, cheeks, nose
//! and head. After the last speech pose the mouth hands back to the live
//! face over [`SPEECH_FADE`]. With one producer, its poses pass through
//! unchanged, so the office (speech only) and a bare live face behave as
//! before.
//!
//! This is the first slice of `add-layered-face-composition`
//! (remotemedia-sdk openspec): a speech layer over a live layer, nothing more.

use std::time::{Duration, Instant};

use super::renderer::ArkitPose;

/// Speech keeps the mouth this long after its last pose: at 20–30 fps,
/// poses are 33–50 ms apart, and a late tick must not let the live mouth
/// flicker through.
pub const SPEECH_HOLD: Duration = Duration::from_millis(150);
/// Then the live mouth fades back in over this long.
pub const SPEECH_FADE: Duration = Duration::from_millis(200);

/// The shapes speech owns while it plays.
pub fn is_articulation(name: &str) -> bool {
    name.starts_with("jaw") || name.starts_with("mouth") || name == "tongueOut" || name == "cheekPuff"
}

#[derive(Debug, Default)]
pub struct FaceLayers {
    live: Option<ArkitPose>,
    /// The newest released speech pose and when it was released.
    speech: Option<(ArkitPose, Instant)>,
}

impl FaceLayers {
    /// A live pose arrived: the face to show now.
    pub fn on_live(&mut self, pose: ArkitPose, now: Instant) -> ArkitPose {
        self.live = Some(pose);
        self.compose(now)
    }

    /// A speech pose was released (its audio is playing): the face to show.
    pub fn on_speech(&mut self, pose: ArkitPose, now: Instant) -> ArkitPose {
        self.speech = Some((pose, now));
        self.compose(now)
    }

    /// Speech cancelled (barge-in): the face to show, which is the live face
    /// if there is one.
    pub fn barge(&mut self) -> ArkitPose {
        self.speech = None;
        self.live.clone().unwrap_or_default()
    }

    /// How much of the articulation speech owns at `now` (0..=1).
    pub fn speech_weight(&self, now: Instant) -> f32 {
        let Some((_, at)) = &self.speech else { return 0.0 };
        let age = now.saturating_duration_since(*at);
        if age <= SPEECH_HOLD {
            return 1.0;
        }
        let fading = (age - SPEECH_HOLD).as_secs_f32() / SPEECH_FADE.as_secs_f32();
        (1.0 - fading).clamp(0.0, 1.0)
    }

    fn compose(&self, now: Instant) -> ArkitPose {
        let w = self.speech_weight(now);
        match (&self.live, &self.speech) {
            (None, Some((speech, _))) => speech.clone(),
            (Some(live), Some(_)) if w <= 0.0 => live.clone(),
            (None, None) => ArkitPose::default(),
            (Some(live), None) => live.clone(),
            (Some(live), Some((speech, _))) => {
                let mut out = live.clone();
                let keys = live.weights.keys().chain(speech.weights.keys()).filter(|k| is_articulation(k));
                let keys: std::collections::HashSet<String> = keys.cloned().collect();
                for k in keys {
                    let l = live.weights.get(&k).copied().unwrap_or(0.0);
                    let s = speech.weights.get(&k).copied().unwrap_or(0.0);
                    out.weights.insert(k, l * (1.0 - w) + s * w);
                }
                // Speech poses carry the content clock the capture is
                // stamped with; keep it while speech leads.
                if w >= 0.5 {
                    out.pts_ms = speech.pts_ms;
                }
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(pairs: &[(&str, f32)], pts_ms: u64) -> ArkitPose {
        ArkitPose { weights: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(), pts_ms }
    }

    #[test]
    fn articulation_is_the_jaw_mouth_tongue_and_puff() {
        for n in ["jawOpen", "jawLeft", "mouthClose", "mouthSmileLeft", "tongueOut", "cheekPuff"] {
            assert!(is_articulation(n), "{n}");
        }
        for n in ["eyeBlinkLeft", "browInnerUp", "cheekSquintLeft", "noseSneerLeft", "headYaw"] {
            assert!(!is_articulation(n), "{n}");
        }
    }

    #[test]
    fn speech_takes_the_mouth_and_the_live_face_keeps_the_rest() {
        let t0 = Instant::now();
        let mut f = FaceLayers::default();
        let live = pose(&[("jawOpen", 0.1), ("mouthSmileLeft", 0.6), ("eyeBlinkLeft", 0.8), ("headYaw", 0.3)], 10);
        assert_eq!(f.on_live(live.clone(), t0).weights, live.weights, "no speech: live unchanged");
        let out = f.on_speech(pose(&[("jawOpen", 0.7), ("mouthFunnel", 0.4)], 500), t0);
        assert_eq!(out.weights["jawOpen"], 0.7);
        assert_eq!(out.weights["mouthFunnel"], 0.4);
        assert_eq!(out.weights["mouthSmileLeft"], 0.0, "speech owns every mouth shape");
        assert_eq!(out.weights["eyeBlinkLeft"], 0.8);
        assert_eq!(out.weights["headYaw"], 0.3);
        assert_eq!(out.pts_ms, 500);
        // A live frame between speech poses keeps the speaking mouth.
        let mut live2 = live.clone();
        live2.weights.insert("eyeBlinkLeft".into(), 0.0);
        let out = f.on_live(live2, t0 + Duration::from_millis(40));
        assert_eq!(out.weights["jawOpen"], 0.7);
        assert_eq!(out.weights["eyeBlinkLeft"], 0.0, "blinks stay live");
    }

    #[test]
    fn the_live_mouth_fades_back_after_speech() {
        let t0 = Instant::now();
        let mut f = FaceLayers::default();
        f.on_speech(pose(&[("jawOpen", 0.0)], 0), t0);
        let live = pose(&[("jawOpen", 0.5)], 0);
        let at = |ms| f.speech_weight(t0 + Duration::from_millis(ms));
        assert_eq!(at(150), 1.0);
        assert!((at(250) - 0.5).abs() < 1e-3);
        assert_eq!(at(400), 0.0);
        let mid = f.on_live(live.clone(), t0 + Duration::from_millis(250));
        assert!((mid.weights["jawOpen"] - 0.25).abs() < 1e-3);
        let back = f.on_live(live, t0 + Duration::from_millis(400));
        assert_eq!(back.weights["jawOpen"], 0.5);
    }

    #[test]
    fn one_producer_passes_through_and_barge_returns_to_live() {
        let t0 = Instant::now();
        let mut f = FaceLayers::default();
        let speech = pose(&[("jawOpen", 0.7), ("eyeBlinkLeft", 0.2)], 33);
        assert_eq!(f.on_speech(speech.clone(), t0).weights, speech.weights, "office: speech only");
        assert_eq!(f.barge().weights, ArkitPose::default().weights, "no live face: neutral");
        let live = pose(&[("jawOpen", 0.1)], 1);
        f.on_live(live.clone(), t0);
        f.on_speech(speech, t0);
        assert_eq!(f.barge().weights, live.weights);
        assert_eq!(f.speech_weight(t0), 0.0);
    }
}
