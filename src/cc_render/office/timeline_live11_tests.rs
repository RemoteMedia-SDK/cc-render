//! Exact captured live-11 regression; no inference, authored motion or scene changes.
use super::*;
use crate::cc_render::office::controller::OfficeController;

fn captured_pose(value: &serde_json::Value) -> SkeletalPose {
    // Diagnostics nest identity; native envelopes carry it at their top level.
    let stream = value.get("stream").unwrap_or(value);
    SkeletalPose {
        stream: StreamIdentity::from_envelope(stream).unwrap(),
        joint_quats: serde_json::from_value(value["joint_quats_xyzw"].clone()).unwrap(),
        root_pos: serde_json::from_value(value["root_pos"].clone()).unwrap(),
        pts_ms: value["pts_ms"].as_u64().unwrap(),
    }
}

/// Offline research diagnostic: assesses all recorded segments, never admits
/// rejected poses or advances a production timeline. Requires explicit artifacts.
#[test]
#[ignore = "requires SCENE_FIT_REPLAY_JSON from a retained research trial"]
fn scene_fit_research_path_replay() {
    let path = std::env::var("SCENE_FIT_REPLAY_JSON").expect("explicit replay artifact required");
    let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut summary = serde_json::Map::new();
    for name in ["control", "guided"] {
        let mut office = OfficeController::default();
        office.objects = serde_json::from_value(data["objects"].clone()).unwrap();
        let roots: Vec<[f32; 3]> = serde_json::from_value(data["streams"][name].clone()).unwrap();
        assert_eq!(roots.len(), 600, "full preregistered horizons required");
        let mut failures = Vec::new();
        for (index, pair) in roots.windows(2).enumerate() {
            office.observed.pelvis = pair[0];
            if !office.permits_root(pair[1].into()) {
                failures.push(index + 1);
            }
        }
        summary.insert(name.into(), serde_json::json!({
            "frames":roots.len(), "rejected_segments":failures.len(),
            "first_rejected_frame_index":failures.first(),
        }));
    }
    println!("SCENE_FIT_ROOT_REPLAY={}", serde_json::Value::Object(summary.clone()));
    assert!(summary["control"]["rejected_segments"].as_u64().unwrap() > 0);
    assert_eq!(summary["guided"]["rejected_segments"], 0);
}

#[test]
fn live11_squat_rejects_chair_clearance_without_committing_interpolated_pose() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/live11-root-rejection.json"
    ))).unwrap();
    let rejection = &fixture["rejection"];
    let previous = captured_pose(&rejection["displayed_before"]);
    let expected = captured_pose(&rejection["candidate"]);
    let frames: Vec<_> = fixture["frames"].as_array().unwrap().iter()
        .map(captured_pose).map(|p| p.normalized().unwrap()).collect();
    let mut office = OfficeController::default();
    office.objects = serde_json::from_value(fixture["geometry"]["objects"].clone()).unwrap();
    office.observed.pelvis = previous.root_pos;
    office.observed.ready = true;
    office.native_motion_revision = Some(3);
    assert_eq!(previous.pts_ms, 13023);
    assert_eq!(expected.pts_ms, 13057);
    assert_eq!(frames.iter().map(|p| p.pts_ms).collect::<Vec<_>>(), [13050, 13100]);
    assert!(office.permits_root(previous.root_pos.into()));
    assert!(office.permits_root(frames[0].root_pos.into()));
    assert!(!office.permits_root(frames[1].root_pos.into()));
    assert!(!office.permits_root(expected.root_pos.into()));

    // Identify the obstacle using the same footprint and visibility predicate,
    // without removing or relocating anything in the real acceptance scene.
    let start = Vec2::new(previous.root_pos[0], previous.root_pos[2]);
    let end = Vec2::new(expected.root_pos[0], expected.root_pos[2]);
    let scene = office.navigation_scene();
    let blocked: Vec<_> = scene.objects.iter().filter(|object| {
        let polygon = [(object.id.clone(), object.footprint.inflated_polygon(0.35))];
        !crate::cc_render::office::pathfinder::visible(start, end, &polygon)
    }).map(|object| object.id.as_str()).collect();
    assert_eq!(blocked, ["chair"]);

    let mut timeline = PoseTimeline::default();
    for frame in &frames { timeline.insert(frame.clone(), 0.); }
    timeline.displayed = Some(previous.clone());
    timeline.watermark = previous.pts_ms;
    timeline.epoch = Some((0., previous.pts_ms));
    // Small sub-ms offset avoids binary floating-point truncation to 33 ms.
    assert!(timeline.sample_checked(0.0340001, |candidate| {
        assert_eq!(candidate.pts_ms, expected.pts_ms);
        for (actual, captured) in candidate.root_pos.iter().zip(expected.root_pos) {
            assert!((actual - captured).abs() < 1e-6);
        }
        for (actual, captured) in candidate.joint_quats.iter().flatten()
            .zip(expected.joint_quats.iter().flatten()) {
            assert!((actual - captured).abs() < 1e-5);
        }
        office.permits_root(candidate.root_pos.into())
    }).is_none());
    assert_eq!(timeline.watermark, previous.pts_ms);
    assert_eq!(timeline.displayed.as_ref().unwrap().root_pos, previous.root_pos);
    assert_eq!(timeline.displayed.as_ref().unwrap().joint_quats, previous.joint_quats);
    assert!(timeline.queue.is_empty());
    assert_eq!(timeline.rejected_revision, Some(3));
    assert_eq!(timeline.rejection.as_ref().unwrap()["rejected_pts_ms"], 13057);
    timeline.insert(frames[1].clone(), 1.);
    assert!(timeline.queue.is_empty(), "rejected revision must not restart itself");
    assert_eq!(timeline.watermark, previous.pts_ms);
}
