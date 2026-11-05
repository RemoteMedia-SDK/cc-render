//! Render consecutive frames of a face opening and closing its mouth, off-line
//! (lockstep, full TAA history), to see how hair and skin behave in motion,
//! which a settled still hides (e.g. hair cards shimmering or sliding).
//! Usage: face_sweep avatar.glb arkit-map.json out-dir [frames] [size] [--fps N] [--cc5]
//!        [--quality low|medium|high] (low without --cc5: MSAA, no TAA)
//! Writes `out-dir/f0000.png`, ... (RGBA, premultiplied, transparent background).
//! The camera follows `CC_AVATAR_CAMERA_POS` / `_LOOK_AT` / `_FOV_DEG`.
use cc_render_plugin::cc_render::offline::OfflineRenderer;
use cc_render_plugin::cc_render::{ArkitPose, RenderQuality, RendererConfig};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    let cc5 = args.iter().position(|a| a == "--cc5").map(|i| args.remove(i)).is_some();
    let fps = match args.iter().position(|a| a == "--fps") {
        Some(i) if i + 1 < args.len() => args.drain(i..i + 2).nth(1).unwrap().parse()?,
        _ => 20,
    };
    let quality = match args.iter().position(|a| a == "--quality") {
        Some(i) if i + 1 < args.len() => match args.drain(i..i + 2).nth(1).unwrap().as_str() {
            "low" => RenderQuality::Low,
            "medium" => RenderQuality::Medium,
            _ => RenderQuality::High,
        },
        _ => RenderQuality::High,
    };
    anyhow::ensure!(args.len() >= 4, "avatar.glb arkit-map.json out-dir [frames] [size] [--fps N] [--cc5]");
    let frames: usize = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(60);
    let size: u32 = args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(480);
    let out = std::path::PathBuf::from(&args[3]);
    std::fs::create_dir_all(&out)?;
    let mut r = OfflineRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(),
        arkit_map_path: args[2].clone().into(),
        width: size,
        height: size,
        fps,
        cc5_materials: cc5,
        studio_lighting: true,
        quality: Some(quality),
        ..Default::default()
    })?;
    r.wait_ready(Duration::from_secs(180))?;
    let pose = |jaw: f32| ArkitPose { weights: [("jawOpen".to_string(), jaw)].into_iter().collect(), pts_ms: 0 };
    r.warm_up(&pose(0.0), 40)?;
    // Open and close twice, over the frames.
    for i in 0..frames {
        let t = i as f32 / frames as f32;
        let jaw = 0.4 * (1.0 - (t * 4.0 * std::f32::consts::PI).cos());
        let f = r.render(pose(jaw))?;
        image::save_buffer(out.join(format!("f{i:04}.png")), &f.pixels, f.width, f.height, image::ColorType::Rgba8)?;
    }
    r.shutdown()?;
    Ok(())
}
