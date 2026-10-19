//! Read-only source-time measurements for the dedicated tissue lab.
use super::IntegrationReport;
use crate::cc_render::soft_tissue::{Attachment, Tissue, TissueAsset};
use bevy::math::DVec3;
use std::{fs::File, io::{BufWriter, Write}, sync::Mutex};

pub(super) struct Trace {
    output: Mutex<(BufWriter<File>, Option<u64>)>,
    include_nodes: bool,
}

impl Trace {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        std::env::var_os("CC_AVATAR_TISSUE_TRACE").map(|path| {
            let file = File::options().write(true).create_new(true).open(path)?;
            Ok(Self {
                output: Mutex::new((BufWriter::new(file), None)),
                include_nodes: std::env::var("CC_AVATAR_TISSUE_TRACE_NODES").as_deref() == Ok("1"),
            })
        }).transpose()
    }

    pub fn record(&self, pts: u64, asset: &TissueAsset, solvers: &[Tissue],
        integration: &IntegrationReport, surface_max_m: f64, crease_deg: f64, skin_error_m: f64) -> anyhow::Result<()> {
        let mut output = self.output.lock().map_err(|_| anyhow::anyhow!("tissue trace lock poisoned"))?;
        if output.1 == Some(pts) { return Ok(()); }
        let regions: Vec<_> = asset.regions.iter().zip(solvers).map(|(region, solver)| {
            let frame = solver.frame();
            let offsets: Vec<_> = solver.positions().iter().zip(&region.initial_positions)
                .zip(&region.attachments).filter(|(_, a)| !matches!(a, Attachment::Fixed))
                .map(|((p, initial), _)| frame.rotation.inverse() * (*p - frame.translation) - DVec3::from(*initial))
                .collect();
            let mean = offsets.iter().copied().sum::<DVec3>() / offsets.len().max(1) as f64;
            let mut row = serde_json::json!({"name":region.name, "anchor_world_m":frame.translation.to_array(),
                "anchor_rotation_xyzw":frame.rotation.to_array(),
                "free_node_centroid_offset_m":mean.to_array(),
                "maximum_node_offset_m":offsets.iter().map(|p|p.length()).fold(0.0_f64, f64::max),
                "metrics":solver.metrics()});
            if self.include_nodes {
                row["nodes_world_m"] = serde_json::json!(solver.positions().iter().map(|p| p.to_array()).collect::<Vec<_>>());
            }
            row
        }).collect();
        writeln!(output.0, "{}", serde_json::json!({"pts_ms":pts,"regions":regions,
            "integration":integration,"maximum_surface_displacement_m":surface_max_m,
            "maximum_crease_deg":crease_deg,"skin_error_m":skin_error_m}))?;
        output.0.flush()?;
        output.1 = Some(pts);
        Ok(())
    }
}
