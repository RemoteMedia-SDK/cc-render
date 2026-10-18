//! Read-only retarget/FK sampling for secondary tissue. No skeleton writes.
use super::pose::{fk_retarget_smpl_to_cc5_with_align, CC5BindRotations};
use super::twist::{hand_roll, twist_locals, ForearmRig};
use super::tissue_collision::ContactPose;
use crate::cc_render::{renderer::SkeletalPose, soft_tissue::Frame};
use bevy::{math::DQuat, prelude::*};
use std::{cell::{Cell, RefCell}, collections::{HashMap, VecDeque}, sync::Arc};

/// Bounded memoization for one immutable motion/morph window. Create a fresh
/// cache for every rendered frame, including paused time and session changes.
#[derive(Default)]
pub(super) struct ContactPoseCache {
    entries: RefCell<VecDeque<(u64, Arc<ContactPose>)>>,
    requests: Cell<usize>,
    misses: Cell<usize>,
}

impl ContactPoseCache {
    pub fn sample(
        &self,
        at_ms: f64,
        sample: impl FnOnce() -> anyhow::Result<ContactPose>,
    ) -> anyhow::Result<Arc<ContactPose>> {
        anyhow::ensure!(at_ms.is_finite(), "nonfinite contact sample time");
        self.requests.set(self.requests.get() + 1);
        let key = at_ms.to_bits();
        let mut entries = self.entries.borrow_mut();
        if let Some(index) = entries.iter().position(|(time, _)| *time == key) {
            let entry = entries.remove(index).unwrap();
            let result = Arc::clone(&entry.1);
            entries.push_back(entry);
            return Ok(result);
        }
        self.misses.set(self.misses.get() + 1);
        let result = Arc::new(sample()?);
        if entries.len() == 8 { entries.pop_front(); }
        entries.push_back((key, Arc::clone(&result)));
        Ok(result)
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.misses.get(), self.requests.get() - self.misses.get())
    }
}

struct Node {
    entity: Entity,
    parent: Option<usize>,
    local: Transform,
    mapped: Option<usize>,
}

pub(super) struct TissueRig {
    nodes: Vec<Node>,
    index: HashMap<Entity, usize>,
    calibrated_world: [Quat; 22],
    inter_rest: [Quat; 22],
    pelvis_parent: Quat,
    align: Quat,
    twists: Vec<(ForearmRig, usize)>,
}

impl TissueRig {
    pub fn capture(
        targets: &[Entity],
        transforms: &Query<(&Transform, Option<&Parent>)>,
        bind: &CC5BindRotations,
        twists: &[ForearmRig],
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            std::env::var("CC_AVATAR_RETARGET_MODE").unwrap_or_else(|_| "calibrated".into())
                == "calibrated",
            "volumetric tissue currently requires the calibrated retarget path"
        );
        let degrees = std::env::var("CC_AVATAR_SMPL_ALIGN_Y_DEG")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0);
        let mut rig = Self {
            nodes: Vec::new(),
            index: HashMap::new(),
            calibrated_world: bind.calibrated_world,
            inter_rest: bind.inter_rest,
            pelvis_parent: bind.parent_chain_rest[0],
            align: Quat::from_rotation_y(degrees.to_radians()),
            twists: Vec::new(),
        };
        for target in targets {
            rig.add_node(*target, transforms, bind)?;
        }
        for forearm in twists {
            if !rig.index.contains_key(&forearm.hand) {
                continue;
            }
            let mapped = bind
                .entities
                .iter()
                .position(|e| *e == Some(forearm.hand))
                .ok_or_else(|| anyhow::anyhow!("unmapped tissue contact hand"))?;
            rig.twists.push((*forearm, mapped));
        }
        Ok(rig)
    }

    fn add_node(
        &mut self,
        entity: Entity,
        transforms: &Query<(&Transform, Option<&Parent>)>,
        bind: &CC5BindRotations,
    ) -> anyhow::Result<usize> {
        if let Some(index) = self.index.get(&entity) {
            return Ok(*index);
        }
        let (local, parent) = transforms
            .get(entity)
            .map_err(|e| anyhow::anyhow!("tissue rig node {entity:?}: {e}"))?;
        let parent = parent
            .map(|p| self.add_node(p.get(), transforms, bind))
            .transpose()?;
        let index = self.nodes.len();
        self.nodes.push(Node {
            entity,
            parent,
            local: *local,
            mapped: bind.entities.iter().position(|e| *e == Some(entity)),
        });
        self.index.insert(entity, index);
        Ok(index)
    }

    pub fn sample(&self, pose: &SkeletalPose) -> HashMap<Entity, Mat4> {
        let rotations = fk_retarget_smpl_to_cc5_with_align(
            &pose.joint_quats,
            &self.calibrated_world,
            &self.inter_rest,
            self.pelvis_parent,
            self.align,
        );
        let mut matrices = Vec::<Mat4>::with_capacity(self.nodes.len());
        let mut twist_rotations = HashMap::new();
        for (rig, mapped) in &self.twists {
            let (a, b) = twist_locals(rig, hand_roll(rotations[*mapped], rig.hand_rest, rig.axis));
            twist_rotations.insert(rig.twist01, a);
            twist_rotations.insert(rig.twist02, b);
        }
        let mut result = HashMap::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let parent = node.parent.map_or(Mat4::IDENTITY, |i| matrices[i]);
            let mut local = node.local;
            if let Some(mapped) = node.mapped {
                local.rotation = rotations[mapped];
                if mapped == 0 {
                    local.translation = parent.inverse().transform_point3(pose.root_pos.into());
                }
            }
            if let Some(rotation) = twist_rotations.get(&node.entity) {
                local.rotation = *rotation;
            }
            let world = parent * local.compute_matrix();
            matrices.push(world);
            result.insert(node.entity, world);
        }
        result
    }
}

