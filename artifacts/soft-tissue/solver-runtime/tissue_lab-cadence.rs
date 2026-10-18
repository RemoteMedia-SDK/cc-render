//! Plain studio, fixed clock, unchanged native motion, GPU-correlated PNGs.
#[path = "condo_capture/native_recording.rs"]
mod native_recording;
use cc_render_plugin::cc_render::{CcRenderer, PhysicsConfig, RendererConfig};
use std::{io::Write, path::Path, time::{Duration, Instant}};

#[derive(Clone, Copy)]
struct CaptureSettings {
    fps: u32,
    tissue: bool,
    save_images: bool,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 7,
        "avatar.glb arkit-map.json packets.ndjson tissue.json|off front|side output-directory");
    let out = Path::new(&args[6]);
    anyhow::ensure!(!out.exists(), "use a new output directory to preserve evidence");
    let poses = native_recording::load(Path::new(&args[3]))?;
    // Keep the lab's first benchmark to one immutable generation stream.
    let stream = poses[0].stream.as_ref().expect("validated native identity");
    anyhow::ensure!(poses.iter().all(|p| p.stream.as_ref().is_some_and(|s|
        s.session_id == stream.session_id && s.generation_revision == stream.generation_revision)),
        "lab requires one native revision");
    std::fs::create_dir_all(out)?;
    let fps = std::env::var("CC_TISSUE_LAB_FPS").unwrap_or_else(|_| "60".into()).parse::<u32>()?;
    anyhow::ensure!([30, 60].contains(&fps), "lab output cadence must be 30 or 60 FPS");
    let settings = CaptureSettings {
        fps,
        tissue: args[4] != "off",
        save_images: std::env::var("CC_TISSUE_LAB_TRACE_ONLY").as_deref() != Ok("1"),
    };
    configure(&args, out, settings)?;
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(), arkit_map_path: args[2].clone().into(),
        width: 720, height: 900, fps,
        offline_frame_step: Some(Duration::from_secs_f64(1.0 / fps as f64)),
        physics: Some(PhysicsConfig { play_volume: None,
            sidecar_path: Some(out.join("physics.json")), ..Default::default() }),
        ..Default::default()
    })?;
    let result = capture(&renderer, &poses, out, settings);
    renderer.shutdown()?;
    result
}

fn configure(args: &[String], out: &Path, settings: CaptureSettings) -> anyhow::Result<()> {
    let CaptureSettings { fps, tissue, save_images } = settings;
    let camera = match args[5].as_str() {
        "front" => "0,1.55,2.6",
        "side" => "2.6,1.55,0",
        _ => anyhow::bail!("camera must be front or side"),
    };
    // An empty office provides existing native-stream observation/validation,
    // without loading the bundled home or any condo geometry.
    std::fs::write(out.join("empty-scene.json"), "[]")?;
    std::fs::write(out.join("physics.json"),
        r#"{"version":1,"secondary_motion":{"auto_detect_patterns":[],"chains_override":[]}}"#)?;
    for (key, value) in [("CC_OFFICE", "1"), ("CC_AVATAR_FOLLOW", "0"),
        ("CC_AVATAR_AUTO_FACE_CAMERA", "0"), ("CC_AVATAR_FIT_FRAME", "0"),
        ("CC_AVATAR_RETARGET_MODE", "calibrated"), ("CC_AVATAR_CAMERA_POS", camera),
        ("CC_AVATAR_CAMERA_LOOK_AT", "0,1.5,0"), ("CC_AVATAR_CAMERA_FOV_DEG", "38"),
        ("CC_AVATAR_CLEAR_COLOR", "0.12,0.14,0.17")] {
        std::env::set_var(key, value);
    }
    std::env::set_var("CC_OFFICE_SCENE", out.join("empty-scene.json"));
    if tissue {
        std::env::set_var("CC_AVATAR_TISSUE_ASSET", &args[4]);
        std::env::set_var("CC_AVATAR_TISSUE_TRACE", out.join("tissue.ndjson"));
        std::fs::copy(&args[4], out.join("tissue-asset.json"))?;
    } else {
        std::env::remove_var("CC_AVATAR_TISSUE_ASSET");
        std::env::remove_var("CC_AVATAR_TISSUE_TRACE");
    }
    let mut inputs = Vec::new();
    for path in [&args[1], &args[2], &args[3]] {
        let bytes = std::fs::read(path)?;
        let hash: String = hmac_sha256::Hash::hash(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        inputs.push(serde_json::json!({"path":path,"sha256":hash}));
    }
    std::fs::copy(&args[3], out.join("packets.ndjson"))?;
    std::fs::write(out.join("config.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "inputs":inputs,"tissue":tissue,"camera":args[5],"camera_position":camera,
        "camera_target":[0.,1.5,0.],"camera_fov_deg":38,"fps":fps,
        "resolution":[720,900],"offline":true,"legacy_secondary_bones":false,"save_images":save_images,
        "tissue_hz":std::env::var("CC_AVATAR_TISSUE_HZ").unwrap_or_else(|_| "120".into()),
        "tissue_iterations":std::env::var("CC_AVATAR_TISSUE_ITERATIONS").unwrap_or_else(|_| "8".into()),
        "scene":"empty studio","motion":"unchanged native recording"}))?)?;
    Ok(())
}

