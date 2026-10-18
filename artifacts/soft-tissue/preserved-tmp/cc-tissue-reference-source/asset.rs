//! Versioned authoring data; exact source-surface hashes bind this to the avatar.
use super::{Attachment, Frame, Material, Tissue, TissueError};
use bevy::math::DVec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TissueAsset {
    pub schema_version: u32,
    pub source_sha256: Vec<String>,
    pub regions: Vec<RegionAsset>,
    pub surfaces: Vec<SurfaceBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegionAsset {
    pub name: String,
    pub anchor_bone: String,
    pub rest_positions: Vec<[f64; 3]>,
    pub initial_positions: Vec<[f64; 3]>,
    pub tetrahedra: Vec<[usize; 4]>,
    pub attachments: Vec<Attachment>,
    pub material: Material,
}

impl RegionAsset {
    pub fn solver(&self, frame: Frame) -> Result<Tissue, TissueError> {
        Tissue::new(
            self.rest_positions
                .iter()
                .copied()
                .map(DVec3::from)
                .collect(),
            self.initial_positions
                .iter()
                .copied()
                .map(DVec3::from)
                .collect(),
            self.tetrahedra.clone(),
            self.attachments.clone(),
            self.material,
            frame,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceBinding {
    pub mesh_name: String,
    pub positions_sha256: String,
    pub vertex_count: usize,
    pub vertices: Vec<VertexBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VertexBinding {
    pub vertex: usize,
    pub region: usize,
    pub tetrahedron: usize,
    pub barycentric: [f64; 4],
    pub influence: f64,
}

impl TissueAsset {
    pub fn from_json(bytes: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "tissue asset exceeds capacity"
        );
        let asset: Self = serde_json::from_slice(bytes)?;
        asset.validate()?;
        Ok(asset)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.source_sha256.is_empty()
                && self.source_sha256.len() <= 64
                && self
                    .source_sha256
                    .iter()
                    .all(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())),
            "missing or invalid tissue source fingerprints"
        );
        anyhow::ensure!(
            self.schema_version == 1
                && (1..=8).contains(&self.regions.len())
                && (1..=64).contains(&self.surfaces.len()),
            "unsupported tissue asset version or size"
        );
        let mut names = BTreeSet::new();
        for region in &self.regions {
            anyhow::ensure!(
                !region.name.is_empty()
                    && !region.anchor_bone.is_empty()
                    && names.insert(&region.name),
                "missing or duplicate tissue region"
            );
            anyhow::ensure!(
                region
                    .attachments
                    .iter()
                    .filter(|a| matches!(a, Attachment::Fixed))
                    .count()
                    >= 3,
                "tissue requires a distributed fixed attachment footprint"
            );
            region.solver(Frame::IDENTITY)?;
        }
        let mut hashes = BTreeSet::new();
        for surface in &self.surfaces {
            anyhow::ensure!(
                surface.positions_sha256.len() == 64
                    && surface
                        .positions_sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit())
                    && hashes.insert(&surface.positions_sha256),
                "invalid or duplicate tissue surface hash"
            );
            anyhow::ensure!(
                (1..=1_000_000).contains(&surface.vertex_count)
                    && !surface.vertices.is_empty()
                    && surface.vertices.len() <= 100_000,
                "tissue surface capacity"
            );
            self.validate_bindings(surface)?;
        }
        Ok(())
    }

    fn validate_bindings(&self, surface: &SurfaceBinding) -> anyhow::Result<()> {
        let mut unique = BTreeSet::new();
        for binding in &surface.vertices {
            anyhow::ensure!(
                binding.vertex < surface.vertex_count && binding.region < self.regions.len(),
                "tissue vertex or region index"
            );
            anyhow::ensure!(
                binding.tetrahedron < self.regions[binding.region].tetrahedra.len()
                    && unique.insert((binding.vertex, binding.region)),
                "tissue tet index or duplicate embedding"
            );
            anyhow::ensure!(
                binding
                    .barycentric
                    .iter()
                    .all(|w| w.is_finite() && *w >= 0.0 && *w <= 1.0)
                    && (binding.barycentric.iter().sum::<f64>() - 1.0).abs() < 1e-8
                    && binding.influence.is_finite()
                    && (0.0..=1.0).contains(&binding.influence),
                "invalid tissue embedding weights"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> TissueAsset {
        TissueAsset::from_json(include_bytes!(
            "../../../assets/tissue/beautiful_assistant.unfitted.json"
        ))
        .unwrap()
    }

    #[test]
    fn real_avatar_asset_is_volumetric_and_bound_to_body_and_outfit() {
        let asset = asset();
        assert_eq!(asset.regions.len(), 2);
        assert!(asset.surfaces.iter().any(|s| s.mesh_name == "CC_Base_Body"));
        assert!(asset
            .surfaces
            .iter()
            .any(|s| s.mesh_name == "Condo cream knit top"));
        for region in &asset.regions {
            assert!(region.tetrahedra.len() > 500);
            let metrics = region.solver(Frame::IDENTITY).unwrap().metrics();
            assert!(
                metrics.mass_kg > 0.5 && metrics.mass_kg < 1.0,
                "{metrics:?}"
            );
            assert!((metrics.minimum_volume_ratio - 1.0).abs() < 1e-8);
        }
    }

    #[test]
    fn malformed_embedding_and_assets_fail_closed() {
        let original = asset();
        let mut broken = original.clone();
        broken.surfaces[0].vertices[0].barycentric = [0.1; 4];
        assert!(broken.validate().is_err());
        broken = original.clone();
        broken.regions[0].tetrahedra[0].swap(0, 1);
        assert!(broken.validate().is_err());
        broken = original;
        broken.regions[0].attachments.fill(Attachment::Free);
        assert!(broken.validate().is_err());
    }

    #[test]
    fn real_cages_settle_under_gravity_without_residual_element_vibration() {
        for mut region in asset().regions {
            region.material.strain_damping_per_second = 60.0;
            let mut solver = region.solver(Frame::IDENTITY).unwrap();
            for _ in 0..2880 {
                solver
                    .step(1.0 / 480.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81)
                    .unwrap();
            }
            let metrics = solver.metrics();
            eprintln!("{}: {metrics:?}", region.name);
            assert!(
                metrics.maximum_speed_m_s < 0.002,
                "{}: {metrics:?}",
                region.name
            );
            assert!(metrics.minimum_volume_ratio > 0.9 && metrics.maximum_volume_ratio < 1.15);
        }
    }

    #[test]
    fn fitted_prototype_reproduces_the_authored_loaded_shape() {
        let asset = TissueAsset::from_json(include_bytes!(
            "../../../assets/tissue/beautiful_assistant.fitted-prototype.json"
        ))
        .unwrap();
        for region in &asset.regions {
            let mut solver = region.solver(Frame::IDENTITY).unwrap();
            let error = solver.equilibrate(DVec3::NEG_Y * 9.81, 2048).unwrap();
            assert!(error < 2e-5);
            let distance = solver
                .positions()
                .iter()
                .zip(&region.initial_positions)
                .map(|(p, q)| p.distance(DVec3::from(*q)))
                .fold(0.0, f64::max);
            assert!(distance < 0.00025, "{}: {distance}", region.name);
            assert_eq!(solver.metrics().kinetic_energy_j, 0.0);
        }
    }
}
