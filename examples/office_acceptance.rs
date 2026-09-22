//! GPU acceptance driver. ARDY_PYTHON and ARDY_WORKER select the isolated worker.
//! Usage: cargo run --example office_acceptance -- avatar.glb arkit-map.json output.mp4
//! Optional OFFICE_FACE_JSONL supplies prerecorded Audio2Face blendshape envelopes.
use cc_render_plugin::cc_render::office::controller::{ActionRequest, OfficeCommand, Verb};
use cc_render_plugin::cc_render::renderer::{Hand, PhysicsConfig};
use cc_render_plugin::cc_render::{ArkitPose, CcRenderNode, CcRenderer, Renderer, RendererConfig};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
#[path = "support/office_choreography.rs"]
mod office_choreography;
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
    let mut ffmpeg = Command::new("ffmpeg")
        .args([
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
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            &args[3],
        ])
        .stdin(Stdio::piped())
        .spawn()?;
    let mut video = ffmpeg.stdin.take().unwrap();
    let mut report = std::fs::File::create(format!("{}.jsonl", args[3]))?;
    let started = Instant::now();
    let mut actions_started: Option<Instant> = None;
    let mut last_request = Instant::now();
    let idle_hold = std::env::var("OFFICE_ACCEPTANCE_HOLD_SEC")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20)
        .clamp(3, 120);
    let mut stage = 0;
    let mut choreography = (std::env::var("OFFICE_ACCEPTANCE_TRANSITIONS").as_deref() == Ok("1")
        || std::env::var("OFFICE_ACCEPTANCE_WALKTHROUGH").as_deref() == Ok("1"))
    .then(office_choreography::Choreography::new);
    let mut seated_hold: Option<Instant> = None;
    let mut last_event = 0;
    let mut frames = vec![];
    let mut saved = false;
    let mut failed = false;
    let (face_tx, face_rx) = mpsc::sync_channel(64);
    // Optional live JSONL producer: ready line, then waits for a start line on
    // stdin before emitting paced blendshape envelopes. Keep speech independent
    // of the body clock so a motion underrun never pauses articulation.
    let mut face_child = if let Ok(program) = std::env::var("OFFICE_FACE_PROGRAM") {
        let mut child = Command::new(program)
            .args([
                std::env::var("OFFICE_FACE_BUNDLE")?,
                std::env::var("OFFICE_FACE_AUDIO")?,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                    if face_tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        Some(child)
    } else {
        None
    };
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
        for v in face_rx.try_iter() {
            if let Some(weights) = v
                .get("arkit_52")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
            {
                let pts_ms = v["pts_ms"].as_u64().unwrap_or(0);
                renderer.push_pose(ArkitPose { weights, pts_ms });
                writeln!(
                    report,
                    "{}",
                    serde_json::json!({"kind":"speech_frame","pts_ms":pts_ms,"wall_ms":actions_started.map(|t| t.elapsed().as_millis())})
                )?;
            }
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
                    serde_json::json!({"kind":"motion_metrics","metrics":metrics})
                )?;
            }
        }
        if failed {
            break;
        }
        let state = renderer.scene_state();
        if let Some(office) = &state.office {
            if renderer.is_ready() && last_request.elapsed() > Duration::from_millis(100) {
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
                let _ = request_tx.try_send(request);
                last_request = Instant::now();
            }
            for e in office.events.iter().filter(|e| e.sequence > last_event) {
                writeln!(report, "{}", serde_json::to_value(e)?)?;
                eprintln!("{} {:?}: {}", e.action_id, e.status, e.detail);
            }
            if let Some(e) = office.events.last() {
                last_event = e.sequence;
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
            if let Some(script) = choreography.as_mut().filter(|_| office.avatar.ready) {
                if actions_started.is_none() {
                    actions_started = Some(Instant::now());
                    if let Some(input) = face_child.as_mut().and_then(|c| c.stdin.as_mut()) {
                        writeln!(input, "start")?;
                    }
                }
                let (command, done) = script.tick(office);
                if let Some(command) = command {
                    renderer.office_command(command);
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
                    0 if office.avatar.ready => {
                        actions_started = Some(Instant::now());
                        if let Some(input) = face_child.as_mut().and_then(|c| c.stdin.as_mut()) {
                            writeln!(input, "start")?;
                        }
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
            if let Some(v) = speech.iter().rev().find(|v| {
                v["pts_ms"].as_u64().unwrap_or(0)
                    <= actions_started.map_or(0, |t| t.elapsed().as_millis() as u64)
            }) {
                if let Some(weights) = v
                    .get("arkit_52")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                {
                    renderer.push_pose(ArkitPose {
                        weights,
                        pts_ms: office.avatar.displayed_pts_ms,
                    });
                }
            }
        }
        renderer.drain_frames(&mut frames);
        for f in frames.drain(..) {
            if actions_started.is_some() {
                video.write_all(&f.pixels)?;
                if !saved {
                    image::save_buffer(
                        format!("{}.png", args[3]),
                        &f.pixels,
                        640,
                        480,
                        image::ColorType::Rgba8,
                    )?;
                    saved = true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(video);
    let _ = ffmpeg.wait();
    let _ = child.kill();
    let _ = child.wait();
    if let Some(mut child) = face_child {
        let _ = child.kill();
        let _ = child.wait();
    }
    renderer.shutdown()?;
    anyhow::ensure!(
        !failed && stage == 8,
        "acceptance incomplete: stage {stage}; inspect {}.jsonl",
        args[3]
    );
    Ok(())
}
