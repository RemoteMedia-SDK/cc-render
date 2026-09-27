use super::*;
use crate::cc_render::{
    office::controller::OfficeController,
    renderer::{RenderedFrame, SceneState, SkeletalPose},
};

struct SceneRenderer(SceneState);
impl Renderer for SceneRenderer {
    fn push_pose(&self, _: ArkitPose) {
        panic!("query must not send poses")
    }
    fn push_skeletal_pose(&self, _: Option<SkeletalPose>) {
        panic!("query must not send motion")
    }
    fn drain_frames(&self, _: &mut Vec<RenderedFrame>) {
        panic!("query must not drain frames")
    }
    fn office_command(&self, _: crate::cc_render::office::controller::OfficeCommand) {
        panic!("query must not enqueue an action")
    }
    fn scene_state(&self) -> SceneState {
        self.0.clone()
    }
}

#[tokio::test]
async fn native_and_python_queries_return_paths_without_enqueuing_motion() {
    let mut controller = OfficeController::default();
    controller.observed.ready = true;
    controller.observed.pelvis = [0., 0.95, 0.];
    let node = CcRenderNode::with_renderer(
        CcRenderConfig::default(),
        Arc::new(SceneRenderer(SceneState {
            office: Some(controller.snapshot()),
            ..Default::default()
        })),
    );
    let query = serde_json::json!({"kind":"generate_waypoint","from":"avatar","to":"bed","request_id":"route-1"});
    for input in [
        RuntimeData::Json(query.clone()),
        RuntimeData::Text(remotemedia_plugin_sdk::types::tag_text_str(
            &query.to_string(),
            "json",
        )),
    ] {
        let mut replies = Vec::new();
        let count = node
            .process_streaming(input, None, |reply| {
                replies.push(reply);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(replies.len(), 1);
        let RuntimeData::Json(reply) = &replies[0] else {
            panic!("expected JSON")
        };
        assert_eq!(reply["kind"], "waypoint_path");
        assert_eq!(reply["request_id"], "route-1");
        assert_eq!(reply["ok"], true);
        assert!(reply["path"]["waypoints"].as_array().unwrap().len() > 2);
    }
    let mut replies = Vec::new();
    node.process_streaming(
        RuntimeData::Json(serde_json::json!({"kind":"get_scene_state","request_id":"state-1"})),
        None,
        |reply| {
            replies.push(reply);
            Ok(())
        },
    )
    .await
    .unwrap();
    let RuntimeData::Json(reply) = &replies[0] else {
        panic!("expected JSON")
    };
    assert_eq!(reply["request_id"], "state-1");
    assert!(reply["state"]["office"]["active"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn scene_queries_expose_native_session_seed_and_revision_not_retired_plan() {
    let node = CcRenderNode::with_renderer(
        CcRenderConfig::default(),
        Arc::new(SceneRenderer(SceneState {
            office: Some(OfficeController::default().snapshot()),
            ..Default::default()
        })),
    );
    assert!(node.observed_scene()["office"]["motion"].is_null());
    let goal = serde_json::json!({"kind":"motion_goal","version":2,"session_id":"owner",
        "generation_revision":2,"pts_ms":8000,"seed":42,
        "goal":{"description":"A person stands up from a squat."}});
    node.process_streaming(RuntimeData::Json(goal.clone()), None, |_| Ok(())).await.unwrap();
    let mut replies = Vec::new();
    node.process_streaming(RuntimeData::Json(serde_json::json!({"kind":"get_scene_state"})),
        None, |reply| { replies.push(reply); Ok(()) }).await.unwrap();
    let RuntimeData::Json(reply) = &replies[0] else { panic!("expected JSON") };
    assert_eq!(reply["state"]["office"]["motion"], goal);
    assert!(reply["state"]["office"]["motion"].get("root_waypoints").is_none());
    let mut foreign = goal;
    foreign["seed"] = 43.into();
    assert!(node.process_streaming(RuntimeData::Json(foreign), None, |_| Ok(())).await.is_err());
    assert_eq!(node.observed_scene()["office"]["motion"]["seed"], 42);
}

#[tokio::test]
async fn stalled_native_clock_emits_one_terminal_error_from_node_without_new_requests() {
    struct FrozenOffice(SceneState);
    impl Renderer for FrozenOffice {
        fn push_pose(&self, _: ArkitPose) {}
        fn push_skeletal_pose(&self, _: Option<SkeletalPose>) {}
        fn drain_frames(&self, _: &mut Vec<RenderedFrame>) {}
        fn scene_state(&self) -> SceneState { self.0.clone() }
    }
    let mut node = CcRenderNode::with_renderer(CcRenderConfig::default(), Arc::new(FrozenOffice(SceneState {
        office: Some(OfficeController::default().snapshot()), ..Default::default()
    })));
    let goal = serde_json::json!({"kind":"motion_goal","version":2,"session_id":"owner",
        "generation_revision":1,"pts_ms":0,"seed":42,"goal":{"description":"A person waves hello."}});
    node.process_streaming(RuntimeData::Json(goal.clone()), None, |_| Ok(())).await.unwrap();
    let outputs = Arc::new(parking_lot::Mutex::new(Vec::new()));
    for tick in 0..2 {
        if tick == 1 { node.motion_clock -= std::time::Duration::from_millis(120_001); }
        let out = outputs.clone();
        node.tick(Tick { clock_id:"avatar".into(),pts_us:tick*33_333,frame_idx:tick,deadline_us:0 },
            None, Box::new(move |v| { out.lock().push(v); Ok(()) })).await.unwrap();
    }
    for tick in 2..100 {
        let out = outputs.clone();
        node.tick(Tick { clock_id:"avatar".into(),pts_us:tick*33_333,frame_idx:tick,deadline_us:0 },
            None, Box::new(move |v| { out.lock().push(v); Ok(()) })).await.unwrap();
    }
    let replies = outputs.lock();
    assert_eq!(replies.iter().filter(|v| matches!(v, RuntimeData::Json(j) if j["kind"] == "motion_goal")).count(), 1);
    assert_eq!(replies.iter().filter(|v| matches!(v, RuntimeData::Json(j) if j["kind"] == "motion_error" && j["error"].as_str().unwrap().contains("stalled"))).count(), 1);
    drop(replies);
    assert!(node.process_streaming(RuntimeData::Json(goal), None, |_| Ok(())).await.is_err());
}

#[tokio::test]
async fn observed_rejection_crosses_node_boundary_once_and_halts_request_driver() {
    struct LiveScene(parking_lot::Mutex<SceneState>);
    impl Renderer for LiveScene {
        fn push_pose(&self, _: ArkitPose) {}
        fn push_skeletal_pose(&self, _: Option<SkeletalPose>) {}
        fn drain_frames(&self, _: &mut Vec<RenderedFrame>) {}
        fn scene_state(&self) -> SceneState { self.0.lock().clone() }
    }
    let renderer = Arc::new(LiveScene(parking_lot::Mutex::new(SceneState {
        office: Some(OfficeController::default().snapshot()), ..Default::default()
    })));
    let node = CcRenderNode::with_renderer(CcRenderConfig::default(), renderer.clone());
    let goal = serde_json::json!({"kind":"motion_goal","version":2,"session_id":"owner",
        "generation_revision":1,"pts_ms":0,"seed":42,"goal":{"description":"A person squats down."}});
    node.process_streaming(RuntimeData::Json(goal.clone()), None, |_| Ok(())).await.unwrap();
    let output = Arc::new(parking_lot::Mutex::new(Vec::new()));
    for frame in 0..100 {
        if frame == 1 {
            let mut scene = renderer.0.lock();
            let office = scene.office.as_mut().unwrap();
            office.motion_error = Some("native root validation rejected frame=75 displayed=0".into());
            office.motion_rejection = Some(serde_json::json!({"rejected_pts_ms":75,"displayed_pts_ms":0}));
            // Even a late worker acknowledgement cannot restart a failed driver.
            node.process_streaming(RuntimeData::Json(serde_json::json!({"kind":"motion_tick",
                "session_id":"owner","generation_revision":1,"pts_ms":0})), None, |_| Ok(())).await.unwrap();
        }
        let out = output.clone();
        node.tick(Tick { clock_id:"avatar".into(),pts_us:frame*33_333,frame_idx:frame,deadline_us:0 },
            None, Box::new(move |v| { out.lock().push(v); Ok(()) })).await.unwrap();
    }
    let out = output.lock();
    assert_eq!(out.iter().filter(|v| matches!(v, RuntimeData::Json(j) if j["kind"] == "motion_goal")).count(), 1);
    let errors: Vec<_> = out.iter().filter_map(|v| match v {
        RuntimeData::Json(j) if j["kind"] == "motion_error" => Some(j), _ => None,
    }).collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["rejection"]["rejected_pts_ms"], 75);
    drop(out);
    assert!(node.process_streaming(RuntimeData::Json(goal), None, |_| Ok(())).await.is_err());
}
