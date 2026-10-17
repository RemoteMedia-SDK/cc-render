//! Read-only diagnostic replay. Revision changes require saved service responses.
use cc_render_plugin::cc_render::{
    office::controller::OfficeCommand, renderer::{SceneState, SkeletalPose}, CcRenderNode, Renderer,
};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

fn lines(path: &Path) -> anyhow::Result<Vec<Value>> {
    std::fs::read_to_string(path)?.lines().map(|line| Ok(serde_json::from_str(line)?)).collect()
}

pub fn load(path: &Path) -> anyhow::Result<Vec<SkeletalPose>> {
    let packets = lines(path)?;
    let poses = validate(&packets)?;
    if poses.windows(2).any(|p| revision(&p[0]) != revision(&p[1])) {
        verify_handoffs(&packets, &lines(&path.with_file_name("responses.ndjson"))?)?;
    }
    Ok(poses)
}

fn revision(pose: &SkeletalPose) -> u64 {
    pose.stream.as_ref().expect("validated native stream").generation_revision
}

fn validate(packets: &[Value]) -> anyhow::Result<Vec<SkeletalPose>> {
    let poses: Vec<_> = packets.iter().map(|packet| {
        CcRenderNode::skeletal_pose_from_json(packet)
            .ok_or_else(|| anyhow::anyhow!("invalid recorded native packet"))
    }).collect::<anyhow::Result<_>>()?;
    let first = poses.first().ok_or_else(|| anyhow::anyhow!("empty recorded motion"))?;
    let stream = first.stream.as_ref().ok_or_else(|| anyhow::anyhow!("recording needs native stream identity"))?;
    anyhow::ensure!(poses.iter().all(|p| p.stream.as_ref().is_some_and(|s| s.session_id == stream.session_id)),
        "mixed recorded motion sessions");
    anyhow::ensure!(poses.windows(2).all(|p| p[1].pts_ms > p[0].pts_ms && revision(&p[1]) >= revision(&p[0])),
        "recorded timestamps or revisions move backwards");
    Ok(poses)
}

fn key(packet: &Value) -> anyhow::Result<(u64, u64)> {
    Ok((packet["generation_revision"].as_u64().ok_or_else(|| anyhow::anyhow!("missing revision"))?,
        packet["pts_ms"].as_u64().ok_or_else(|| anyhow::anyhow!("missing timestamp"))?))
}

fn verify_handoffs(packets: &[Value], responses: &[Value]) -> anyhow::Result<()> {
    let mut source = BTreeMap::new();
    let mut anchors = BTreeMap::new();
    for response in responses {
        let audit = &response["audit"];
        anyhow::ensure!(audit["future_nonzero"] == 0 && audit["future_channels"] == serde_json::json!([])
            && audit["exceptions"] == serde_json::json!([]), "replay requires semantic-only service responses");
        let frames = response["frames"].as_array().ok_or_else(|| anyhow::anyhow!("missing source frames"))?;
        if let Some(first) = frames.first() {
            anchors.insert(key(first)?, response["provenance"]["history_end_pts_ms"].as_u64());
        }
        for frame in frames {
            anyhow::ensure!(source.insert(key(frame)?, frame).is_none(), "duplicate service source frame");
        }
    }
    for packet in packets {
        anyhow::ensure!(source.get(&key(packet)?) == Some(&packet), "replay packet differs from service source");
    }
    for pair in packets.windows(2) {
        let previous = key(&pair[0])?;
        let next = key(&pair[1])?;
        if previous.0 != next.0 {
            anyhow::ensure!(anchors.get(&next) == Some(&Some(previous.1)),
                "revision boundary does not match service-owned history");
        }
    }
    Ok(())
}

fn revision_command(previous: Option<&SkeletalPose>, next: &SkeletalPose) -> Option<OfficeCommand> {
    let generation_revision = revision(next);
    match previous {
        None => Some(OfficeCommand::NativeMotionRevision { generation_revision }),
        Some(old) if revision(old) != generation_revision => Some(OfficeCommand::NativeSceneHandoff {
            generation_revision, history_pts_ms: old.pts_ms,
        }),
        _ => None,
    }
}

fn handoff_ready(state: &SceneState, previous: &SkeletalPose) -> bool {
    let Some(office) = &state.office else { return false };
    let Some(stream) = &office.avatar.native_stream else { return false };
    let expected = previous.stream.as_ref().expect("validated native stream");
    stream.session_id == expected.session_id && stream.generation_revision == expected.generation_revision
        && office.avatar.buffered_until_pts_ms.unwrap_or(office.avatar.displayed_pts_ms) >= previous.pts_ms
}

