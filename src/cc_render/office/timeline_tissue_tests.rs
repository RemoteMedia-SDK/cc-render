use super::*;

fn pose(at: u64, revision: u64, x: f32) -> SkeletalPose {
    let mut pose = SkeletalPose::rest();
    pose.pts_ms = at;
    pose.root_pos = [x, 1.0, 0.0];
    pose.stream = Some(StreamIdentity {
        session_id: "tissue-test".into(),
        generation_revision: revision,
        skeleton_id: "smpl22".into(),
        root_mode: "world".into(),
    });
    pose
}

#[test]
fn tissue_window_retains_source_corners_skipped_by_rendering() {
    let mut timeline = PoseTimeline::default();
    for (at, x) in [(0, 0.0), (50, 0.02), (100, -0.01), (150, 0.0)] {
        timeline.insert(pose(at, 1, x), 0.0);
    }
    timeline.sample_checked(0.125, |_| true).unwrap();
    let window = timeline.motion_window(Some(0.0)).unwrap();
    assert_eq!(window.displayed_ms, 125);
    assert_eq!(
        window.knots.iter().map(|p| p.pts_ms).collect::<Vec<_>>(),
        [0, 50, 100, 150]
    );
    assert_eq!(window.knots[1].root_pos[0], 0.02);
}

#[test]
fn tissue_window_preserves_pause_and_revision_boundaries_without_extrapolation() {
    let mut timeline = PoseTimeline::default();
    timeline.insert(pose(0, 1, 0.0), 0.0);
    timeline.insert(pose(100, 1, 1.0), 0.0);
    let held = timeline.sample(0.025).unwrap();
    timeline.schedule_revision(2, None);
    timeline.sample(0.5);
    let paused = timeline.motion_window(None).unwrap();
    assert_eq!(paused.displayed_ms, 25);
    assert_eq!(paused.knots.last().unwrap().root_pos, held.root_pos);
    timeline.insert(pose(75, 2, 2.0), 1.0);
    timeline.sample(1.025);
    let window = timeline.motion_window(Some(25.0)).unwrap();
    assert_eq!(window.knots[0].root_pos, held.root_pos);
    assert_eq!(window.knots[0].pts_ms, 25);
    assert_eq!(window.knots.last().unwrap().pts_ms, 75);
}

#[test]
fn tissue_window_rolls_back_rejected_source_knots() {
    let mut timeline = PoseTimeline::default();
    timeline.insert(pose(0, 1, 0.0), 0.0);
    timeline.insert(pose(50, 1, 1.0), 0.0);
    timeline.sample_checked(0.0, |_| true).unwrap();
    assert!(timeline.sample_checked(0.05, |_| false).is_none());
    let window = timeline.motion_window(None).unwrap();
    assert_eq!(window.displayed_ms, 0);
    assert_eq!(window.knots.len(), 1);
    assert_eq!(window.knots[0].pts_ms, 0);
}
