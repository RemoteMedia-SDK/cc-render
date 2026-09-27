//! Offline production-packet playback check; no renderer/GPU or motion rewriting.
//! Usage: cargo run --example verify_native_motion_stream -- block0.json block1.json
use cc_render_plugin::cc_render::{office::timeline::PoseTimeline, CcRenderNode};

fn main() -> anyhow::Result<()> {
    let mut blocks = Vec::new();
    for path in std::env::args().skip(1) {
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let frames = value["frames"].as_array().ok_or_else(|| anyhow::anyhow!("missing frames"))?;
        anyhow::ensure!(frames.len() == 40, "expected complete Core40 block");
        let poses = frames.iter().map(|f| {
            CcRenderNode::skeletal_pose_from_json(f).ok_or_else(|| anyhow::anyhow!("invalid packet"))
        }).collect::<anyhow::Result<Vec<_>>>()?;
        blocks.push(poses);
    }
    anyhow::ensure!(blocks.len() == 2, "supply exactly two native response files");
    let mut timeline = PoseTimeline::default();
    for pose in &blocks[0] {
        timeline.insert(pose.clone(), 0.);
    }
    let expected: Vec<_> = blocks.iter().flatten().collect();
    for (i, pose) in expected.iter().enumerate() {
        if i == 28 {
            for next in &blocks[1] {
                timeline.insert(next.clone(), 1.35);
            }
        }
        let played = timeline.sample(i as f64 / 20.).ok_or_else(|| anyhow::anyhow!("missing playback"))?;
        anyhow::ensure!(played.pts_ms.abs_diff(pose.pts_ms) <= 1, "native timestamp lost");
        for (a, b) in played.root_pos.iter().zip(pose.root_pos) {
            anyhow::ensure!((a - b).abs() < 1e-5, "root changed");
        }
        for (a, b) in played.joint_quats.iter().zip(pose.joint_quats) {
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            anyhow::ensure!((dot.abs() - 1.).abs() < 1e-5, "joint rotation changed");
        }
    }
    anyhow::ensure!(timeline.stale_frames == 0, "native frames discarded");
    println!("Verified all 80 native frames through production parser/timeline; no dropped tail.");
    Ok(())
}
