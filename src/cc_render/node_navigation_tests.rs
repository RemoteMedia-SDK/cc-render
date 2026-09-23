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
