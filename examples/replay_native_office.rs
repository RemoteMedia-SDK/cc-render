//! Replay recorded native packets through the unchanged office renderer/validator.
//! This is diagnostic playback, not online generation or an alternative motion source.
use cc_render_plugin::cc_render::{CcRenderNode, CcRenderer, PhysicsConfig, Renderer, RendererConfig};
use cc_render_plugin::cc_render::office::controller::OfficeCommand;
use cc_render_plugin::cc_render::renderer::{GrabCmd, Hand, SceneState};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayGrab {
    on_displayed_pts_ms: u64,
    hand: Hand,
    target: String,
}
use std::{io::Write, process::{Command, Stdio}, time::{Duration, Instant}};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaySeat {
    on_displayed_pts_ms: u64,
    session_id: String,
    generation_revision: u64,
    target: String,
    geometry_path: std::path::PathBuf,
    geometry_sha256: String,
    avatar_sha256: String,
    profile_identity: String,
}

impl ReplaySeat {
    fn check_timing(&self, poses: &[cc_render_plugin::cc_render::renderer::SkeletalPose]) -> anyhow::Result<()> {
        anyhow::ensure!(!self.target.is_empty() && poses.iter().any(|p| p.pts_ms == self.on_displayed_pts_ms)
            && poses.last().is_some_and(|p| p.pts_ms > self.on_displayed_pts_ms), "invalid recorded seat handoff");
        anyhow::ensure!(poses.iter().all(|p| p.stream.as_ref().is_some_and(|s|
            s.session_id == self.session_id && s.generation_revision == self.generation_revision)),
            "seat scope must match the recorded native stream");
        Ok(())
    }
}