#[derive(Default)]
pub struct PlaybackCursor {
    pub sent: usize,
    requested_revision: Option<u64>,
}

impl PlaybackCursor {
    pub fn feed(&mut self, renderer: &impl Renderer, poses: &[SkeletalPose], elapsed_ms: u64) {
        // Pace transport only. Never change source timestamps or pose coordinates.
        let Some(first) = poses.first() else { return };
        let state = renderer.scene_state();
        let observed = state.office.as_ref().filter(|office|
            office.avatar.native_stream.as_ref().is_some_and(|stream|
                first.stream.as_ref().is_some_and(|source| stream.session_id == source.session_id)))
            .map_or(first.pts_ms, |office| office.avatar.displayed_pts_ms);
        // GPU/encoder backpressure can make wall time outrun playback. Keep
        // the prefetch bounded by the observed clock so no source frames are
        // silently lost to PoseTimeline's bounded future queue.
        let deadline = elapsed_ms.saturating_add(500).min(observed.saturating_add(500));
        while self.sent < poses.len() && poses[self.sent].pts_ms <= deadline {
            let previous = self.sent.checked_sub(1).map(|index| &poses[index]);
            if !self.prepare_revision(renderer, previous, &poses[self.sent]) {
                break;
            }
            renderer.push_skeletal_pose(Some(poses[self.sent].clone()));
            self.sent += 1;
        }
    }

