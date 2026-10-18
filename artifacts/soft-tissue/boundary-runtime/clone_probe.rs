// Allocation/copy microbenchmark only; this does not measure playback or physics.
use soft_tissue::{Frame, TissueAsset};
use std::{hint::black_box, time::Instant};

fn measure(mut operation: impl FnMut(), iterations: usize) -> f64 {
    let started = Instant::now();
    for _ in 0..iterations { operation(); }
    started.elapsed().as_secs_f64()
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("asset path");
    let asset = TissueAsset::from_json(&std::fs::read(path)?)?;
    let mut solvers = asset.regions.iter().map(|r| r.solver(Frame::IDENTITY))
        .collect::<Result<Vec<_>, _>>()?;
    for solver in &mut solvers {
        solver.step(1. / 240., Frame::IDENTITY, bevy::math::DVec3::NEG_Y * 9.81)?;
    }
    let without_pressure: Vec<_> = solvers.iter().map(|s| s.probe_without_pressure()).collect();
    let iterations = 10_000;
    let mut trials = Vec::new();
    for _ in 0..3 {
        let full = measure(|| { black_box(black_box(&solvers).clone()); }, iterations);
        let pressure = measure(|| {
            for solver in black_box(&solvers) { solver.probe_pressure_clone(); }
        }, iterations);
        let other = measure(|| { black_box(black_box(&without_pressure).clone()); }, iterations);
        trials.push(serde_json::json!({"full_seconds":full, "pressure_only_seconds":pressure,
            "without_pressure_seconds":other}));
    }
    println!("{}", serde_json::json!({"pair_clones_per_trial":iterations,
        "regions":solvers.len(), "trials":trials,
        "limitation":"Isolated clone/drop workload after one step; not a playback benchmark."}));
    Ok(())
}
