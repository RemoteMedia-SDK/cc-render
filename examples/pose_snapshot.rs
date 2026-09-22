//! Render one recorded pose without a motion worker, controller or contact IK.
//! Usage: pose_snapshot avatar.glb arkit-map.json pose.json output.png
use cc_render_plugin::cc_render::{CcRenderNode, CcRenderer, RendererConfig};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 5,
        "avatar.glb arkit-map.json pose.json output.png required"
    );
    std::env::remove_var("CC_OFFICE");
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(),
        arkit_map_path: args[2].clone().into(),
        width: 640,
        height: 480,
        fps: 30,
        ..Default::default()
    })?;
    let value = serde_json::from_slice(&std::fs::read(&args[3])?)?;
    let pose = CcRenderNode::skeletal_pose_from_json(&value)
        .ok_or_else(|| anyhow::anyhow!("invalid skeletal pose"))?;
    let start = Instant::now();
    let mut frames = Vec::new();
    while !renderer.is_ready() && start.elapsed() < Duration::from_secs(30) {
        renderer.drain_frames(&mut frames);
        frames.clear();
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = if renderer.is_ready() {
        renderer.push_skeletal_pose(Some(pose));
        std::thread::sleep(Duration::from_secs(1));
        renderer.drain_frames(&mut frames);
        if let Some(frame) = frames.last() {
            image::save_buffer(&args[4], &frame.pixels, 640, 480, image::ColorType::Rgba8)
                .map_err(anyhow::Error::from)
        } else {
            Err(anyhow::anyhow!("renderer produced no frames"))
        }
    } else {
        Err(anyhow::anyhow!("avatar did not become ready"))
    };
    renderer.shutdown()?;
    result
}
