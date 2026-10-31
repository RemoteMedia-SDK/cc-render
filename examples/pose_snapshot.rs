//! Render one recorded pose without a motion worker, controller or contact IK.
//! Usage: pose_snapshot avatar.glb arkit-map.json pose.json output.png [width height]
//!        [--arkit name=weight,...] [--cc5]
//! `--arkit` holds ARKit blendshape weights on the face (e.g. `jawOpen=0.6`);
//! `--cc5` renders `extras.cc5` materials with the CC5 shaders.
use cc_render_plugin::cc_render::{ArkitPose, CcRenderNode, CcRenderer, RendererConfig};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let mut args: Vec<_> = std::env::args().collect();
    let arkit = match args.iter().position(|a| a == "--arkit") {
        Some(i) if i + 1 < args.len() => args.drain(i..i + 2).nth(1).unwrap(),
        Some(_) => anyhow::bail!("--arkit needs name=weight,..."),
        None => String::new(),
    };
    let weights = arkit
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').ok_or_else(|| anyhow::anyhow!("--arkit {kv}: expected name=weight"))?;
            Ok((k.to_string(), v.parse::<f32>()?))
        })
        .collect::<anyhow::Result<std::collections::HashMap<_, _>>>()?;
    let cc5 = args.iter().position(|a| a == "--cc5").map(|i| args.remove(i)).is_some();
    anyhow::ensure!(
        args.len() == 5 || args.len() == 7,
        "avatar.glb arkit-map.json pose.json output.png [width height] required"
    );
    let (width, height) = if args.len() == 7 {
        (args[5].parse()?, args[6].parse()?)
    } else {
        (640, 480)
    };
    std::env::remove_var("CC_OFFICE");
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(),
        arkit_map_path: args[2].clone().into(),
        width,
        height,
        fps: 30,
        cc5_materials: cc5,
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
        if !weights.is_empty() {
            renderer.push_pose(ArkitPose { weights, pts_ms: 0 });
        }
        // the CC5 shaders compile once their maps load, and TAA converges
        std::thread::sleep(Duration::from_secs(if cc5 { 8 } else { 1 }));
        renderer.drain_frames(&mut frames);
        if let Some(frame) = frames.last() {
            image::save_buffer(&args[4], &frame.pixels, width, height, image::ColorType::Rgba8)
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