fn require_displayed_end(state: &SceneState, end: u64) -> anyhow::Result<()> {
    anyhow::ensure!(state.office.as_ref().is_some_and(|office|
        office.avatar.native_stream.is_some() && office.avatar.displayed_pts_ms == end),
        "replay did not display the final recorded native frame ({end} ms)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_render_plugin::cc_render::office::controller::OfficeController;
    use cc_render_plugin::cc_render::office::timeline::StreamIdentity;

    #[test]
    fn recorded_seat_request_rejects_body_fields() {
        let value = serde_json::json!({"on_displayed_pts_ms":100, "session_id":"recorded",
            "generation_revision":1,"target":"chair","geometry_path":"unused",
            "geometry_sha256":"pin","avatar_sha256":"avatar","profile_identity":"profile",
            "root_pos":[0.,0.,0.]});
        assert!(serde_json::from_value::<ReplaySeat>(value).is_err());
    }

    #[test]
    fn recorded_seat_handoff_requires_remaining_native_motion() {
        let request: ReplaySeat = serde_json::from_value(serde_json::json!({
            "on_displayed_pts_ms":100,"session_id":"recorded","generation_revision":1,
            "target":"chair","geometry_path":"unused","geometry_sha256":"pin",
            "avatar_sha256":"avatar","profile_identity":"profile"})).unwrap();
        let mut pose = cc_render_plugin::cc_render::renderer::SkeletalPose::default();
        pose.pts_ms = 100;
        assert!(request.check_timing(&[pose]).is_err());
    }

    #[test]
    fn zero_displayed_time_is_not_a_successful_offset_replay() {
        let state = SceneState { office: Some(OfficeController::default().snapshot()), ..Default::default() };
        assert!(require_displayed_end(&state, 19600).is_err());
    }

    #[test]
    fn completion_requires_actual_stream_and_last_recorded_timestamp() {
        let mut state = SceneState { office: Some(OfficeController::default().snapshot()), ..Default::default() };
        let avatar = &mut state.office.as_mut().unwrap().avatar;
        avatar.displayed_pts_ms = 19600;
        assert!(require_displayed_end(&state, 19600).is_err());
        state.office.as_mut().unwrap().avatar.native_stream = Some(StreamIdentity {
            session_id: "recorded".into(), generation_revision: 1,
            skeleton_id: "smpl22".into(), root_mode: "world".into(),
        });
        assert!(require_displayed_end(&state, 19600).is_ok());
        assert!(require_displayed_end(&state, 19650).is_err());
    }
}

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
    // Follow by default; an explicit CC_AVATAR_FOLLOW=0 keeps a fixed world camera.
    if std::env::var_os("CC_AVATAR_FOLLOW").is_none() { std::env::set_var("CC_AVATAR_FOLLOW","1"); }
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
    let seat: Option<ReplaySeat> = std::env::var("OFFICE_REPLAY_SEAT_REQUEST").ok()
        .map(|path| -> anyhow::Result<_> { Ok(serde_json::from_slice(&std::fs::read(path)?)?) }).transpose()?;
    if let Some(request) = &seat {
        request.check_timing(&poses)?;
        let mut hash = hmac_sha256::Hash::new();
        hash.update(&std::fs::read(&args[1])?);
        let avatar_hash: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        anyhow::ensure!(avatar_hash == request.avatar_sha256, "replay avatar binding mismatch");
        let geometry = cc_render_plugin::cc_render::office::AvatarGeometry::load_bound(
            &request.geometry_path, &request.geometry_sha256, &avatar_hash, &request.profile_identity)
            .map_err(anyhow::Error::msg)?;
        renderer.office_command(OfficeCommand::InstallNativeSeatGeometry { geometry: std::sync::Arc::new(geometry) });
    }
    renderer.office_command(OfficeCommand::NativeMotionRevision {generation_revision:1});
    let mut encoder=Command::new(&ffmpeg).args(["-y","-loglevel","error","-f","rawvideo",
        "-pixel_format","rgba","-video_size","512x512","-framerate","30","-i","pipe:0",
        "-an","-c:v","libx264","-preset","veryfast","-crf","20","-pix_fmt","yuv420p"])
        .arg(out.join("office-replay.mp4")).stdin(Stdio::piped()).spawn()?;
    let mut video=encoder.stdin.take().unwrap();
    let mut states=std::fs::File::create(out.join("states.ndjson"))?;
    let start=Instant::now();
    // Revision invalidation starts the timeline clock at zero. Bootstrap its
    // epoch immediately, then feed against absolute recorded PTS. Offset clips
    // must not be retimed or reported complete before they have actually played.
    renderer.push_skeletal_pose(Some(poses[0].clone()));
    let mut next=1;
    let mut encoded=0;
    let mut last_logged=0;
    let mut last_logged_pose=0;
    let end=poses.last().unwrap().pts_ms;
    let mut failure=None;
    // Optional recorded semantic request. Physics decides IF/WHEN capture occurs;
    // this does not select an attachment frame or supply any body/object pose.
    let grab: Option<ReplayGrab> = std::env::var("OFFICE_REPLAY_GRAB_REQUEST").ok()
        .map(|path| -> anyhow::Result<_> {
            Ok(serde_json::from_slice(&std::fs::read(path)?)?)
        }).transpose()?;
    let mut grab_requested=false;
    let mut seat_requested=false;
    while start.elapsed().as_millis() as u64 <= end.saturating_add(1500) {
        let clock=start.elapsed().as_millis() as u64;
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
        if let Some(request) = seat.as_ref().filter(|_| !seat_requested) {
            if state.office.as_ref().is_some_and(|office|
                office.avatar.displayed_pts_ms >= request.on_displayed_pts_ms) {
                renderer.office_command(OfficeCommand::AuthorizeNativeSeat {
                    session_id: request.session_id.clone(), generation_revision: request.generation_revision,
                    target_id: request.target.clone(),
                });
                seat_requested=true;
            }
        }
        let displayed = state.office.as_ref().map_or(0, |o| o.avatar.displayed_pts_ms);
        if clock.saturating_sub(last_logged)>=100 || displayed != last_logged_pose {
            writeln!(states,"{}",serde_json::to_string(&state)?)?;
            last_logged=clock;
            last_logged_pose=displayed;
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
        "motion_failure":failure,"grab_requested":grab_requested,"seat_requested":seat_requested,
        "strict_seat_collisions":std::env::var("CC_RENDER_STRICT_SEAT_COLLISIONS").as_deref()==Ok("1"),
        "state":final_state
    }))?)?;
    renderer.shutdown()?;
    anyhow::ensure!(encoded_ok,"video encoding failed");
    anyhow::ensure!(failure.is_none(),"motion validator stopped replay: {failure:?}");
    require_displayed_end(&final_state, end)?;
    Ok(())
}