fn capture(renderer: &CcRenderer, poses: &[cc_render_plugin::cc_render::renderer::SkeletalPose],
    out: &Path, settings: CaptureSettings) -> anyhow::Result<()> {
    let CaptureSettings { fps, tissue, save_images } = settings;
    let maximum_source_gap_ms = 1000_u64.div_ceil(u64::from(fps));
    let started = Instant::now();
    let mut frames = Vec::new();
    while !renderer.is_ready() {
        anyhow::ensure!(started.elapsed() < Duration::from_secs(90), "readiness timeout");
        renderer.drain_frames(&mut frames);
        frames.clear();
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut replay = native_recording::PlaybackCursor::default();
    let mut times = std::fs::File::create(out.join("frames.ndjson"))?;
    let mut last = None;
    let mut count = 0;
    let end = poses.last().unwrap().pts_ms;
    while last != Some(end) {
        anyhow::ensure!(renderer.is_ready(), "renderer withdrew readiness; preserve log");
        anyhow::ensure!(started.elapsed() < Duration::from_secs(600), "capture timeout");
        // Bound transport by displayed source time, never wall time.
        replay.feed(renderer, poses, u64::MAX);
        renderer.drain_frames(&mut frames);
        for frame in frames.drain(..) {
            let Some(pts) = frame.body_pts_ms else { continue };
            if last == Some(pts) { continue; }
            anyhow::ensure!(last.is_none_or(|old| pts > old && pts - old <= maximum_source_gap_ms),
                "capture lost a source frame: {last:?} -> {pts}");
            anyhow::ensure!(!tissue || frame.tissue_pts_ms == Some(pts), "body/tissue capture clock mismatch");
            let name = save_images.then(|| format!("{count:06}.png"));
            if let Some(name) = &name {
                image::save_buffer(out.join(name), &frame.pixels, frame.width, frame.height, image::ColorType::Rgba8)?;
            }
            writeln!(times, "{}", serde_json::json!({"frame":count,"pts_ms":pts,"image":name,
                "wall_elapsed_ms":started.elapsed().as_secs_f64()*1000.}))?;
            count += 1;
            last = Some(pts);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    anyhow::ensure!(replay.sent == poses.len(), "source packets were not all delivered");
    std::fs::write(out.join("capture.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "frames":count,"last_pts_ms":last,"sent_packets":replay.sent,"images_saved":if save_images {count} else {0},
        "wall_seconds":started.elapsed().as_secs_f64(),"realtime_performance_claim":false}))?)?;
    Ok(())
}
