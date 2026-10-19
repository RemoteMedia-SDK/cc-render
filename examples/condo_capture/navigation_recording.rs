//! Live normal navigation through CcRenderNode and the registered scene daemon.
//! The capture observes raw frames; all motion requests and handoffs belong to the node.
use cc_render_plugin::cc_render::{
    CcRenderConfig, CcRenderNode, CcRenderer,
    renderer::{ArkitPose, RenderedFrame, Renderer, SceneState, SkeletalPose},
    office::controller::OfficeCommand,
};
use remotemedia_plugin_sdk::{traits::streaming::{AsyncStreamingNode, Tick}, types::RuntimeData};
use serde_json::{json, Value};
use std::{fs::File, io::{BufRead, BufReader, Write}, os::unix::net::UnixStream,
    path::Path, sync::{Arc, mpsc}, time::{Duration, Instant}};

struct CaptureRenderer {
    inner: Arc<CcRenderer>,
    frames: parking_lot::Mutex<Vec<RenderedFrame>>,
}
impl Renderer for CaptureRenderer {
    fn push_pose(&self, pose: ArkitPose) { self.inner.push_pose(pose); }
    fn push_skeletal_pose(&self, pose: Option<SkeletalPose>) { self.inner.push_skeletal_pose(pose); }
    fn drain_frames(&self, _: &mut Vec<RenderedFrame>) {
        self.inner.drain_frames(&mut self.frames.lock());
    }
    fn is_ready(&self) -> bool { self.inner.is_ready() }
    fn skeletal_queue_depth(&self) -> usize { self.inner.skeletal_queue_depth() }
    fn office_command(&self, command: OfficeCommand) { self.inner.office_command(command); }
    fn scene_state(&self) -> SceneState { self.inner.scene_state() }
}

pub struct NavigationCapture {
    node: CcRenderNode,
    renderer: Arc<CaptureRenderer>,
    runtime: tokio::runtime::Runtime,
    requests: mpsc::SyncSender<Value>,
    responses: mpsc::Receiver<Result<(Value, Value), String>>,
    events: File,
    goals: File,
    target: String,
    session: String,
    submitted: bool,
    ticks: u64,
    started: Instant,
}

fn ingest(runtime: &tokio::runtime::Runtime, node: &CcRenderNode, value: Value) -> anyhow::Result<()> {
    runtime.block_on(node.process_streaming(RuntimeData::Json(value), None, |_| Ok(())))?;
    Ok(())
}

fn worker(socket: UnixStream, receiver: mpsc::Receiver<Value>, sender: mpsc::Sender<Result<(Value, Value), String>>,
          mut requests: File, mut responses: File) {
    let mut reader = BufReader::new(socket);
    for request in receiver {
        let result = (|| -> anyhow::Result<Value> {
            writeln!(requests, "{request}")?; requests.flush()?;
            writeln!(reader.get_mut(), "{request}")?;
            let mut line = String::new();
            anyhow::ensure!(reader.read_line(&mut line)? > 0, "navigation daemon disconnected");
            let response: Value = serde_json::from_str(&line)?;
            writeln!(responses, "{response}")?; responses.flush()?;
            Ok(response)
        })().map(|response| (request, response)).map_err(|error| error.to_string());
        let failed = result.is_err();
        if sender.send(result).is_err() || failed { break; }
    }
}

