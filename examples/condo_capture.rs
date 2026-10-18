//! Render a scene package with the actual CC5 avatar. Optional native packet replay
//! is unchanged recorded motion; absent packets means bind-pose visual inspection.
#[path = "condo_capture/native_recording.rs"]
mod native_recording;
#[path = "condo_capture/weather_recording.rs"]
mod weather_recording;
use cc_render_plugin::cc_render::{
    CcRenderer, PhysicsConfig, RendererConfig,
};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct CaptureOptions {
    width: u32,
    height: u32,
    frames: usize,
    timeout: Duration,
}

impl CaptureOptions {
    fn from_env(seconds: f32) -> anyhow::Result<Self> {
        fn number(
            name: &str,
            default: u32,
            range: std::ops::RangeInclusive<u32>,
        ) -> anyhow::Result<u32> {
            let value = std::env::var(name).map_or(Ok(default), |s| s.parse::<u32>())?;
            anyhow::ensure!(range.contains(&value), "{name} outside allowed range");
            Ok(value)
        }
        let width = number("CC_CONDO_WIDTH", 1920, 64..=4096)?;
        let height = number("CC_CONDO_HEIGHT", 1080, 64..=4096)?;
        anyhow::ensure!(
            width % 2 == 0 && height % 2 == 0,
            "capture dimensions must be even for yuv420p"
        );
        let frames = number("CC_CONDO_FRAMES", (seconds * 30.).round() as u32, 1..=18000)? as usize;
        let timeout = Duration::from_secs(number(
            "CC_CONDO_TIMEOUT",
            (seconds * 10. + 120.) as u32,
            30..=7200,
        )? as u64);
        Ok(Self {
            width,
            height,
            frames,
            timeout,
        })
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 5,
        "avatar.glb arkit-map.json scene.json output-directory"
    );
    let out = Path::new(&args[4]);
    anyhow::ensure!(
        !out.exists(),
        "capture directory already exists; use a new path to preserve evidence"
    );
    let seconds: f32 = std::env::var("CC_CONDO_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8.0);
    anyhow::ensure!(
        seconds.is_finite() && (1.0..=600.0).contains(&seconds),
        "invalid capture duration"
    );
    let options = CaptureOptions::from_env(seconds)?;
    std::fs::create_dir_all(out)?;
    record_inputs(&args, out, seconds)?;
    let weather = weather_recording::WeatherCapture::new(out, options.frames)?;
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(),
        arkit_map_path: args[2].clone().into(),
        scene_package_path: Some(args[3].clone().into()),
        width: options.width,
        height: options.height,
        fps: 30,
        physics: Some(PhysicsConfig::default()),
        ..Default::default()
    })?;
    let result = record(&renderer, out, &options, weather);
    renderer.shutdown()?;
    result
}

fn capture_asset_paths(args: &[String], folder: &Path, package: &serde_json::Value) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let geometry = package["geometry"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing geometry"))?;
    let mut paths = vec![
        Path::new(&args[1]).to_path_buf(),
        Path::new(&args[2]).to_path_buf(),
        folder.join(geometry),
    ];
    paths.extend(
        [
            "vehicle.glb",
            "water_normal.png",
            "sky.wgsl",
            "sky_common.wgsl",
            "water.wgsl",
            "wet_surfaces.wgsl",
            "city_windows.wgsl",
            "env/pisa_diffuse_rgb9e5_zstd.ktx2",
            "env/pisa_specular_rgb9e5_zstd.ktx2",
        ]
        .map(|p| folder.join(p)),
    );
    if let Some(lightmaps) = package["lightmaps"].as_array() {
        for lightmap in lightmaps {
            let image = lightmap["image"].as_str().ok_or_else(|| anyhow::anyhow!("missing lightmap image"))?;
            paths.push(folder.join(image));
        }
    }
    for part in package["mechanisms"].as_array().into_iter().flatten()
        .flat_map(|m| m["parts"].as_array().into_iter().flatten()) {
        paths.push(folder.join(part["geometry"].as_str()
            .ok_or_else(|| anyhow::anyhow!("missing mechanism geometry"))?));
    }
    for cloth in package["cloths"].as_array().into_iter().flatten() {
        paths.push(folder.join(cloth["surface"].as_str()
            .ok_or_else(|| anyhow::anyhow!("missing cloth surface"))?));
    }
    Ok(paths)
}

