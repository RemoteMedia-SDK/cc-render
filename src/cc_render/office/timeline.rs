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
    pub stale_frames: u64,
    pub underruns: u64,
}
impl PoseTimeline {
    pub fn invalidate(&mut self, revision: u64) {
        self.schedule_revision(revision, None);
    }
    pub fn schedule_revision(&mut self, revision: u64, handoff: Option<u64>) {
        if revision > self.revision {
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
        if self.queue.len() < 32 {
            self.queue.push_back(p);
        } // at most 1.6 seconds at 20 Hz
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
mod tests {
    use super::*;
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
    pub fn receive(
        &mut self,
        rx: &crossbeam_channel::Receiver<Option<SkeletalPose>>,
    ) -> (Option<SkeletalPose>, bool) {
        if let Some(o) = self.office.as_ref() {
            self.timeline
                .schedule_revision(o.revision, o.motion.handoff_pts_ms);
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
            let position = office.supported_root(Vec3::from(pose.root_pos));
            if office.permits_root(position) {
                return true;
            }
            office.blocked = true;
            office.future_root = None;
            false
        });
        if let Some(office) = self.office.as_mut() {
            office.future_root = self
                .timeline
                .future_root()
                .filter(|(_, root)| !office.blocked && office.permits_root(Vec3::from(*root)));
        }
        (pose, false)
    }
    pub fn root_translation(&mut self, pelvis: Entity, pose: &SkeletalPose) -> Option<Vec3> {
        let proposed = Vec3::from(pose.root_pos);
        let position = self
            .office
            .as_mut()
            .map_or(proposed, |o| o.supported_root(proposed));
        let allowed = self
            .office
            .as_ref()
            .map_or(true, |o| o.permits_root(position));
        if let Some(o) = self.office.as_mut() {
            o.observed.stale_frames = self.timeline.stale_frames;
            o.observed.underrun_frames = self.timeline.underruns;
            o.observed.displayed_pts_ms = pose.pts_ms;
            o.motion_ready = o.motion_error.is_none();
            if !allowed {
                o.blocked = true;
            }
        }
        if !allowed {
            return None;
        }
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
