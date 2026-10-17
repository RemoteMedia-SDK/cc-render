//! Render ARKit-52 face clips to mp4 with no motion worker or controller.
//! Usage: face_clips avatar.glb arkit-map.json clips.json out_dir
//! clips.json: {"fps":30,"clips":[{"name":"joy","frames":[{"mouthSmileLeft":0.7,...},...]}]}
//! Framing comes from the usual env (e.g. CC_AVATAR_FIT_FRAME=1 CC_AVATAR_FOCUS=face).
use cc_render_plugin::cc_render::{ArkitPose, CcRenderer, RendererConfig};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const W: u32 = 720;
const H: u32 = 960;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 5,
        "avatar.glb arkit-map.json clips.json out_dir required"
    );
    std::env::remove_var("CC_OFFICE");
    let spec: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[3])?)?;
    let fps = spec["fps"].as_u64().unwrap_or(30) as u32;
    let out_dir = std::path::Path::new(&args[4]);
    std::fs::create_dir_all(out_dir)?;
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(),
        arkit_map_path: args[2].clone().into(),
        width: W,
        height: H,
        fps,
        ..Default::default()
    })?;
    let start = Instant::now();
    let mut frames = Vec::new();
    while !renderer.is_ready() && start.elapsed() < Duration::from_secs(60) {
        renderer.drain_frames(&mut frames);
        frames.clear();
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = if renderer.is_ready() {
        render_clips(&renderer, &spec, fps, out_dir)
    } else {
        Err(anyhow::anyhow!("avatar did not become ready"))
    };
    renderer.shutdown()?;
    result
}

fn render_clips(
    renderer: &CcRenderer,
    spec: &serde_json::Value,
    fps: u32,
    out_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let ffmpeg = std::env::var("FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let mut pts_ms = 0u64;
    let mut frames = Vec::new();
    for clip in spec["clips"].as_array().into_iter().flatten() {
        let name = clip["name"].as_str().unwrap_or("clip");
        let poses = clip["frames"].as_array().cloned().unwrap_or_default();
        let mp4 = out_dir.join(format!("{name}.mp4"));
        let mut enc = Command::new(&ffmpeg)
            .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{W}x{H}"), "-r", &fps.to_string(), "-i", "-"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18"])
            .arg(&mp4)
            .stdin(Stdio::piped())
            .spawn()?;
        let stdin = enc.stdin.as_mut().expect("piped stdin");
        let peak_index = poses.len() * 11 / 20;
        for (i, weights) in poses.iter().enumerate() {
            pts_ms += 1;
            let weights: HashMap<String, f32> = serde_json::from_value(weights.clone())?;
            renderer.push_pose(ArkitPose { weights, pts_ms });
            let frame = wait_for_frame(renderer, &mut frames, pts_ms)?;
            stdin.write_all(&frame.pixels)?;
            if i == peak_index {
                image::save_buffer(
                    out_dir.join(format!("{name}_peak.png")),
                    &frame.pixels,
                    W,
                    H,
                    image::ColorType::Rgba8,
                )?;
            }
        }
        drop(enc.stdin.take());
        anyhow::ensure!(enc.wait()?.success(), "ffmpeg failed for {name}");
        eprintln!("wrote {}", mp4.display());
    }
    Ok(())
}

/// Waits for the first frame stamped with `pts_ms`, so each pose is captured exactly once.
fn wait_for_frame(
    renderer: &CcRenderer,
    frames: &mut Vec<cc_render_plugin::cc_render::RenderedFrame>,
    pts_ms: u64,
) -> anyhow::Result<cc_render_plugin::cc_render::RenderedFrame> {
    let start = Instant::now();
    loop {
        renderer.drain_frames(frames);
        if let Some(i) = frames.iter().position(|f| f.pts_ms == pts_ms) {
            let frame = frames.swap_remove(i);
            frames.clear();
            return Ok(frame);
        }
        frames.clear();
        anyhow::ensure!(
            start.elapsed() < Duration::from_secs(5),
            "no frame rendered for pts {pts_ms}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
