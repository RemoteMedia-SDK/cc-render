//! Authored material topology and fixed collision references; no motion targets.
use super::{
    cloth_solver::{ClothParameters, ClothSolver, FixedClothCollider},
    scene_package::ScenePackage,
};
use anyhow::{ensure, Context, Result};
use bevy::math::{Vec2, Vec3};
use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Component, Path},
    sync::Arc,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneCloth {
    pub id: String,
    pub surface: String,
    pub sha256: String,
    pub parameters: ClothParameters,
    pub collision_solids: Vec<String>,
    #[serde(default)]
    pub collision_fixed_parts: Vec<String>,
    pub replaces_nodes: Vec<String>,
    pub color_srgb: [f32; 3],
    #[serde(skip)]
    pub(crate) loaded: Option<Arc<ClothAsset>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClothAsset {
    pub schema_version: u32,
    pub rest_positions: Vec<[f32; 3]>,
    pub initial_positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub material_uv: Vec<[f32; 2]>,
}

impl SceneCloth {
    pub(crate) fn load(&mut self, root: &Path) -> Result<()> {
        let file = root
            .join(&self.surface)
            .canonicalize()
            .context("missing cloth surface")?;
        ensure!(file.starts_with(root), "cloth surface escapes asset root");
        ensure!(
            file.metadata()?.len() <= 32 * 1024 * 1024,
            "cloth surface exceeds capacity"
        );
        let bytes = std::fs::read(file)?;
        let digest: String = hmac_sha256::Hash::hash(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        ensure!(
            digest.eq_ignore_ascii_case(&self.sha256),
            "cloth surface hash mismatch"
        );
        let asset: ClothAsset = serde_json::from_slice(&bytes).context("decode cloth surface")?;
        ensure!(
            asset.schema_version == 1
                && (3..=16384).contains(&asset.rest_positions.len())
                && asset.triangles.len() <= 32768,
            "unsupported cloth surface size or version"
        );
        self.loaded = Some(Arc::new(asset));
        self.solver()?;
        Ok(())
    }

    pub(crate) fn solver(&self) -> Result<ClothSolver> {
        let asset = self.loaded.as_ref().context("cloth surface not loaded")?;
        Ok(ClothSolver::new(
            asset
                .rest_positions
                .iter()
                .copied()
                .map(Vec3::from)
                .collect(),
            asset
                .initial_positions
                .iter()
                .copied()
                .map(Vec3::from)
                .collect(),
            asset.triangles.clone(),
            asset.material_uv.iter().copied().map(Vec2::from).collect(),
            self.parameters,
        )?)
    }

    pub(crate) fn colliders(&self, package: &ScenePackage) -> Result<Vec<FixedClothCollider>> {
        let mut colliders: Vec<_> = self.collision_solids
            .iter()
            .map(|id| {
                let solid = package
                    .static_solids()
                    .find(|s| &s.id == id)
                    .context("cloth fixed collider missing")?;
                Ok(FixedClothCollider::new(
                    solid.collider().raw.clone(),
                    solid.isometry(),
                    0.7,
                )?)
            })
            .collect::<Result<_>>()?;
        for id in &self.collision_fixed_parts {
            let (mechanism, part) = fixed_part(package, id).context("cloth fixed part missing")?;
            for shape in &part.colliders {
                let center = Vec3::from(mechanism.placement) + Vec3::from(part.position)
                    + Vec3::from(shape.position);
                let [x, y, z] = shape.half_extents;
                colliders.push(FixedClothCollider::new(
                    bevy_rapier3d::rapier::parry::shape::SharedShape::cuboid(x, y, z),
                    bevy_rapier3d::rapier::parry::math::Isometry::translation(center.x, center.y, center.z),
                    0.7,
                )?);
            }
        }
        Ok(colliders)
    }
}

fn fixed_part<'a>(package: &'a ScenePackage, id: &str)
    -> Option<(&'a super::scene_mechanism::SceneMechanism, &'a super::scene_mechanism::MechanismPart)> {
    package.mechanisms.iter().find_map(|m| m.parts.iter()
        .find(|p| p.id == id && p.mass_kg.is_none()).map(|p| (m, p)))
}

pub(crate) fn validate(package: &ScenePackage) -> Result<()> {
    ensure!(package.cloths.len() <= 8, "cloth count exceeds capacity");
    let mut ids: HashSet<&str> = package.solids.iter().map(|s| s.id.as_str()).collect();
    let mut replaced: HashSet<&str> = package
        .mechanisms
        .iter()
        .flat_map(|m| m.replaces_nodes.iter().map(String::as_str))
        .collect();
    for cloth in &package.cloths {
        ensure!(
            !cloth.id.is_empty() && ids.insert(&cloth.id),
            "duplicate/empty cloth id"
        );
        ensure!(
            !cloth.surface.is_empty()
                && Path::new(&cloth.surface)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "cloth surface must be package-relative"
        );
        ensure!(
            cloth.sha256.len() == 64 && cloth.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid cloth digest"
        );
        ensure!(
            cloth
                .color_srgb
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "invalid cloth color"
        );
        ensure!(
            cloth.replaces_nodes.iter().all(|n| !n.is_empty() && replaced.insert(n)),
            "duplicate/empty replaced cloth node"
        );
        let mut collisions = HashSet::new();
        ensure!(!cloth.collision_solids.is_empty() && cloth.collision_solids.iter().all(|id|
            collisions.insert(id) && package.static_solids().any(|s|s.id==*id)),"invalid cloth collision solids");
        ensure!(cloth.collision_fixed_parts.len() <= 32 && cloth.collision_fixed_parts.iter().all(|id|
            collisions.insert(id) && fixed_part(package, id).is_some()), "invalid cloth fixed parts");
    }
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ClothSnapshot {
    pub id: String,
    pub status: String,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub clock_origin_pts_ms: Option<u64>,
    pub displayed_pts_ms: Option<u64>,
    pub simulated_ticks: u64,
    pub surface_revision: u64,
    pub max_speed_m_s: f64,
    pub max_edge_stretch_ratio: f64,
    pub max_particle_penetration_m: f64,
    pub surface_position_sha256: String,
    pub mesh_position_sha256: String,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub particles: usize,
    pub hand_contacts: Vec<ClothHandContacts>,
}

/// Read-only distal-capsule observations. Contact is not pinch admission.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ClothHandContacts {
    pub hand: super::renderer::Hand,
    pub status: String,
    pub error: Option<String>,
    pub fingers: Vec<ClothFingerContacts>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ClothFingerContacts {
    pub finger: String,
    pub segment_start: [f32; 3],
    pub segment_end: [f32; 3],
    pub radius_m: f32,
    pub contacts: Vec<ClothContactWitness>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ClothContactWitness {
    pub triangle: usize,
    pub barycentric: [f32; 3],
    pub surface_revision: u64,
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub separation_m: f32,
}
