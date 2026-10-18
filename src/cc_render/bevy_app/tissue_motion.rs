//! Read-only retarget/FK sampling for secondary tissue. No skeleton writes.
use super::pose::{fk_retarget_smpl_to_cc5_with_align, CC5BindRotations};
use super::twist::{hand_roll, twist_locals, ForearmRig};
use crate::cc_render::{renderer::SkeletalPose, soft_tissue::Frame};
use bevy::{math::DQuat, prelude::*};
use std::collections::HashMap;

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
