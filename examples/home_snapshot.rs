//! Render the bundled home without requiring a motion or speech worker.
//! Usage: cargo run --example home_snapshot -- avatar.glb arkit-map.json output.png
//! Camera environment overrides also allow close inspection of individual rooms.
use cc_render_plugin::cc_render::renderer::PhysicsConfig;
use cc_render_plugin::cc_render::{CcRenderer, RendererConfig};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 4,
        "avatar.glb arkit-map.json output.png required"
    );
    std::env::set_var("CC_OFFICE", "1");
    std::env::set_var("CC_AVATAR_AUTO_FACE_CAMERA", "0");
    std::env::set_var("CC_AVATAR_FIT_FRAME", "0");
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(),
        arkit_map_path: args[2].clone().into(),
        width: 1280,
        height: 960,
        fps: 15,
        physics: Some(PhysicsConfig::default()),
        ..Default::default()
    })?;
    let started = Instant::now();
    let mut frames = Vec::new();
    let mut ready_frames = 0;
    let result = loop {
        renderer.drain_frames(&mut frames);
        if renderer.is_ready() {
            ready_frames += frames.len();
            if ready_frames >= 12 {
                if let Some(frame) = frames.last() {
                    break image::save_buffer(
                        &args[3],
                        &frame.pixels,
                        frame.width,
                        frame.height,
                        image::ColorType::Rgba8,
                    )
                    .map_err(anyhow::Error::from);
                }
            }
        }
        if started.elapsed() > Duration::from_secs(180) {
            break Err(anyhow::anyhow!(
                "timed out waiting for rendered home frames"
            ));
        }
        frames.clear();
        std::thread::sleep(Duration::from_millis(50));
    };
    renderer.shutdown()?;
    result?;
    println!("Saved {}", args[3]);
    Ok(())
}
