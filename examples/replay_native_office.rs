//! Replay recorded native packets through the unchanged office renderer/validator.
//! This is diagnostic playback, not online generation or an alternative motion source.
use cc_render_plugin::cc_render::{CcRenderNode, CcRenderer, PhysicsConfig, Renderer, RendererConfig};
use cc_render_plugin::cc_render::office::controller::OfficeCommand;
use cc_render_plugin::cc_render::renderer::{GrabCmd, Hand};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayGrab {
    on_displayed_pts_ms: u64,
    hand: Hand,
    target: String,
}
use std::{io::Write, process::{Command, Stdio}, time::{Duration, Instant}};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len()==5, "avatar.glb map.json packets.ndjson output-directory required");
    let out=std::path::Path::new(&args[4]);
    std::fs::create_dir_all(out)?;
    let poses: Vec<_> = std::fs::read_to_string(&args[3])?.lines().map(|line| {
        let value: serde_json::Value=serde_json::from_str(line)?;
        CcRenderNode::skeletal_pose_from_json(&value).ok_or_else(||anyhow::anyhow!("invalid recorded packet"))
    }).collect::<anyhow::Result<_>>()?;
    anyhow::ensure!(!poses.is_empty(), "no recorded frames");
    anyhow::ensure!(poses.windows(2).all(|p|p[1].pts_ms>p[0].pts_ms), "non-monotonic replay");
    let ffmpeg=std::env::var("OFFICE_REPLAY_FFMPEG").unwrap_or_else(|_| "/usr/bin/ffmpeg".into());
    let codec_check=Command::new(&ffmpeg).args(["-v","error","-f","lavfi","-i","color=s=16x16:d=0.05",
        "-c:v","libx264","-f","null","-"]).status()?;
    anyhow::ensure!(codec_check.success(), "explicit replay encoder preflight failed");
    std::env::set_var("CC_OFFICE","1");
    std::env::set_var("CC_AVATAR_FOLLOW","1");
    let renderer=CcRenderer::spawn(RendererConfig {
        glb_path:args[1].clone().into(),arkit_map_path:args[2].clone().into(),
        width:512,height:512,fps:30,physics:Some(PhysicsConfig::default()),..Default::default()
    })?;
    let warmup=Instant::now();
    let mut frames=Vec::new();
    while !renderer.is_ready() && warmup.elapsed()<Duration::from_secs(45) {
        renderer.drain_frames(&mut frames);frames.clear();std::thread::sleep(Duration::from_millis(10));
    }
    if !renderer.is_ready() { renderer.shutdown()?; anyhow::bail!("renderer readiness deadline"); }
    renderer.office_command(OfficeCommand::NativeMotionRevision {generation_revision:1});
    let mut encoder=Command::new(&ffmpeg).args(["-y","-loglevel","error","-f","rawvideo",
        "-pixel_format","rgba","-video_size","512x512","-framerate","30","-i","pipe:0",
        "-an","-c:v","libx264","-preset","veryfast","-crf","20","-pix_fmt","yuv420p"])
        .arg(out.join("office-replay.mp4")).stdin(Stdio::piped()).spawn()?;
    let mut video=encoder.stdin.take().unwrap();
    let mut states=std::fs::File::create(out.join("states.ndjson"))?;
    let start=Instant::now();
    let mut next=0;
    let mut encoded=0;
    let mut last_logged=0;
    let first=poses[0].pts_ms;
    let end=poses.last().unwrap().pts_ms;
    let mut failure=None;
    // Optional recorded semantic request. Physics decides IF/WHEN capture occurs;
    // this does not select an attachment frame or supply any body/object pose.
    let grab: Option<ReplayGrab> = std::env::var("OFFICE_REPLAY_GRAB_REQUEST").ok()
        .map(|path| -> anyhow::Result<_> {
            Ok(serde_json::from_slice(&std::fs::read(path)?)?)
        }).transpose()?;
    let mut grab_requested=false;
    while start.elapsed().as_millis() as u64 <= end-first+1500 {
        let clock=first+start.elapsed().as_millis() as u64;
        while next<poses.len() && poses[next].pts_ms<=clock+500 {
            renderer.push_skeletal_pose(Some(poses[next].clone())); next+=1;
        }
        renderer.drain_frames(&mut frames);
        for frame in frames.drain(..) {
            video.write_all(&frame.pixels)?; encoded+=1;
        }
        let state=renderer.scene_state();
        if let Some(request) = grab.as_ref().filter(|_| !grab_requested) {
            if state.office.as_ref().is_some_and(|office|
                office.avatar.displayed_pts_ms >= request.on_displayed_pts_ms) {
                renderer.grab(GrabCmd { hand: request.hand, target: request.target.clone(), palm_offset_override: None });
                grab_requested=true;
            }
        }
        if clock.saturating_sub(last_logged)>=100 {
            writeln!(states,"{}",serde_json::to_string(&state)?)?; last_logged=clock;
        }
        if let Some(error)=state.office.as_ref().and_then(|o|o.motion_error.as_ref()) {
            failure=Some(error.clone()); break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(video);
    let encoded_ok=encoder.wait()?.success();
    let final_state=renderer.scene_state();
    std::fs::write(out.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "diagnostic_replay":true,"online_office_acceptance":false,"sent_packets":next,
        "total_packets":poses.len(),"encoded_frames":encoded,"encoder_ok":encoded_ok,
        "motion_failure":failure,"grab_requested":grab_requested,"state":final_state
    }))?)?;
    renderer.shutdown()?;
    anyhow::ensure!(encoded_ok,"video encoding failed");
    anyhow::ensure!(failure.is_none(),"motion validator stopped replay: {failure:?}");
    Ok(())
}
