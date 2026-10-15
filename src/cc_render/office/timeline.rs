//! Bounded, revision-aware world-space stream. Legacy poses bypass this timeline.
use super::super::renderer::SkeletalPose;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamIdentity {
    pub session_id: String,
    pub generation_revision: u64,
    pub skeleton_id: String,
    pub root_mode: String,
}
impl StreamIdentity {
    /// Outer None rejects a malformed envelope; inner None is legacy anchoring.
    pub fn from_envelope(v: &serde_json::Value) -> Option<Option<Self>> {
        if v.get("session_id").is_none() {
            return Some(None);
        }
        let identity = Self {
            session_id: v.get("session_id")?.as_str()?.into(),
            generation_revision: v.get("generation_revision")?.as_u64()?,
            skeleton_id: v.get("skeleton_id")?.as_str()?.into(),
            root_mode: v.get("root_mode")?.as_str()?.into(),
        };
        identity.valid().then_some(Some(identity))
    }
    pub fn valid(&self) -> bool {
        !self.session_id.is_empty() && self.skeleton_id == "smpl22" && self.root_mode == "world"
    }
}
#[derive(Resource, Default)]
pub struct PoseTimeline {
    session: Option<String>,
    revision: u64,
    queue: VecDeque<SkeletalPose>,
    displayed: Option<SkeletalPose>,
    epoch: Option<(f64, u64)>,
    watermark: u64,
    starved: bool,
    rejected_revision: Option<u64>,
    rejection: Option<serde_json::Value>,
    pub stale_frames: u64,
    pub underruns: u64,
}
impl PoseTimeline {
    pub fn invalidate(&mut self, revision: u64) {
        self.schedule_revision(revision, None);
    }
    pub fn schedule_revision(&mut self, revision: u64, handoff: Option<u64>) {
        if revision > self.revision {
            self.rejection = None;
            self.queue
                .retain(|p| handoff.is_some_and(|at| p.pts_ms <= at));
            self.starved = self.queue.is_empty();
            self.revision = revision;
        }
    }
    /// Pick a buffered token boundary near a 500 ms lead. Its full pose remains
    /// committed in this queue and in the worker history until the handoff.
    pub fn future_root(&self) -> Option<(u64, [f32; 3])> {
        let last = self.queue.back()?.pts_ms;
        self.queue
            .iter()
            .filter(|p| p.pts_ms >= self.watermark + 150 && (last - p.pts_ms) % 200 == 0)
            .min_by_key(|p| p.pts_ms.abs_diff(self.watermark + 500))
            .map(|p| (p.pts_ms, p.root_pos))
    }
    pub fn insert(&mut self, p: SkeletalPose, now: f64) {
        let Some(id) = &p.stream else { return };
        if !id.valid() {
            return;
        }
        if self.session.is_none() {
            self.session = Some(id.session_id.clone());
            self.epoch = Some((now, p.pts_ms));
        }
        if self.session.as_ref() != Some(&id.session_id)
            || id.generation_revision < self.revision
            || self
                .rejected_revision
                .is_some_and(|revision| id.generation_revision <= revision)
            || self.displayed.is_some() && p.pts_ms <= self.watermark
        {
            self.stale_frames += 1;
            return;
        }
        if id.generation_revision > self.revision {
            self.queue.clear();
            self.revision = id.generation_revision;
        }
        // A worker may resolve to the preceding token boundary. Replace only
        // its superseded suffix, leaving the committed prefix playing.
        self.queue.retain(|old| {
            old.pts_ms < p.pts_ms
                || old
                    .stream
                    .as_ref()
                    .is_some_and(|s| s.generation_revision == id.generation_revision)
        });
        if self
            .queue
            .back()
            .is_some_and(|last| p.pts_ms <= last.pts_ms)
        {
            self.stale_frames += 1;
            return;
        }
        if self.starved && self.queue.is_empty() {
            self.epoch = Some((now, self.watermark));
            self.starved = false;
        }
        if self.queue.len() < 136 {
            self.queue.push_back(p);
        } // Core40 + maximum 4.4s scene prefetch and token margin; bounded to 6.8s
    }
    pub fn sample(&mut self, now: f64) -> Option<SkeletalPose> {
        let (epoch, base) = self.epoch?;
        let pts = base + ((now - epoch).max(0.) * 1000.) as u64;
        while self.queue.front().is_some_and(|p| p.pts_ms <= pts) {
            self.displayed = self.queue.pop_front();
        }
        let a = self.displayed.as_ref()?;
        let Some(b) = self.queue.front() else {
            self.underruns += 1;
            self.starved = true;
            self.watermark = a.pts_ms;
            return Some(a.clone());
        };
        let alpha = (pts.saturating_sub(a.pts_ms) as f32 / (b.pts_ms - a.pts_ms).max(1) as f32)
            .clamp(0., 1.);
        let mut out = a.clone();
        out.pts_ms = pts;
        out.root_pos = Vec3::from(a.root_pos)
            .lerp(Vec3::from(b.root_pos), alpha)
            .to_array();
        for i in 0..22 {
            out.joint_quats[i] = Quat::from_array(a.joint_quats[i])
                .slerp(Quat::from_array(b.joint_quats[i]), alpha)
                .to_array();
        }
        self.watermark = pts;
        self.displayed = Some(out.clone());
        Some(out)
    }
    /// Validate the complete frame before retargeting. A blocked root must not
    /// leave the new joint rotations playing on top of the previous root.
    fn sample_checked(
        &mut self,
        now: f64,
        mut permits: impl FnMut(&SkeletalPose) -> bool,
    ) -> Option<SkeletalPose> {
        let previous = self.displayed.clone();
        let pose = self.sample(now)?;
        if permits(&pose) {
            return Some(pose);
        }
        let diagnostic_pose = |p: &SkeletalPose| serde_json::json!({
            "pts_ms":p.pts_ms,"root_pos":p.root_pos,"joint_quats_xyzw":p.joint_quats,
            "stream":p.stream,
        });
        self.rejection = Some(serde_json::json!({
            "session_id":pose.stream.as_ref().map(|s| s.session_id.as_str()),
            "generation_revision":pose.stream.as_ref().map(|s| s.generation_revision),
            "scheduled_generation_revision":self.revision,
            "rejected_pts_ms":pose.pts_ms,"displayed_pts_ms":previous.as_ref().map_or(0, |p| p.pts_ms),
            "reason":"office root-path validation rejected the unchanged candidate",
            "validator":"OfficeController::resolve_native_pose",
            "candidate":diagnostic_pose(&pose),
            "displayed_before":previous.as_ref().map(diagnostic_pose),
        }));
        self.displayed = previous;
        self.watermark = self.displayed.as_ref().map_or(0, |p| p.pts_ms);
        self.queue.clear();
        self.starved = true;
        self.epoch = Some((now, self.watermark));
        self.rejected_revision = Some(self.revision);
        None
    }
}
#[cfg(test)]
#[path = "timeline_live11_tests.rs"]
mod live11_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_root_rejection_is_visible_without_advancing_committed_state() {
        use bevy::ecs::system::SystemState;
        let mut office = super::super::controller::OfficeController::default();
        office.native_motion_revision = Some(1);
        office.observed.ready = true;
        office.observed.pelvis = [1.2504, 0.79, -1.81];
        let mut safe = pose(0, 1, 0.);
        safe.root_pos = office.observed.pelvis;
        let mut blocked = pose(100, 1, 0.);
        let chair = office.objects.iter().find(|o| o.id == "chair").unwrap().position;
        blocked.root_pos = [chair[0], 0.83, chair[2]];
        assert!(!office.permits_native_root(Vec3::from(blocked.root_pos)));
        let mut timeline = PoseTimeline::default();
        timeline.insert(safe.clone(), 0.);
        timeline.insert(blocked, 0.);
        timeline.sample(0.);
        let mut world = World::new();
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_millis(75));
        world.insert_resource(time);
        world.insert_resource(timeline);
        world.insert_resource(office);
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut system = SystemState::<WorldMotion>::new(&mut world);
        assert!(system.get_mut(&mut world).receive(&rx).0.is_none());
        let office = world.resource::<super::super::controller::OfficeController>();
        assert!(office.motion_error.is_some(), "root rejection must not silently poll forever");
        let timeline = world.resource::<PoseTimeline>();
        assert_eq!(timeline.watermark, 0);
        assert_eq!(timeline.displayed.as_ref().unwrap().root_pos, safe.root_pos);
        assert_eq!(timeline.displayed.as_ref().unwrap().joint_quats, safe.joint_quats);
        let diagnostic = timeline.rejection.clone().unwrap();
        assert_eq!(diagnostic["session_id"], "a");
        assert_eq!(diagnostic["generation_revision"], 1);
        assert_eq!(diagnostic["rejected_pts_ms"], 75);
        assert_eq!(diagnostic["displayed_pts_ms"], 0);
        assert_eq!(diagnostic["displayed_before"]["root_pos"], serde_json::json!(safe.root_pos));
        tx.send(Some(pose(200, 1, 99.))).unwrap();
        for _ in 0..100 {
            let mut motion = system.get_mut(&mut world);
            assert!(motion.native_rejected());
            assert!(motion.receive(&rx).0.is_none());
        }
        assert!(rx.is_empty());
        let timeline = world.resource::<PoseTimeline>();
        assert_eq!(timeline.watermark, 0);
        assert_eq!(timeline.rejection.as_ref(), Some(&diagnostic));
        assert_eq!(timeline.displayed.as_ref().unwrap().root_pos, safe.root_pos);
    }

    #[test]
    fn starvation_and_stale_packets_are_not_root_rejections() {
        let mut t = PoseTimeline::default();
        t.insert(pose(0, 2, 0.), 0.);
        t.insert(pose(50, 1, 99.), 0.);
        assert_eq!(t.stale_frames, 1);
        t.sample_checked(10., |_| true);
        assert!(t.starved);
        assert!(t.rejection.is_none());
    }

    #[test]
    fn complete_core40_horizon_survives_prefetch() {
        let mut t = PoseTimeline::default();
        for i in 1..=40 {
            t.insert(pose(i * 50, 1, 0.), 0.);
        }
        assert_eq!(t.queue.len(), 40);
        t.sample(1.35); // displayed 1400 ms; 600 ms remain
        for i in 41..=80 {
            t.insert(pose(i * 50, 1, 0.), 1.35);
        }
        assert_eq!(t.queue.back().unwrap().pts_ms, 4000);
        assert_eq!(t.queue.len(), 52);
    }

    #[test]
    fn reserved_scene_handoff_keeps_playing_original_prefix() {
        let mut timeline = PoseTimeline::default();
        for i in 1..=40 { timeline.insert(pose(i*50, 1, i as f32*0.01), 0.); }
        timeline.schedule_revision(2, Some(2000));
        // Simulate slower generation: the new complete block arrives 1.5s
        // later, but none of its timestamps is late or rewritten.
        for i in 41..=80 { timeline.insert(pose(i*50, 2, i as f32*0.01), 1.5); }
        assert_eq!(timeline.queue.len(), 80);
        let old = timeline.sample(1.5).unwrap();
        assert_eq!(old.stream.unwrap().generation_revision, 1);
        assert_eq!(old.pts_ms, 1550);
        assert_eq!(timeline.sample(1.95).unwrap().pts_ms, 2000);
        let next = timeline.sample(2.).unwrap();
        assert_eq!(next.stream.unwrap().generation_revision, 2);
        assert_eq!(next.pts_ms, 2050);
    }

    #[test]
    fn complete_two_block_scene_window_is_not_truncated() {
        let mut timeline = PoseTimeline::default();
        for i in 1..=80 { timeline.insert(pose(i*50, 1, 0.), 0.); }
        assert_eq!(timeline.queue.len(), 80);
        assert_eq!(timeline.queue.back().unwrap().pts_ms, 4000);
        assert_eq!(timeline.sample(3.95).unwrap().pts_ms, 4000);
    }

    fn pose(t: u64, rev: u64, x: f32) -> SkeletalPose {
        let mut p = SkeletalPose::rest();
        p.pts_ms = t;
        p.root_pos = [x, 1., 0.];
        p.stream = Some(StreamIdentity {
            session_id: "a".into(),
            generation_revision: rev,
            skeleton_id: "smpl22".into(),
            root_mode: "world".into(),
        });
        p
    }
    #[test]
    fn scheduled_handoff_plays_committed_prefix_without_rewinding() {
        let mut t = PoseTimeline::default();
        for i in 0..=20 {
            t.insert(pose(i * 50, 1, i as f32 * 0.05), 0.);
        }
        t.sample(0.1);
        let (handoff, _) = t.future_root().unwrap();
        assert_eq!(handoff, 600);
        t.schedule_revision(2, Some(handoff));
        assert_eq!(t.queue.back().unwrap().pts_ms, 600);
        t.insert(pose(650, 1, 99.), 0.1);
        t.insert(pose(650, 2, 0.65), 0.1);
        t.insert(pose(700, 2, 0.7), 0.1);
        assert!((t.sample(0.5).unwrap().root_pos[0] - 0.5).abs() < 1e-5);
        assert_eq!(t.sample(0.6).unwrap().pts_ms, 600);
        assert!((t.sample(0.65).unwrap().root_pos[0] - 0.65).abs() < 1e-5);
        assert_eq!(t.stale_frames, 1);
    }
    #[test]
    fn collision_rejects_whole_pose_and_resumes_from_last_safe_timestamp() {
        let mut t = PoseTimeline::default();
        t.insert(pose(0, 1, 0.), 0.);
        let mut unsafe_pose = pose(100, 1, 1.);
        unsafe_pose.joint_quats[0] = Quat::from_rotation_x(2.).to_array();
        t.insert(unsafe_pose, 0.);
        let safe = t.sample_checked(0.025, |_| true).unwrap();
        assert!(t.sample_checked(0.075, |p| p.root_pos[0] < 0.5).is_none());
        assert_eq!(t.watermark, safe.pts_ms);
        assert_eq!(t.displayed.as_ref().unwrap().joint_quats, safe.joint_quats);
        assert!(t.future_root().is_none());
        t.insert(pose(150, 1, 2.), 1.);
        assert!(t.queue.is_empty());
        t.schedule_revision(2, None);
        t.insert(pose(50, 2, 0.3), 1.);
        assert_eq!(
            t.sample_checked(1., |_| true).unwrap().root_pos,
            safe.root_pos
        );
        assert_eq!(
            t.sample_checked(1.026, |_| true).unwrap().root_pos,
            [0.3, 1., 0.]
        );
    }
    #[test]
    fn mid_frame_edit_freezes_displayed_pose_until_new_revision_arrives() {
        let mut t = PoseTimeline::default();
        t.insert(pose(0, 1, 0.), 0.);
        t.insert(pose(50, 1, 1.), 0.);
        let shown = t.sample(0.025).unwrap();
        t.invalidate(2);
        t.insert(pose(100, 1, 99.), 0.03);
        let frozen = t.sample(2.).unwrap();
        assert_eq!(frozen.root_pos, shown.root_pos);
        assert_eq!(frozen.pts_ms, shown.pts_ms);
        t.insert(pose(75, 2, 1.5), 2.);
        assert_eq!(t.sample(2.).unwrap().root_pos, shown.root_pos);
        assert_eq!(t.stale_frames, 1);
        assert!(t.sample(2.025).unwrap().root_pos[0] > shown.root_pos[0]);
    }
    #[test]
    fn interpolates_and_rejects_stale() {
        let mut t = PoseTimeline::default();
        t.insert(pose(0, 1, 0.), 0.);
        t.insert(pose(50, 1, 1.), 0.);
        assert!((t.sample(0.025).unwrap().root_pos[0] - 0.5).abs() < 0.001);
        t.insert(pose(100, 2, 2.), 0.025);
        t.insert(pose(150, 1, 10.), 0.025);
        assert_eq!(t.stale_frames, 1);
        assert_eq!(t.queue.len(), 1);
        assert_eq!(t.sample(0.1).unwrap().root_pos[0], 2.);
    }
}

