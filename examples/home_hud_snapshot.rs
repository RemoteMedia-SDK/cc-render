//! Render the bundled home with a fixture home-task overlay, without a motion
//! or speech worker. The overlay is display-only producer state.
//! Usage: cargo run --example home_hud_snapshot -- avatar.glb arkit-map.json output.png [hud.json]
use cc_render_plugin::cc_render::office::home_hud;
use cc_render_plugin::cc_render::renderer::PhysicsConfig;
use cc_render_plugin::cc_render::{CcRenderer, RendererConfig};
use std::time::{Duration, Instant};

fn fixture() -> serde_json::Value {
    serde_json::json!({"kind":"home_hud","version":1,"hud":{
        "needs":[
            {"id":"hunger","label":"Hunger","value":0.28},
            {"id":"energy","label":"Energy","value":0.74},
            {"id":"hygiene","label":"Hygiene","value":0.55},
            {"id":"bladder","label":"Bladder","value":0.9},
            {"id":"fun","label":"Fun","value":0.4}],
        "current":{"label":"Set the table","steps":[
            {"label":"Walk to the kitchen sink","status":"done"},
            {"label":"Pick up the plate","status":"running"},
            {"label":"Walk to the dining table","status":"pending"},
            {"label":"Put the plate on the table","status":"blocked"}],
            "detail":"place: unsupported_capability"},
        "queue":["Watch TV","Nap"],"autonomy":true}})
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        (4..=5).contains(&args.len()),
        "avatar.glb arkit-map.json output.png [hud.json] required"
    );
    let hud = match args.get(4) {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => fixture(),
    };
    let hud = home_hud::parse(&hud).map_err(anyhow::Error::msg)?;
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
    renderer.home_hud(hud);
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
            break Err(anyhow::anyhow!("timed out waiting for rendered home frames"));
        }
        frames.clear();
        std::thread::sleep(Duration::from_millis(50));
    };
    renderer.shutdown()?;
    result?;
    println!("Saved {}", args[3]);
    Ok(())
}