impl NavigationCapture {
    pub fn from_env(renderer: Arc<CcRenderer>, out: &Path, avatar: &str) -> anyhow::Result<Option<Self>> {
        let Ok(target) = std::env::var("CC_CONDO_NAVIGATE") else { return Ok(None) };
        anyhow::ensure!(std::env::var_os("CC_CONDO_NATIVE_PACKETS").is_none(), "navigation and packet replay are exclusive");
        let mut socket = BufReader::new(UnixStream::connect(std::env::var("ARDY_SOCKET")?)?);
        socket.get_ref().set_read_timeout(Some(Duration::from_secs(120)))?;
        let mut line = String::new(); socket.read_line(&mut line)?;
        let ready: Value = serde_json::from_str(&line)?;
        std::fs::write(out.join("navigation-ready.json"), serde_json::to_vec_pretty(&ready)?)?;
        let renderer = Arc::new(CaptureRenderer { inner: renderer, frames: Default::default() });
        let node = CcRenderNode::with_renderer(CcRenderConfig {
            glb_path: avatar.into(), scene_key_file: Some(std::env::var("CC_CONDO_SCENE_KEY")?.into()),
            ..Default::default()
        }, renderer.clone());
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        ingest(&runtime, &node, json!({"kind":"motion_capabilities", "scene_sampler":ready["scene_sampler"]}))?;
        let (requests, request_rx) = mpsc::sync_channel(1);
        let (response_tx, responses) = mpsc::channel();
        let request_file = File::create(out.join("navigation-requests.ndjson"))?;
        let response_file = File::create(out.join("navigation-responses.ndjson"))?;
        std::thread::spawn(move || worker(socket.into_inner(), request_rx, response_tx, request_file, response_file));
        let mut this = Self { node, renderer, runtime, requests, responses,
            events: File::create(out.join("navigation-events.ndjson"))?,
            goals: File::create(out.join("navigation-goals.ndjson"))?, target,
            session: format!("{}-731", out.file_name().unwrap().to_string_lossy()), submitted:false,
            ticks:0, started:Instant::now() };
        this.goal(1, 0, 2, json!({"description":"A person stands relaxed in the room."}))?;
        Ok(Some(this))
    }

    fn goal(&mut self, revision: u64, pts: u64, version: u8, goal: Value) -> anyhow::Result<()> {
        let request = json!({"kind":"motion_goal", "version":version, "session_id":self.session,
            "generation_revision":revision, "pts_ms":pts, "seed":731, "goal":goal});
        writeln!(self.goals, "{request}")?; self.goals.flush()?;
        ingest(&self.runtime, &self.node, request)
    }

    fn receive(&mut self) -> anyhow::Result<()> {
        while let Ok(result) = self.responses.try_recv() {
            let (envelope, response) = result.map_err(anyhow::Error::msg)?;
            if let Some(error) = response.get("error") {
                ingest(&self.runtime, &self.node, json!({"kind":"motion_error","error":error}))?;
                writeln!(self.events, "{}", json!({"kind":"worker_error","error":error}))?;
                continue;
            }
            for frame in response["frames"].as_array().ok_or_else(|| anyhow::anyhow!("missing native frames"))? {
                ingest(&self.runtime, &self.node, frame.clone())?;
            }
            let request = if envelope["kind"] == "scene_step" {
                let payload: Value = serde_json::from_str(envelope["payload"].as_str().unwrap())?;
                payload["request"].clone()
            } else { envelope };
            ingest(&self.runtime, &self.node, json!({"kind":"motion_tick", "session_id":request["session_id"],
                "generation_revision":request["generation_revision"], "pts_ms":request["pts_ms"]}))?;
        }
        Ok(())
    }

    pub fn advance(&mut self, frames: &mut Vec<RenderedFrame>) -> anyhow::Result<()> {
        if self.started.elapsed().as_secs_f64() * 30. < self.ticks as f64 { return Ok(()); }
        self.receive()?;
        if !self.submitted {
            if let Some(office) = self.renderer.scene_state().office.filter(|o| o.avatar.ready
                && o.avatar.native_stream.as_ref().is_some_and(|s| s.session_id == self.session)) {
                self.goal(2, office.avatar.displayed_pts_ms, 3,
                    json!({"operation":"navigate", "target_id":self.target, "pace":"normal"}))?;
                self.submitted = true;
            }
        }
        let outputs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = outputs.clone();
        self.runtime.block_on(self.node.tick(Tick {clock_id:"condo-navigation".into(),
            pts_us:self.started.elapsed().as_micros() as u64, frame_idx:self.ticks, deadline_us:0}, None,
            Box::new(move |value| { if let RuntimeData::Json(value) = value { sink.lock().push(value); } Ok(()) })))?;
        self.ticks += 1;
        for value in outputs.lock().drain(..) {
            if matches!(value["kind"].as_str(), Some("motion_goal" | "scene_step")) {
                self.requests.try_send(value)?;
            } else {
                writeln!(self.events, "{value}")?;
            }
        }
        self.events.flush()?;
        frames.append(&mut self.renderer.frames.lock());
        Ok(())
    }
}
