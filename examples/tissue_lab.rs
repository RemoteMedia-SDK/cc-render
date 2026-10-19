//! Plain studio, fixed or live clock, unchanged native motion, GPU-correlated PNGs.
#[path = "condo_capture/native_recording.rs"]
mod native_recording;
use cc_render_plugin::cc_render::{CcRenderer, PhysicsConfig, RendererConfig};
use std::{io::Write, path::Path, time::{Duration, Instant}};

#[derive(Clone, Copy)]
struct CaptureSettings {
    fps: u32,
    live_clock: bool,
    tissue: bool,
    save_images: bool,
    /// Scripted rig packets (synthetic-rig.json beside them) are not office
    /// motion: they play without the office runtime and its acceptance guards.
    synthetic: bool,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 7,
        "avatar.glb arkit-map.json packets.ndjson tissue.json|off front|side|chest-front|chest-side output-directory");
    let out = Path::new(&args[6]);
    anyhow::ensure!(!out.exists(), "use a new output directory to preserve evidence");
    let poses = native_recording::load(Path::new(&args[3]))?;
    // load verifies one session, monotone revisions, unchanged service packets
    // and the service-owned history anchor at every revision handoff.
    // PlaybackCursor uses that existing handoff protocol without editing poses.
    std::fs::create_dir_all(out)?;
    let fps = std::env::var("CC_TISSUE_LAB_FPS").unwrap_or_else(|_| "60".into()).parse::<u32>()?;
    anyhow::ensure!([30, 60].contains(&fps), "lab output cadence must be 30 or 60 FPS");
    let live_clock = match std::env::var("CC_TISSUE_LAB_CLOCK").as_deref().unwrap_or("fixed") {
        "fixed" => false,
        "live" => true,
        _ => anyhow::bail!("lab clock must be fixed or live"),
    };
    anyhow::ensure!(!live_clock || std::env::var("CC_RENDER_FAST").as_deref() != Ok("1"),
        "live-clock lab requires normal renderer pacing; use fixed clock for uncapped profiling");
    let settings = CaptureSettings {
        fps,
        live_clock,
        tissue: args[4] != "off",
        save_images: std::env::var("CC_TISSUE_LAB_TRACE_ONLY").as_deref() != Ok("1"),
        synthetic: Path::new(&args[3]).with_file_name("synthetic-rig.json").exists(),
    };
    configure(&args, out, settings)?;
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(), arkit_map_path: args[2].clone().into(),
        width: 720, height: 900, fps,
        offline_frame_step: (!live_clock).then(|| Duration::from_secs_f64(1.0 / fps as f64)),
        physics: Some(PhysicsConfig { play_volume: None,
            sidecar_path: Some(out.join("physics.json")), ..Default::default() }),
        ..Default::default()
    })?;
    let result = capture(&renderer, &poses, out, settings);
    renderer.shutdown()?;
    result
}