/// ECS integration kept outside the retargeter's legacy playback path.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct WorldMotion<'w, 's> {
    time: Res<'w, Time>,
    timeline: ResMut<'w, PoseTimeline>,
    office: Option<ResMut<'w, super::controller::OfficeController>>,
    parents: Query<'w, 's, &'static Parent>,
    globals: Query<'w, 's, &'static GlobalTransform>,
}
impl WorldMotion<'_, '_> {
    pub fn native_rejected(&self) -> bool {
        self.office.as_ref().is_some_and(|o| o.motion_rejection.is_some())
    }

    pub fn failed(&self) -> bool {
        self.office.as_ref().is_some_and(|o| o.motion_error.is_some())
    }

    pub fn receive(
        &mut self,
        rx: &crossbeam_channel::Receiver<Option<SkeletalPose>>,
    ) -> (Option<SkeletalPose>, bool) {
        if self.native_rejected() {
            while rx.try_recv().is_ok() {} // Keep the committed state and diagnostic, not incoming poses.
            return (None, false);
        }
        if self.failed() {
            *self.timeline = PoseTimeline::default();
            while rx.try_recv().is_ok() {} // discard already queued unsafe poses
            return (None, true);
        }
        if let Some(o) = self.office.as_ref() {
            if let Some((revision, handoff)) = o.native_scene_handoff {
                self.timeline.schedule_revision(revision, Some(handoff));
            } else if let Some(revision) = o.native_motion_revision {
                self.timeline.schedule_revision(revision, None);
            } else {
                self.timeline.schedule_revision(o.revision, o.motion.handoff_pts_ms);
            }
        }
        let now = self.time.elapsed_secs_f64();
        match rx.try_recv() {
            Ok(Some(p)) if p.stream.is_none() => return (Some(p), false),
            Ok(Some(p)) => {
                self.timeline.insert(p, now);
                for p in rx.try_iter().flatten() {
                    self.timeline.insert(p, now);
                }
            }
            Ok(None) if self.office.is_none() => {
                *self.timeline = PoseTimeline::default();
                return (None, true);
            }
            _ => {}
        }
        let pose = self.timeline.sample_checked(now, |pose| {
            let Some(office) = self.office.as_mut() else {
                return true;
            };
            if office.resolve_native_pose(pose).is_some() {
                return true;
            }
            office.blocked = true;
            office.future_root = None;
            false
        });
        if let Some(office) = self.office.as_mut() {
            if office.native_motion_revision.is_some() && office.motion_rejection.is_none() {
                if let Some(rejection) = self.timeline.rejection.as_ref() {
                    let error = office.motion_error.clone().unwrap_or_else(|| format!("native root validation rejected session={} revision={} frame={} displayed={}: {}",
                        rejection["session_id"], rejection["generation_revision"],
                        rejection["rejected_pts_ms"], rejection["displayed_pts_ms"], rejection["reason"]));
                    tracing::error!(%error, diagnostic=%rejection, "native motion stopped without repair");
                    office.motion_error = Some(error);
                    office.motion_rejection = Some(rejection.clone());
                    office.motion_ready = false;
                    office.observed.ready = false;
                }
            }
            office.future_root = self.timeline.future_root().filter(|(_, root)| {
                office.future_handoffs && !office.blocked && office.permits_native_root(Vec3::from(*root))
            });
        }
        (pose, false)
    }
    pub fn root_translation(&mut self, pelvis: Entity, pose: &SkeletalPose) -> Option<Vec3> {
        let proposed = Vec3::from(pose.root_pos);
        let resolved = self
            .office
            .as_mut()
            .map_or(Some(proposed), |o| o.resolve_native_pose(pose));
        let allowed = resolved.is_some();
        if let Some(o) = self.office.as_mut() {
            o.observed.buffered_until_pts_ms = self.timeline.queue.back().map(|p| p.pts_ms);
            o.observed.stale_frames = self.timeline.stale_frames;
            o.observed.underrun_frames = self.timeline.underruns;
            o.observed.displayed_pts_ms = pose.pts_ms;
            if allowed {
                o.record_native_pose(pose);
                o.observed.native_stream = pose.stream.clone();
                if let Some(stream) = &pose.stream {
                    o.native_motion_revision = Some(stream.generation_revision);
                    if o.native_scene_handoff.is_some_and(|(revision, _)| stream.generation_revision >= revision) {
                        o.native_scene_handoff = None;
                    }
                }
            }
            o.motion_ready = o.motion_error.is_none();
            if !allowed {
                o.blocked = true;
            }
        }
        let position = resolved?;
        let parent = self.parents.get(pelvis).ok()?;
        Some(
            self.globals
                .get(parent.get())
                .ok()?
                .affine()
                .inverse()
                .transform_point3(position),
        )
    }
}
