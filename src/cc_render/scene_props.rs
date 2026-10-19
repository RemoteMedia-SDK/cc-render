//! Loose and fixed authored props. Geometry and initial placement, never motion.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::path::{Component, Path};
use super::office::OfficeObject;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneProp {
    pub object: OfficeObject,
    pub geometry: String,
    pub sha256: String,
    pub mass_kg: Option<f32>,
}

impl SceneProp {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.object.dynamic == self.mass_kg.is_some(), "prop mass must match dynamic state");
        ensure!(self.mass_kg.is_none_or(|m| m.is_finite() && m > 0. && m <= 100.), "invalid prop mass");
        ensure!(!self.object.pushable, "loose props cannot use legacy push control");
        ensure!(!self.geometry.is_empty() && Path::new(&self.geometry).components().all(|c|
            matches!(c, Component::Normal(_))), "prop geometry must be package-relative");
        ensure!(self.sha256.len() == 64 && self.sha256.bytes().all(|b| b.is_ascii_hexdigit()), "invalid prop digest");
        ensure!(self.object.parts.len() <= 64, "prop collider capacity exceeded");
        for part in &self.object.parts {
            ensure!(!part.cylinder || (part.half_extents[0] - part.half_extents[2]).abs() < 1e-6,
                "cylinder must have equal horizontal radii");
        }
        Ok(())
    }

    pub(crate) fn verify_asset(&self, folder: &Path) -> Result<()> {
        let path = folder.join(&self.geometry).canonicalize().context("missing prop geometry")?;
        ensure!(path.starts_with(folder), "prop geometry escapes scene asset root");
        let bytes = std::fs::read(path)?;
        let digest: String = hmac_sha256::Hash::hash(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        ensure!(digest.eq_ignore_ascii_case(&self.sha256), "prop geometry hash mismatch: {}", self.object.id);
        Ok(())
    }
}
