//! Bounded speech playback on the recording clock, independent of body buffering.
use cc_render_plugin::cc_render::{ArkitPose, CcRenderNode, CcRenderer};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

pub(super) struct Speech {
    child: Option<Child>,
    receiver: mpsc::Receiver<serde_json::Value>,
    pending: VecDeque<ArkitPose>,
    finished: bool,
}
impl Speech {
    pub(super) fn spawn() -> anyhow::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(64);
        let mut child = None;
        if let Ok(program) = std::env::var("OFFICE_FACE_PROGRAM") {
            let mut process = Command::new(program)
                .args([
                    std::env::var("OFFICE_FACE_BUNDLE")?,
                    std::env::var("OFFICE_FACE_AUDIO")?,
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?;
            let stdout = process.stdout.take().unwrap();
            // Start inference now; actions wait for a small prebuffer, excluding
            // model warmup from the exported recording.
            writeln!(process.stdin.take().unwrap(), "start")?;
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let result = line
                        .map_err(anyhow::Error::from)
                        .and_then(|s| Ok(serde_json::from_str(&s)?));
                    let value =
                        result.unwrap_or_else(|e| serde_json::json!({"error":e.to_string()}));
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            });
            child = Some(process);
        }
        Ok(Self {
            child,
            receiver,
            pending: VecDeque::new(),
            finished: false,
        })
    }

    pub(super) fn poll(&mut self) -> anyhow::Result<()> {
        while self.pending.len() < 120 {
            let Ok(value) = self.receiver.try_recv() else {
                break;
            };
            anyhow::ensure!(value.get("error").is_none(), "Audio2Face: {value}");
            if value["event"] == "finished" {
                self.finished = true;
            }
            if value.get("arkit_52").is_some() {
                self.pending.push_back(
                    CcRenderNode::arkit_pose_from_json(&value)
                        .ok_or_else(|| anyhow::anyhow!("invalid Audio2Face envelope"))?,
                );
            }
        }
        if let Some(child) = self.child.as_mut() {
            if let Some(status) = child.try_wait()? {
                anyhow::ensure!(status.success(), "Audio2Face exited: {status}");
            }
        }
        Ok(())
    }

    pub(super) fn ready(&self) -> bool {
        self.child.is_none()
            || self.finished
            || self.pending.back().is_some_and(|p| p.pts_ms >= 500)
    }

    pub(super) fn present(
        &mut self,
        elapsed: Duration,
        renderer: &CcRenderer,
        report: &mut impl Write,
    ) -> anyhow::Result<()> {
        let wall_ms = elapsed.as_millis() as u64;
        if let Some(pose) = take_due(&mut self.pending, wall_ms) {
            writeln!(
                report,
                "{}",
                serde_json::json!({"kind":"speech_frame", "pts_ms":pose.pts_ms, "wall_ms":wall_ms, "weights":pose.weights})
            )?;
            renderer.push_pose(pose);
        }
        Ok(())
    }
}

fn take_due(pending: &mut VecDeque<ArkitPose>, wall_ms: u64) -> Option<ArkitPose> {
    let mut latest = None;
    while pending.front().is_some_and(|p| p.pts_ms <= wall_ms) {
        latest = pending.pop_front();
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio2face_array_maps_to_named_renderer_channels() {
        let mut weights = vec![0.0; 52];
        weights[17] = 0.65;
        weights[24] = 0.4;
        let mut value = serde_json::json!({"kind":"blendshapes", "arkit_52":weights, "pts_ms":123});
        let pose = CcRenderNode::arkit_pose_from_json(&value).unwrap();
        assert_eq!(pose.pts_ms, 123);
        assert_eq!(pose.weights["jawOpen"], 0.65);
        assert_eq!(pose.weights["mouthSmileRight"], 0.4);
        value["arkit_52"] = serde_json::json!([0.0]);
        assert!(CcRenderNode::arkit_pose_from_json(&value).is_none());
    }
    #[test]
    fn playback_keeps_future_frames_and_catches_up_after_a_stall() {
        let mut pending: VecDeque<_> = [0, 16, 33, 50, 66, 83]
            .into_iter()
            .map(|pts_ms| ArkitPose {
                weights: Default::default(),
                pts_ms,
            })
            .collect();
        assert_eq!(take_due(&mut pending, 0).unwrap().pts_ms, 0);
        assert!(take_due(&mut pending, 10).is_none());
        assert_eq!(pending.front().unwrap().pts_ms, 16);
        assert_eq!(take_due(&mut pending, 70).unwrap().pts_ms, 66);
        assert_eq!(pending.front().unwrap().pts_ms, 83);
        assert_eq!(take_due(&mut pending, 1000).unwrap().pts_ms, 83);
        assert!(take_due(&mut pending, 2000).is_none());
    }
}
impl Drop for Speech {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