fn record_inputs(args: &[String], out: &Path, seconds: f32) -> anyhow::Result<()> {
    if let Some(path) = std::env::var_os("CC_AVATAR_TISSUE_ASSET") {
        std::fs::copy(path, out.join("tissue-asset.json"))?;
    }
    let scene = Path::new(&args[3]);
    let folder = scene.parent().unwrap_or(Path::new("."));
    std::fs::copy(scene, out.join("scene-package.json"))?;
    std::fs::copy(
        folder.join("provenance.json"),
        out.join("scene-provenance.json"),
    )?;
    let mut assets = Vec::new();
    let package: serde_json::Value = serde_json::from_slice(&std::fs::read(scene)?)?;
    let paths = capture_asset_paths(args, folder, &package)?;
    for path in paths {
        let bytes = std::fs::read(&path)?;
        let mut hash = hmac_sha256::Hash::new();
        hash.update(&bytes);
        let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assets.push(serde_json::json!({"path":path,"bytes":bytes.len(),"sha256":digest}));
    }
    let environment: std::collections::BTreeMap<_, _> = std::env::vars()
        .filter(|(key, _)| {
            key.starts_with("CC_CONDO_")
                || key.starts_with("CC_AVATAR_")
                || key == "AVATAR_BEVY_GPU_INDEX"
                || key == "WGPU_BACKEND"
                || key == "VK_ICD_FILENAMES"
        })
        .collect();
    std::fs::write(
        out.join("render-config.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "seconds":seconds,"environment":environment,"assets":assets,"diagnostic_capture":true
        }))?,
    )?;
    if let Ok(path) = std::env::var("CC_CONDO_NATIVE_PACKETS") {
        std::fs::copy(&path, out.join("packets.ndjson"))?;
        if let Some(parent) = Path::new(&path).parent() {
            for filename in ["request.json", "requests.ndjson", "responses.ndjson", "handoff.json", "attempt-plan.json"] {
                let source = parent.join(filename);
                if source.is_file() {
                    std::fs::copy(source, out.join(filename))?;
                }
            }
        }
    }
    Ok(())
}

fn record(renderer: &CcRenderer, out: &Path, options: &CaptureOptions, mut weather: weather_recording::WeatherCapture) -> anyhow::Result<()> {
    let mut frames = Vec::new();
    let warmup = Instant::now();
    while !renderer.is_ready() {
        anyhow::ensure!(
            warmup.elapsed() < options.timeout,
            "scene readiness timeout"
        );
        renderer.drain_frames(&mut frames);
        frames.clear();
        std::thread::sleep(Duration::from_millis(10));
    }
    let packets = std::env::var("CC_CONDO_NATIVE_PACKETS").ok();
    let poses = packets
        .as_ref()
        .map(|path| native_recording::load(Path::new(path)))
        .transpose()?
        .unwrap_or_default();
    let dimensions = format!("{}x{}", options.width, options.height);
    let mut ffmpeg = Command::new("/usr/bin/ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            &dimensions,
            "-framerate",
            "30",
            "-i",
            "-",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-crf",
            "19",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(out.join("view.mp4"))
        .stdin(Stdio::piped())
        .spawn()?;
    let mut input = ffmpeg.stdin.take().unwrap();
    let target = options.frames;
    let started = Instant::now();
    let mut previous = started;
    let mut intervals = Vec::new();
    let mut count = 0;
    let mut replay = native_recording::PlaybackCursor::default();
    let mut states = std::fs::File::create(out.join("states.ndjson"))?;
    let mut frame_times = std::fs::File::create(out.join("frame-times.ndjson"))?;
    while count < target {
        anyhow::ensure!(renderer.is_ready(), "renderer withdrew readiness during capture; inspect renderer log");
        anyhow::ensure!(started.elapsed() < options.timeout, "capture timeout");
        weather.advance(renderer, count)?;
        replay.feed(renderer, &poses, started.elapsed().as_millis() as u64);
        renderer.drain_frames(&mut frames);
        for frame in frames.drain(..) {
            let now = Instant::now();
            intervals.push(now.duration_since(previous).as_secs_f64() * 1000.0);
            previous = now;
            writeln!(frame_times, "{}", serde_json::json!({
                "frame": count, "facial_pts_ms": frame.pts_ms,
                "tissue_pts_ms": frame.tissue_pts_ms,
                "arrival_elapsed_ms": now.duration_since(started).as_secs_f64() * 1000.0,
            }))?;
            input.write_all(&frame.pixels)?;
            weather.observe(renderer, count)?;
            writeln!(
                states,
                "{}",
                serde_json::to_string(&renderer.scene_state())?
            )?;
            if count == 0 || count == target / 2 || count + 1 == target {
                image::save_buffer(
                    out.join(format!("frame-{count:04}.png")),
                    &frame.pixels,
                    frame.width,
                    frame.height,
                    image::ColorType::Rgba8,
                )?;
            }
            count += 1;
            if count >= target {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    drop(input);
    anyhow::ensure!(ffmpeg.wait()?.success(), "ffmpeg failed");
    intervals.sort_by(f64::total_cmp);
    let final_state = renderer.scene_state();
    let report = serde_json::json!({"frames":count,"resolution":[options.width,options.height],"encoded_fps":30,
        "encoded_duration_seconds":count as f64/30.,
        "wall_seconds":started.elapsed().as_secs_f64(),"native_packets":packets,
        "sent_packets":replay.sent,"total_packets":poses.len(),"final_state":final_state,"final_weather":renderer.weather_state(),
        "avatar_evidence":if packets.is_some(){"unchanged recorded native playback"}else{"bind pose only; not motion acceptance"},
        "arrival_interval_p50_ms":intervals[intervals.len()/2],
        "arrival_interval_p95_ms":intervals[intervals.len()*95/100],
        "note":"Arrival timings include capture polling and encoder backpressure, not GPU timings."});
    std::fs::write(
        out.join("capture.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", report);
    Ok(())
}