fn configure(args: &[String], out: &Path, settings: CaptureSettings) -> anyhow::Result<()> {
    let CaptureSettings { fps, live_clock, tissue, save_images, synthetic } = settings;
    // Chest views frame the breasts at the pivot the orientation rig turns about.
    let (camera, target) = match args[5].as_str() {
        "front" => ("0,1.55,2.6", [0., 1.5, 0.]),
        "side" => ("2.6,1.55,0", [0., 1.5, 0.]),
        "chest-front" => ("0,1.2,1.3", [0., 1.2, 0.]),
        "chest-side" => ("1.3,1.2,0.05", [0., 1.2, 0.05]),
        _ => anyhow::bail!("camera must be front, side, chest-front or chest-side"),
    };
    let look_at = target.map(|v: f64| v.to_string()).join(",");
    // An empty office provides existing native-stream observation/validation,
    // without loading the bundled home or any condo geometry.
    std::fs::write(out.join("empty-scene.json"), "[]")?;
    std::fs::write(out.join("physics.json"),
        r#"{"version":1,"secondary_motion":{"auto_detect_patterns":[],"chains_override":[]}}"#)?;
    for (key, value) in [("CC_OFFICE", "1"), ("CC_AVATAR_FOLLOW", "0"),
        ("CC_AVATAR_AUTO_FACE_CAMERA", "0"), ("CC_AVATAR_FIT_FRAME", "0"),
        ("CC_AVATAR_RETARGET_MODE", "calibrated"), ("CC_AVATAR_CAMERA_POS", camera),
        ("CC_AVATAR_CAMERA_LOOK_AT", look_at.as_str()), ("CC_AVATAR_CAMERA_FOV_DEG", "38"),
        ("CC_AVATAR_CLEAR_COLOR", "0.12,0.14,0.17")] {
        std::env::set_var(key, value);
    }
    if synthetic {
        // Same studio light as the office preview, without office motion validation.
        std::env::remove_var("CC_OFFICE");
        std::env::set_var("CC_AVATAR_AMBIENT", "500");
    } else {
        std::env::set_var("CC_OFFICE_SCENE", out.join("empty-scene.json"));
    }
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
    // Preserve the source responses required to authenticate revision handoffs
    // when replaying the saved capture later.
    for name in ["requests.ndjson", "responses.ndjson", "attempt-plan.json", "source-audit.json", "synthetic-rig.json"] {
        let source = Path::new(&args[3]).with_file_name(name);
        if source.exists() {
            std::fs::copy(source, out.join(name))?;
        }
    }
    // The orientation rig (scripts/avatars/tissue_orientation_lab.py) marks its
    // scripted rigid-body packets; they are a physics test, never ARDY motion.
    let motion = if synthetic {
        "synthetic rigid-body test rig (not ARDY)"
    } else {
        "unchanged native recording"
    };
    std::fs::write(out.join("config.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "inputs":inputs,"tissue":tissue,"camera":args[5],"camera_position":camera,
        "camera_target":target,"camera_fov_deg":38,"fps":fps,
        "resolution":[720,900],"offline":!live_clock,"legacy_secondary_bones":false,"save_images":save_images,
        "clock":if live_clock {"Bevy realtime"} else {"fixed source-time step"},
        "uncapped_scheduler":std::env::var("CC_RENDER_FAST").as_deref() == Ok("1"),
        "tissue_hz":std::env::var("CC_AVATAR_TISSUE_HZ").unwrap_or_else(|_| "120".into()),
        "tissue_iterations":std::env::var("CC_AVATAR_TISSUE_ITERATIONS").unwrap_or_else(|_| "32".into()),
        "scene":if synthetic {"empty studio, office runtime off"} else {"empty studio"},"motion":motion}))?)?;
    Ok(())
}

fn capture(renderer: &CcRenderer, poses: &[cc_render_plugin::cc_render::renderer::SkeletalPose],
    out: &Path, settings: CaptureSettings) -> anyhow::Result<()> {
    let CaptureSettings { fps, live_clock, tissue, save_images, synthetic } = settings;
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
        // Bound transport by displayed source time, never wall time. Without
        // the office there is no observed clock, so use the captured one.
        if synthetic {
            // One scripted revision: no handoffs, so pace it directly.
            let deadline = last.unwrap_or(poses[0].pts_ms) + 500;
            while replay.sent < poses.len() && poses[replay.sent].pts_ms <= deadline {
                renderer.push_skeletal_pose(Some(poses[replay.sent].clone()));
                replay.sent += 1;
            }
        } else {
            replay.feed(renderer, poses, u64::MAX);
        }
        if let Some(error) = renderer.scene_state().office.and_then(|office| office.motion_error) {
            anyhow::bail!("office stopped the motion: {error}");
        }
        renderer.drain_frames(&mut frames);
        for frame in frames.drain(..) {
            let Some(pts) = frame.body_pts_ms else { continue };
            if last == Some(pts) { continue; }
            // Live rendering samples the source at variable wall-clock times.
            // Record its actual gaps for analysis; retain the strict fixed-clock
            // frame-completeness check for deterministic offline comparisons.
            anyhow::ensure!(last.is_none_or(|old| pts > old && (live_clock || pts - old <= maximum_source_gap_ms)),
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