    fn prepare_revision(&mut self, renderer: &impl Renderer, previous: Option<&SkeletalPose>, next: &SkeletalPose) -> bool {
        let Some(command) = revision_command(previous, next) else { return true };
        let state = renderer.scene_state();
        if previous.is_some_and(|old| !handoff_ready(&state, old)) {
            return false;
        }
        if self.requested_revision != Some(revision(next)) {
            renderer.office_command(command);
            self.requested_revision = Some(revision(next));
        }
        previous.is_none_or(|old| state.office.as_ref().is_some_and(|office|
            office.avatar.acknowledged_native_handoff == Some((revision(next), old.pts_ms))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(revision: u64, pts: u64) -> Value {
        serde_json::json!({"kind":"skeletal_pose", "session_id":"fixture", "generation_revision":revision,
            "skeleton_id":"smpl22", "root_mode":"world", "root_pos":[0,1,0], "pts_ms":pts,
            "joint_quats_xyzw":vec![[0.,0.,0.,1.];22]})
    }

    fn response(frames: &[Value], anchor: Option<u64>) -> Value {
        serde_json::json!({"frames":frames, "provenance":{"history_end_pts_ms":anchor},
            "audit":{"future_nonzero":0,"future_channels":[],"exceptions":[]}})
    }

    #[test]
    fn handoff_preserves_only_service_committed_prefix_and_original_packets() {
        let original = vec![packet(1, 50), packet(1, 100), packet(1, 150)];
        let continuation = vec![packet(2, 150), packet(2, 200)];
        let packets = vec![original[0].clone(), original[1].clone(), continuation[0].clone(), continuation[1].clone()];
        let responses = vec![response(&original, None), response(&continuation, Some(100))];
        verify_handoffs(&packets, &responses).unwrap();
        let poses = validate(&packets).unwrap();
        assert!(matches!(revision_command(Some(&poses[1]), &poses[2]),
            Some(OfficeCommand::NativeSceneHandoff { generation_revision:2, history_pts_ms:100 })));
        let mut modified = packets.clone();
        modified[2]["root_pos"][1] = serde_json::json!(1.01);
        assert!(verify_handoffs(&modified, &responses).is_err());
        let wrong_prefix = vec![original[0].clone(), continuation[0].clone()];
        assert!(verify_handoffs(&wrong_prefix, &responses).is_err());
    }

    #[test]
    fn malformed_streams_and_unproven_revisions_are_rejected() {
        assert!(validate(&[]).is_err());
        assert!(validate(&[packet(2, 50), packet(1, 100)]).is_err());
        assert!(validate(&[packet(1, 100), packet(1, 50)]).is_err());
        let mut foreign = packet(1, 100);
        foreign["session_id"] = serde_json::json!("foreign");
        assert!(validate(&[packet(1, 50), foreign]).is_err());
        assert!(verify_handoffs(&[packet(1, 50), packet(2, 100)], &[]).is_err());
    }

    #[test]
    fn handoff_waits_for_the_correct_stream_prefix_to_reach_the_renderer() {
        let poses = validate(&[packet(1, 50), packet(1, 100)]).unwrap();
        let mut state = SceneState::default();
        assert!(!handoff_ready(&state, &poses[1]));
        let mut office = cc_render_plugin::cc_render::office::controller::OfficeController::default().snapshot();
        office.avatar.native_stream = poses[1].stream.clone();
        office.avatar.buffered_until_pts_ms = Some(50);
        state.office = Some(office);
        assert!(!handoff_ready(&state, &poses[1]));
        state.office.as_mut().unwrap().avatar.buffered_until_pts_ms = Some(100);
        assert!(handoff_ready(&state, &poses[1]));
        state.office.as_mut().unwrap().avatar.native_stream.as_mut().unwrap().session_id = "foreign".into();
        assert!(!handoff_ready(&state, &poses[1]));
    }

    use cc_render_plugin::cc_render::renderer::{ArkitPose, RenderedFrame};
    use std::sync::Mutex;
    #[derive(Default)]
    struct Recorder {
        state: Mutex<SceneState>,
        commands: Mutex<Vec<OfficeCommand>>,
        packets: Mutex<Vec<SkeletalPose>>,
    }
    impl Renderer for Recorder {
        fn push_pose(&self, _: ArkitPose) {}
        fn push_skeletal_pose(&self, pose: Option<SkeletalPose>) { self.packets.lock().unwrap().push(pose.unwrap()); }
        fn drain_frames(&self, _: &mut Vec<RenderedFrame>) {}
        fn office_command(&self, command: OfficeCommand) { self.commands.lock().unwrap().push(command); }
        fn scene_state(&self) -> SceneState { self.state.lock().unwrap().clone() }
    }

    #[test]
    fn new_revision_packets_wait_for_applied_command_acknowledgement() {
        let poses = validate(&[packet(1, 50), packet(1, 100), packet(2, 150), packet(2, 200)]).unwrap();
        let renderer = Recorder::default();
        let mut replay = PlaybackCursor::default();
        replay.feed(&renderer, &poses, 200);
        assert_eq!(replay.sent, 2);
        let mut office = cc_render_plugin::cc_render::office::controller::OfficeController::default().snapshot();
        office.avatar.native_stream = poses[1].stream.clone();
        office.avatar.buffered_until_pts_ms = Some(100);
        renderer.state.lock().unwrap().office = Some(office);
        replay.feed(&renderer, &poses, 200);
        replay.feed(&renderer, &poses, 200);
        assert_eq!(replay.sent, 2, "buffered prefix alone is not command acknowledgement");
        assert_eq!(renderer.commands.lock().unwrap().len(), 2, "each revision requested once");
        renderer.state.lock().unwrap().office.as_mut().unwrap().avatar.acknowledged_native_handoff = Some((2, 100));
        replay.feed(&renderer, &poses, 200);
        assert_eq!(replay.sent, 4);
        let recorded = renderer.packets.lock().unwrap();
        assert!(recorded.iter().zip(&poses).all(|(a,b)|
            a.pts_ms == b.pts_ms && a.root_pos == b.root_pos && a.joint_quats == b.joint_quats));
    }
    #[test]
    fn slow_capture_bounds_prefetch_without_losing_or_rewriting_source_packets() {
        let raw: Vec<_> = (1..=436).map(|i| {
            let mut p = packet(1, i*50);
            p["root_pos"] = serde_json::json!([i as f32*0.001, 1., 0.]);
            p
        }).collect();
        let poses = validate(&raw).unwrap();
        let renderer = Recorder::default();
        let mut replay = PlaybackCursor::default();
        replay.feed(&renderer, &poses, 60_000);
        assert_eq!(replay.sent, 11, "wall time must not fill the future queue before playback starts");
        let mut office = cc_render_plugin::cc_render::office::controller::OfficeController::default().snapshot();
        office.avatar.native_stream = poses[0].stream.clone();
        renderer.state.lock().unwrap().office = Some(office);
        for pts in (250..=21_800).step_by(250) {
            renderer.state.lock().unwrap().office.as_mut().unwrap().avatar.displayed_pts_ms = pts;
            replay.feed(&renderer, &poses, 60_000);
            let recorded = renderer.packets.lock().unwrap();
            assert!(recorded.iter().filter(|p| p.pts_ms > pts).count() <= 10);
        }
        let recorded = renderer.packets.lock().unwrap();
        assert_eq!(recorded.len(), poses.len());
        assert!(recorded.iter().zip(&poses).all(|(a,b)|
            a.pts_ms == b.pts_ms && a.root_pos == b.root_pos && a.joint_quats == b.joint_quats));
    }

}