pub(super) fn frame(matrix: Mat4) -> anyhow::Result<Frame> {
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    anyhow::ensure!(
        scale.abs_diff_eq(Vec3::ONE, 1e-4) && matrix.is_finite(),
        "tissue anchor must use the calibrated metre scale"
    );
    Ok(Frame {
        translation: translation.as_dvec3(),
        rotation: DQuat::from_array(rotation.to_array().map(f64::from)).normalize(),
    })
}

/// Sample only between exact committed source knots, including skipped knots.
/// This is the same adjacent-pose interpolation as ordinary playback.
pub(super) fn sample_pose(knots: &[SkeletalPose], at_ms: f64) -> anyhow::Result<SkeletalPose> {
    anyhow::ensure!(
        !knots.is_empty() && at_ms.is_finite(),
        "missing tissue motion knots"
    );
    let first = knots.first().unwrap();
    let last = knots.last().unwrap();
    anyhow::ensure!(
        at_ms >= first.pts_ms as f64 - 1e-6 && at_ms <= last.pts_ms as f64 + 1e-6,
        "tissue motion window does not bracket {at_ms} ms"
    );
    let right = knots.partition_point(|p| (p.pts_ms as f64) < at_ms);
    if right == 0 {
        return Ok(first.clone());
    }
    if right == knots.len() {
        return Ok(last.clone());
    }
    let (a, b) = (&knots[right - 1], &knots[right]);
    anyhow::ensure!(b.pts_ms > a.pts_ms, "unordered tissue motion knots");
    let alpha = ((at_ms - a.pts_ms as f64) / (b.pts_ms - a.pts_ms) as f64) as f32;
    let mut out = a.clone();
    out.pts_ms = at_ms.round() as u64;
    out.root_pos = Vec3::from(a.root_pos)
        .lerp(Vec3::from(b.root_pos), alpha)
        .to_array();
    for i in 0..22 {
        out.joint_quats[i] = Quat::from_array(a.joint_quats[i])
            .slerp(Quat::from_array(b.joint_quats[i]), alpha)
            .to_array();
    }
    Ok(out)
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn geometry(value: f64) -> ContactPose {
        ContactPose { frames: vec![Frame::IDENTITY], skin: vec![bevy::math::DVec3::X * value] }
    }

    #[test]
    fn tissue_pose_cache_uses_exact_times_and_does_not_outlive_its_window() {
        let cache = ContactPoseCache::default();
        let at = 4.25_f64;
        let first = cache.sample(at, || Ok(geometry(1.))).unwrap();
        let repeated = cache.sample(at, || panic!("same timestamp sampled twice")).unwrap();
        assert!(Arc::ptr_eq(&first, &repeated));
        let adjacent = f64::from_bits(at.to_bits() + 1);
        let distinct = cache.sample(adjacent, || Ok(geometry(2.))).unwrap();
        assert_eq!(distinct.skin[0].x, 2.);
        assert!(!Arc::ptr_eq(&first, &distinct));
        for tick in 20..29 {
            cache.sample(tick as f64, || Ok(geometry(tick as f64))).unwrap();
            assert!(cache.entries.borrow().len() <= 8);
        }
        let evicted = cache.sample(at, || Ok(geometry(3.))).unwrap();
        assert_eq!(evicted.skin[0].x, 3.);
        assert_eq!(first.skin[0].x, 1.);
        assert_eq!(cache.counts(), (12, 1));
        // A new presentation can observe different morphs at the same time.
        let next_window = ContactPoseCache::default();
        let updated = next_window.sample(at, || Ok(geometry(4.))).unwrap();
        assert_eq!(updated.skin[0].x, 4.);
        assert_eq!(next_window.counts(), (1, 0));
    }

    #[test]
    fn tissue_pose_cache_does_not_hide_failed_or_nonfinite_samples() {
        let cache = ContactPoseCache::default();
        for _ in 0..2 {
            assert!(cache.sample(1., || anyhow::bail!("source unavailable")).is_err());
        }
        assert!(cache.entries.borrow().is_empty());
        assert_eq!(cache.counts(), (2, 0));
        let recovered = cache.sample(1., || Ok(geometry(5.))).unwrap();
        assert_eq!(recovered.skin[0].x, 5.);
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(cache.sample(invalid, || panic!("nonfinite time reached sampler")).is_err());
        }
        assert_eq!(cache.counts(), (3, 0));
    }
}
