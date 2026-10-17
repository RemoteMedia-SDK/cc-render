//! Passive authored furniture. Contains geometry and joint frames, never motion commands.
use anyhow::{ensure, Context, Result};
use bevy::prelude::Vec3;
use serde::Deserialize;
use std::{collections::HashSet, path::Path};
use super::scene_package::positive;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneMechanism {
    pub id: String,
    pub placement: [f32; 3],
    pub replaces_solid: String,
    pub replaces_nodes: Vec<String>,
    pub parts: Vec<MechanismPart>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MechanismPart {
    pub id: String,
    pub geometry: String,
    pub sha256: String,
    pub position: [f32; 3],
    pub mass_kg: Option<f32>,
    pub colliders: Vec<MechanismBox>,
    pub joint: Option<PassiveJoint>,
    pub handle_region: Option<HandleRegion>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MechanismBox {
    pub id: String,
    pub position: [f32; 3],
    pub half_extents: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandleRegion {
    pub mesh: String,
    pub centre: [f32; 3],
    pub half_extents: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointKind { Revolute, Prismatic }

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassiveJoint {
    pub kind: JointKind,
    pub parent: String,
    pub parent_anchor: [f32; 3],
    pub child_anchor: [f32; 3],
    pub axis: [f32; 3],
    pub limits_rad: Option<[f32; 2]>,
    pub limits_m: Option<[f32; 2]>,
}

impl PassiveJoint {
    pub(crate) fn limits(&self) -> Result<[f32; 2]> {
        let (limits, maximum) = match (&self.kind, self.limits_rad, self.limits_m) {
            (JointKind::Revolute, Some(limits), None) => (limits, std::f32::consts::TAU),
            (JointKind::Prismatic, None, Some(limits)) => (limits, 10.),
            _ => anyhow::bail!("joint limits must use exactly the matching physical unit"),
        };
        let [lo, hi] = limits;
        ensure!(lo.is_finite() && hi.is_finite() && lo <= 0. && hi >= 0.
            && hi > lo && hi - lo <= maximum, "invalid joint limits or closed rest position");
        Ok(limits)
    }
}

impl MechanismPart {
    /// Centre the runtime body on its authored bounds. Render/collider/hinge
    /// offsets all subtract this same constant; no animated transforms change.
    pub(crate) fn bounds(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for shape in &self.colliders {
            let center = Vec3::from(shape.position);
            let half = Vec3::from(shape.half_extents);
            min = min.min(center - half);
            max = max.max(center + half);
        }
        ((min + max) * 0.5, (max - min) * 0.5)
    }

    fn validate(&self) -> Result<()> {
        ensure!(!self.id.is_empty() && Vec3::from(self.position).is_finite(), "invalid mechanism part");
        ensure!(!self.geometry.is_empty() && Path::new(&self.geometry).components().all(|c|
            matches!(c, std::path::Component::Normal(_))), "part geometry must be package-relative");
        ensure!(self.sha256.len() == 64 && self.sha256.bytes().all(|c| c.is_ascii_hexdigit()), "invalid part digest");
        ensure!((1..=64).contains(&self.colliders.len()), "invalid collider count");
        let mut ids = HashSet::new();
        for shape in &self.colliders {
            ensure!(!shape.id.is_empty() && ids.insert(&shape.id), "duplicate/empty collider id");
            ensure!(Vec3::from(shape.position).is_finite() && positive(&shape.half_extents), "invalid collider box");
        }
        ensure!(self.mass_kg.is_some() == self.joint.is_some(), "dynamic parts require a passive joint; fixed parts cannot have one");
        ensure!(self.mass_kg.is_none_or(|m| m.is_finite() && m > 0. && m <= 1000.), "invalid mass");
        if let Some(region) = &self.handle_region {
            ensure!(self.mass_kg.is_some() && self.colliders.iter().any(|shape|
                shape.id == region.mesh && shape.position == region.centre
                    && shape.half_extents == region.half_extents), "handle region must match a dynamic collider");
        }
        Ok(())
    }

    pub(crate) fn verify_asset(&self, folder: &Path) -> Result<()> {
        let path = folder.join(&self.geometry).canonicalize().context("missing mechanism geometry")?;
        ensure!(path.starts_with(folder), "mechanism geometry escapes scene asset root");
        let bytes = std::fs::read(path)?;
        let digest: String = hmac_sha256::Hash::hash(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        ensure!(digest.eq_ignore_ascii_case(&self.sha256), "mechanism geometry hash mismatch: {}", self.id);
        Ok(())
    }
}

impl SceneMechanism {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(!self.id.is_empty() && Vec3::from(self.placement).is_finite(), "invalid mechanism");
        ensure!(!self.replaces_nodes.is_empty() && self.replaces_nodes.iter().all(|n| !n.is_empty()), "mechanism needs explicit replaced nodes");
        ensure!((2..=32).contains(&self.parts.len()), "invalid mechanism part count");
        for part in &self.parts {
            part.validate()?;
            if let Some(joint) = &part.joint { self.validate_joint(part, joint)?; }
        }
        Ok(())
    }

    fn validate_joint(&self, part: &MechanismPart, joint: &PassiveJoint) -> Result<()> {
        let parent = self.parts.iter().find(|p| p.id == joint.parent).context("missing joint parent")?;
        ensure!(parent.mass_kg.is_none() && parent.id != part.id, "only joints to fixed parents are supported");
        ensure!(Vec3::from(joint.parent_anchor).is_finite() && Vec3::from(joint.child_anchor).is_finite(), "invalid joint anchor");
        let axis = Vec3::from(joint.axis);
        ensure!(axis.is_finite() && (axis.length() - 1.).abs() < 1e-5, "joint axis must be normalized");
        match joint.kind {
            JointKind::Revolute => ensure!(axis == Vec3::Y, "revolute mechanisms must be upright"),
            JointKind::Prismatic => ensure!(axis.y == 0., "prismatic mechanisms must be horizontal"),
        }
        joint.limits()?;
        let a = Vec3::from(parent.position) + Vec3::from(joint.parent_anchor);
        let b = Vec3::from(part.position) + Vec3::from(joint.child_anchor);
        ensure!(a.distance(b) < 1e-4, "rest joint anchors do not coincide");
        Ok(())
    }

    pub(crate) fn office_objects(&self) -> impl Iterator<Item = super::office::OfficeObject> + '_ {
        self.parts.iter().map(|part| {
            let (center, half) = part.bounds();
            super::office::OfficeObject {
                id: part.id.clone(), label: part.id.replace('_', " "),
                position: (Vec3::from(self.placement) + Vec3::from(part.position) + center).to_array(),
                half_extents: half.to_array(), yaw_deg: 0., color: [0.5; 3],
                parts: part.colliders.iter().map(|s| super::office::OfficePart {
                    position: (Vec3::from(s.position) - center).to_array(), half_extents: s.half_extents,
                    color: None, cylinder: false,
                }).collect(),
                dynamic: part.mass_kg.is_some(), pushable: false, rotation_xyzw: None,
                keypoints: Default::default(), approach: [0.; 3], seat: None, seat_surface_part: None,
                grasp: part.handle_region.as_ref().map(|h| (Vec3::from(h.centre) - center).to_array()),
                surface: None, zones: Vec::new(), interactions: Vec::new(),
            }
        })
    }
}

#[cfg(test)]
#[path = "scene_mechanism_tests.rs"]
mod tests;

pub(crate) fn validate_mechanisms(mechanisms: &[SceneMechanism], solids: &[super::scene_package::SceneSolid]) -> Result<()> {
    ensure!(mechanisms.len() <= 32, "mechanism capacity exceeded");
    let mut ids: HashSet<&str> = solids.iter().map(|s| s.id.as_str()).collect();
    let mut mechanisms_ids = HashSet::new();
    let mut replaced = HashSet::new();
    let mut nodes = HashSet::new();
    for mechanism in mechanisms {
        mechanism.validate()?;
        ensure!(mechanisms_ids.insert(&mechanism.id), "duplicate mechanism id");
        ensure!(solids.iter().any(|s| s.id == mechanism.replaces_solid)
            && replaced.insert(&mechanism.replaces_solid), "invalid/duplicate replacement solid");
        for node in &mechanism.replaces_nodes { ensure!(nodes.insert(node), "duplicate replacement node"); }
        for part in &mechanism.parts { ensure!(ids.insert(&part.id), "duplicate mechanism part id"); }
    }
    Ok(())
}
