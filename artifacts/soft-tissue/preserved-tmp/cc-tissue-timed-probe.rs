// Isolated optimized kernel benchmark, using the actual worktree solver source.
use bevy::math::DVec3;
#[path = "/tmp/cc-tissue-profile-source/mod.rs"]
mod soft_tissue;
use soft_tissue::{Frame, TissueAsset};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let path = args.get(1).ok_or_else(|| {
        anyhow::anyhow!(
            "tissue_probe asset.json [rate_hz=480] [iterations=8] [bulk_pa=30000] [seconds=6]"
        )
    })?;
    let argument = |index: usize, default: f64| -> anyhow::Result<f64> {
        Ok(args
            .get(index)
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(default))
    };
    let rate = argument(2, 480.0)?;
    let iterations = argument(3, 8.0)? as usize;
    let bulk = argument(4, 30000.0)?;
    let seconds = argument(5, 6.0)?;
    anyhow::ensure!((60.0..=1920.0).contains(&rate) && (0.0..=60.0).contains(&seconds));
    let asset = TissueAsset::from_json(&std::fs::read(path)?)?;
    for region in &asset.regions {
        let start = std::time::Instant::now();
        let mut region = region.clone();
        region.material.strain_damping_per_second = 60.0;
        region.material.iterations = iterations;
        region.material.bulk_pa = bulk;
        let mut tissue = region.solver(Frame::IDENTITY)?;
        for _ in 0..(seconds * rate).round() as usize {
            // This probe's gravity is supplied in the cage's local frame.
            // Real runtime uses world gravity and the actual rig frame.
            tissue.step(1.0 / rate, Frame::IDENTITY, DVec3::NEG_Y * 9.81)?;
        }
        println!(
            "{}",
            serde_json::json!({"region":region.name, "seconds":seconds,
                "rate_hz":rate, "iterations":iterations, "bulk_pa":bulk,
                "wall_seconds":start.elapsed().as_secs_f64(), "metrics":tissue.metrics()})
        );
    }
    soft_tissue::solver::implicit::profile();
    Ok(())
}
