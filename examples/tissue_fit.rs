//! Offline reference-shape fitting; never moves the avatar or edits a GLB.
use bevy::math::DVec3;
use cc_render_plugin::cc_render::soft_tissue::{Attachment, Frame, TissueAsset};
use std::io::Write;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        (3..=4).contains(&args.len()),
        "tissue_fit unfitted.json fitted.json [bulk_pa]"
    );
    anyhow::ensure!(!std::path::Path::new(&args[2]).exists(), "output exists");
    let mut asset = TissueAsset::from_json(&std::fs::read(&args[1])?)?;
    for region in &mut asset.regions {
        if let Some(bulk) = args.get(3) {
            region.material.bulk_pa = bulk.parse()?;
        }
        // Static equilibrium has no damping; preserve the authored material.
        let mut converged = false;
        for iteration in 0..40 {
            let mut solver = region.solver(Frame::IDENTITY)?;
            let residual = solver.equilibrate(DVec3::NEG_Y * 9.81, 2048)?;
            let errors: Vec<_> = region
                .initial_positions
                .iter()
                .zip(solver.positions())
                .map(|(target, actual)| DVec3::from(*target) - *actual)
                .collect();
            let maximum = errors.iter().map(|d| d.length()).fold(0.0, f64::max);
            println!(
                "{}",
                serde_json::json!({"region":region.name, "iteration":iteration,
                "maximum_shape_error_m":maximum, "force_residual_n":residual,
                "metrics":solver.metrics()})
            );
            if maximum < 0.00025 {
                converged = true;
                break;
            }
            let previous = region.rest_positions.clone();
            let mut fraction = 0.5;
            loop {
                for (i, rest) in region.rest_positions.iter_mut().enumerate() {
                    if !matches!(region.attachments[i], Attachment::Fixed) {
                        *rest = (DVec3::from(previous[i]) + fraction * errors[i]).to_array();
                    }
                }
                if region.solver(Frame::IDENTITY).is_ok() {
                    break;
                }
                fraction *= 0.5;
                anyhow::ensure!(fraction >= 1.0 / 1024.0, "invalid reference fit");
            }
        }
        anyhow::ensure!(converged, "{} shape fit did not converge", region.name);
    }
    asset.validate()?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    output.write_all(&serde_json::to_vec(&asset)?)?;
    output.write_all(b"\n")?;
    Ok(())
}
