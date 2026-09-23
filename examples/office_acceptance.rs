//! GPU acceptance driver. ARDY_PYTHON and ARDY_WORKER select the isolated worker.
//! Usage: cargo run --example office_acceptance -- avatar.glb arkit-map.json output.mp4
//! Optional OFFICE_FACE_JSONL supplies prerecorded Audio2Face blendshape envelopes.
use cc_render_plugin::cc_render::office::controller::{ActionRequest, OfficeCommand, Verb};
use cc_render_plugin::cc_render::renderer::{Hand, PhysicsConfig};
use cc_render_plugin::cc_render::{CcRenderNode, CcRenderer, Renderer, RendererConfig};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
#[path = "support/office_choreography.rs"]
mod office_choreography;
#[path = "support/office_speech.rs"]
mod office_speech;
fn action(id: &str, verb: Verb, target: Option<&str>, hand: Option<Hand>) -> OfficeCommand {
    OfficeCommand::PerformAction {
        request: ActionRequest {
            action_id: id.into(),
            verb,
            target: target.map(str::to_string),
            hand,
            position: None,
            duration_sec: 3.,
            intensity: 0.7,
        },
    }
}
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 4,
        "avatar.glb arkit-map.json output.mp4 required"
    );
    std::env::set_var("CC_OFFICE", "1");
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(),
        arkit_map_path: args[2].clone().into(),
        width: 640,
        height: 480,
        fps: 30,
        physics: Some(PhysicsConfig::default()),
        ..Default::default()
    })?;
    let (request_tx, request_rx) = mpsc::sync_channel::<serde_json::Value>(1);
    let (output_tx, output_rx) = mpsc::channel();
    let python = std::env::var("ARDY_PYTHON")?;
    let worker = std::env::var("ARDY_WORKER")?;
    let mut child = Command::new(python)
        .args(["-u", &worker])
        .env(
            "CUDA_VISIBLE_DEVICES",
            std::env::var("ARDY_GPU").unwrap_or_else(|_| "1".into()),
        )
        .env("ARDY_DEVICE", "cuda:0")
        .env("TEXT_ENCODER_DEVICE", "cuda:0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        eprintln!("worker: {}", line.trim());
        while let Ok(request) = request_rx.recv() {
            if writeln!(stdin, "{}", request).is_err() || stdin.flush().is_err() {
                break;
            }
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if let Ok(reply) = serde_json::from_str::<serde_json::Value>(&line) {
                if output_tx.send(reply).is_err() {
                    break;
                }
            }
        }
    });
    let mut encoder = Command::new(std::env::var("FFMPEG").unwrap_or_else(|_| "ffmpeg".into()));
    encoder.args([
        "-y",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        "640x480",
        "-r",
        "30",
        "-i",
        "-",
    ]);
    if let Ok(audio) = std::env::var("OFFICE_FACE_AUDIO") {
        encoder.args([
            "-f",
            "f32le",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-i",
            &audio,
            "-af",
            "apad",
            "-c:a",
            "aac",
            "-shortest",
        ]);
    } else {
        encoder.arg("-an");
    }
    let mut ffmpeg = encoder
        .args([
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            &args[3],
        ])
        .stdin(Stdio::piped())
        .spawn()?;
    let mut video = ffmpeg.stdin.take().unwrap();
    let mut report = std::fs::File::create(format!("{}.jsonl", args[3]))?;
    let started = Instant::now();
    let mut actions_started: Option<Instant> = None;
    let mut last_request = Instant::now();
    let mut last_request_revision = None;
    let mut command_revision = None;
    let mut last_pose_sample_pts = None;
    let idle_hold = std::env::var("OFFICE_ACCEPTANCE_HOLD_SEC")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20)
        .clamp(3, 120);
    let mut stage = 0;
    let mut choreography = (std::env::var("OFFICE_ACCEPTANCE_TRANSITIONS").as_deref() == Ok("1")
        || std::env::var("OFFICE_ACCEPTANCE_WALKTHROUGH").as_deref() == Ok("1")
        || std::env::var("OFFICE_ACCEPTANCE_SURFACE_JUMP").as_deref() == Ok("1"))
    .then(office_choreography::Choreography::new);
    let mut seated_hold: Option<Instant> = None;
    let mut last_event = 0;
    let mut frames = vec![];
    let mut saved = false;
    let mut failed = false;
    let mut live_speech = office_speech::Speech::spawn()?;
    let mut video_frames = 0_u64;
    let mut last_pixels = None;
    let mut last_face_pts = 0;
    let speech: Vec<serde_json::Value> = std::env::var("OFFICE_FACE_JSONL")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            s.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default();
    while started.elapsed() < Duration::from_secs(600)
        && actions_started.map_or(true, |t| t.elapsed() < Duration::from_secs(180))
    {
        live_speech.poll()?;
        if let Some(start) = actions_started {
            live_speech.present(start.elapsed(), &renderer, &mut report)?;
        }
        for reply in output_rx.try_iter() {
            if let Some(error) = reply.get("error") {
                eprintln!("worker failed: {error}");
                failed = true;
                break;
            }
            if let Some(poses) = reply.get("frames").and_then(|v| v.as_array()) {
                for v in poses {
                    if v.get("source_joints").is_some() {
                        writeln!(report, "{}", v)?;
                    }
                    if let Some(p) = CcRenderNode::skeletal_pose_from_json(v) {
                        renderer.push_skeletal_pose(Some(p));
                    }
                }
            }
            if let Some(metrics) = reply.get("metrics") {
                writeln!(
                    report,
                    "{}",
                    serde_json::json!({"kind":"motion_metrics","metrics":metrics,
                        "received_wall_ms":started.elapsed().as_millis(),
                        "generation_revision":reply.get("frames").and_then(|v| v.as_array())
                            .and_then(|frames| frames.first()).and_then(|p| p.get("generation_revision"))})
                )?;
            }
        }
        if failed {
            break;
        }
        let state = renderer.scene_state();
        if let Some(office) = &state.office {
            for e in office.events.iter().filter(|e| e.sequence > last_event) {
                writeln!(report, "{}", serde_json::to_value(e)?)?;
                eprintln!("{} {:?}: {}", e.action_id, e.status, e.detail);
            }
            if let Some(e) = office.events.last() {
                last_event = e.sequence;
            }
            if last_pose_sample_pts != Some(office.avatar.displayed_pts_ms) {
                writeln!(report, "{}", serde_json::json!({
                    "kind":"pose_sample", "avatar":office.avatar,
                    "generation_revision":office.motion.generation_revision,
                    "wall_ms":started.elapsed().as_millis()
                }))?;
                last_pose_sample_pts = Some(office.avatar.displayed_pts_ms);
            }
            let complete = |id: &str| {
                office.events.iter().any(|e| {
                    e.action_id == id
                        && e.status
                            == cc_render_plugin::cc_render::office::controller::Status::Completed
                })
            };
            if office.events.iter().any(|e| {
                e.status == cc_render_plugin::cc_render::office::controller::Status::Failed
            }) {
                failed = true;
                break;
            }
            if let Some(script) = choreography
                .as_mut()
                .filter(|_| office.avatar.ready && live_speech.ready())
            {
                if actions_started.is_none() {
                    actions_started = Some(Instant::now());
                }
                let (command, done) = script.tick(office);
                if let Some(command) = command {
                    renderer.office_command(command);
                    command_revision = Some(office.motion.generation_revision);
                }
                if done {
                    if std::env::var("OFFICE_ACCEPTANCE_WALKTHROUGH").as_deref() == Ok("1") {
                        failed = office.posture != "seated"
                            || !office.occupied_hands.iter().any(|(_, id)| id == "mug");
                        if failed {
                            eprintln!(
                                "walkthrough ended without the seated avatar holding the mug"
                            );
                        }
                    }
                    stage = 8;
                    break;
                }
            } else if choreography.is_none() {
                match stage {
                    0 if office.avatar.ready && live_speech.ready() => {
                        actions_started = Some(Instant::now());
                        if std::env::var("OFFICE_ACCEPTANCE_SIT_ONLY").as_deref() == Ok("1") {
                            renderer.office_command(action("sit", Verb::Sit, Some("chair"), None));
                            stage = 10;
                            continue;
                        }
                        if std::env::var("OFFICE_ACCEPTANCE_GRAB_ONLY").as_deref() == Ok("1") {
                            renderer.office_command(action(
                                "grab",
                                Verb::Grab,
                                Some("mug"),
                                Some(Hand::Right),
                            ));
                            stage = 5;
                            continue;
                        }
                        renderer.office_command(action("walk", Verb::Walk, Some("desk"), None));
                        if std::env::var("OFFICE_ACCEPTANCE_WALK_ONLY").as_deref() == Ok("1") {
                            stage = 9;
                            continue;
                        }
                        renderer.office_command(action(
                            "wave-walk",
                            Verb::Wave,
                            None,
                            Some(Hand::Left),
                        ));
                        stage = 1;
                    }
                    10 if complete("sit") => {
                        if seated_hold.get_or_insert_with(Instant::now).elapsed()
                            >= Duration::from_secs(idle_hold)
                        {
                            stage = 8;
                            break;
                        }
                    }
                    9 if complete("walk") => {
                        stage = 8;
                        break;
                    }
                    1 if complete("walk") => {
                        renderer.office_command(action("sit", Verb::Sit, Some("chair"), None));
                        stage = 2;
                    }
                    2 if complete("sit") => {
                        renderer.office_command(action("smile", Verb::Smile, None, None));
                        stage = 3;
                    }
                    3 if complete("smile") => {
                        renderer.office_command(action("stand", Verb::StandUp, None, None));
                        stage = 4;
                    }
                    4 if complete("stand") => {
                        if std::env::var("OFFICE_ACCEPTANCE_SEAT_ONLY").as_deref() == Ok("1") {
                            stage = 8;
                            break;
                        }
                        renderer.office_command(action(
                            "grab",
                            Verb::Grab,
                            Some("mug"),
                            Some(Hand::Right),
                        ));
                        stage = 5;
                    }
                    5 if complete("grab") => {
                        if std::env::var("OFFICE_ACCEPTANCE_PLACE_ONLY").as_deref() == Ok("1") {
                            renderer.office_command(action(
                                "place", Verb::Release, Some("desk"), Some(Hand::Right),
                            ));
                            stage = 7;
                            continue;
                        }
                        renderer.office_command(action("carry", Verb::Walk, Some("sofa"), None));
                        renderer.office_command(action(
                            "wave-carry",
                            Verb::Wave,
                            None,
                            Some(Hand::Left),
                        ));
                        stage = 6;
                    }
                    6 if complete("carry") => {
                        renderer.office_command(action(
                            "place",
                            Verb::Release,
                            Some("desk"),
                            Some(Hand::Right),
                        ));
                        stage = 7;
                    }
                    7 if complete("place") => {
                        stage = 8;
                        break;
                    }
                    _ => {}
                }
            }
            if renderer.is_ready() && command_revision != Some(office.motion.generation_revision)
                && (last_request_revision != Some(office.motion.generation_revision)
                || last_request.elapsed() > Duration::from_millis(100)) {
                let mut request = office.motion.clone();
                request.pts_ms = office.avatar.displayed_pts_ms;
                request.observed_root = office.avatar.pelvis;
                writeln!(
                    report,
                    "{}",
                    serde_json::json!({"kind":"observation","avatar":office.avatar,"motion":request,
                        "posture":office.posture,"occupied_hands":office.occupied_hands,
                        "props":state.props,"grabs":state.grabs})
                )?;
                let mut request = serde_json::to_value(request)?;
                if std::env::var("OFFICE_DEBUG_MOTION").as_deref() == Ok("1") {
                    request["debug_source_pose"] = true.into();
                }
                if let Ok(seed) = std::env::var("OFFICE_ACCEPTANCE_SEED") {
                    request["seed"] = seed.parse::<u64>()?.into();
                }
                match request_tx.try_send(request) {
                    Ok(()) => {
                        last_request = Instant::now();
                        last_request_revision = Some(office.motion.generation_revision);
                        writeln!(report, "{}", serde_json::json!({
                            "kind":"motion_request_sent", "generation_revision":office.motion.generation_revision,
                            "pts_ms":office.avatar.displayed_pts_ms, "wall_ms":started.elapsed().as_millis()
                        }))?;
                    }
                    Err(mpsc::TrySendError::Full(_)) => {}, // Retry on the next tick; don't lose the revision.
                    Err(mpsc::TrySendError::Disconnected(_)) => anyhow::bail!("motion worker disconnected"),
                }
            }
            if let Some(v) = speech.iter().rev().find(|v| {
                v["pts_ms"].as_u64().unwrap_or(0)
                    <= actions_started.map_or(0, |t| t.elapsed().as_millis() as u64)
            }) {
                if let Some(pose) = CcRenderNode::arkit_pose_from_json(v) {
                    renderer.push_pose(pose);
                }
            }
        }
        renderer.drain_frames(&mut frames);
        if let Some(f) = frames.pop() {
            last_face_pts = f.pts_ms;
            last_pixels = Some(f.pixels);
        }
        frames.clear();
        if let (Some(start), Some(pixels)) = (actions_started, last_pixels.as_ref()) {
            // CFR export follows elapsed playback time, repeating the last frame
            // through a render stall instead of shortening video relative to audio.
            let due = (start.elapsed().as_secs_f64() * 30.0).floor() as u64 + 1;
            while video_frames < due {
                video.write_all(pixels)?;
                writeln!(
                    report,
                    "{}",
                    serde_json::json!({"kind":"video_frame", "frame":video_frames, "video_ms":video_frames * 1000 / 30, "face_pts_ms":last_face_pts})
                )?;
                video_frames += 1;
            }
            if !saved {
                image::save_buffer(
                    format!("{}.png", args[3]),
                    pixels,
                    640,
                    480,
                    image::ColorType::Rgba8,
                )?;
                saved = true;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(video);
    let encoded = ffmpeg.wait()?;
    let _ = child.kill();
    let _ = child.wait();
    drop(live_speech);
    renderer.shutdown()?;
    anyhow::ensure!(encoded.success(), "ffmpeg failed: {encoded}");
    anyhow::ensure!(
        !failed && stage == 8,
        "acceptance incomplete: stage {stage}; inspect {}.jsonl",
        args[3]
    );
    Ok(())
}
