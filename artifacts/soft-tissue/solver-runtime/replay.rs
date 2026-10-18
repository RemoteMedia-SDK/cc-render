// CPU-only workload from observed attachment frames. No avatar pose is authored
// or repaired. This interpolated-frame benchmark is not GPU acceptance evidence.
use bevy::math::{DQuat, DVec3};
use soft_tissue::{Frame, Tissue, TissueAsset, TissueError};
use std::io::Write;

#[derive(serde::Deserialize)]
struct Region {
    anchor_world_m: [f64; 3],
    anchor_rotation_xyzw: [f64; 4],
}
impl Region {
    fn frame(&self) -> Frame {
        Frame {
            translation: DVec3::from(self.anchor_world_m),
            rotation: DQuat::from_array(self.anchor_rotation_xyzw),
        }
    }
}
#[derive(serde::Deserialize)]
struct Sample {
    pts_ms: f64,
    regions: Vec<Region>,
}

fn advance(solver: &mut Tissue, start: f64, end: f64, depth: usize,
    frame_at: &impl Fn(f64) -> Frame) -> Result<usize, TissueError> {
    match solver.step((end-start)/1000.0, frame_at(end), DVec3::NEG_Y*9.81) {
        Ok(()) => Ok(0),
        Err(TissueError::InvertedTet(_) | TissueError::NonlinearSolveFailed { .. }) if depth < 6 => {
            let middle = (start+end)*0.5;
            let mut split = solver.clone();
            let left = advance(&mut split, start, middle, depth+1, frame_at)?;
            let right = advance(&mut split, middle, end, depth+1, frame_at)?;
            *solver = split;
            Ok(1+left+right)
        }
        Err(error) => Err(error),
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 6, "asset.json trace.ndjson positions.bin rate_hz start_ms");
    let asset = TissueAsset::from_json(&std::fs::read(&args[1])?)?;
    let samples: Vec<Sample> = std::fs::read_to_string(&args[2])?
        .lines().map(serde_json::from_str).collect::<Result<_, _>>()?;
    anyhow::ensure!(samples.len() > 1 && samples.windows(2).all(|w| w[1].pts_ms > w[0].pts_ms));
    anyhow::ensure!(samples.iter().all(|s| s.regions.len() == asset.regions.len()));
    let frame_at = |region: usize, time: f64| {
        let hi = samples.partition_point(|s| s.pts_ms < time).min(samples.len() - 1);
        let lo = hi.saturating_sub(1);
        if lo == hi { return samples[hi].regions[region].frame(); }
        let alpha = (time - samples[lo].pts_ms) / (samples[hi].pts_ms - samples[lo].pts_ms);
        samples[lo].regions[region].frame().interpolate(samples[hi].regions[region].frame(), alpha)
    };
    let rate: u32 = args[4].parse()?;
    let start: f64 = args[5].parse()?;
    anyhow::ensure!([120, 240, 480].contains(&rate) && start.is_finite()
        && start >= samples[0].pts_ms && start < samples.last().unwrap().pts_ms);
    let dt = 1.0 / f64::from(rate);
    let steps = (((samples.last().unwrap().pts_ms - start) * f64::from(rate) / 1000.0) + 1e-9).floor() as usize;
    let mut solvers = asset.regions.iter().enumerate()
        .map(|(i, r)| r.solver(frame_at(i, start))).collect::<Result<Vec<_>, _>>()?;
    for (i, solver) in solvers.iter_mut().enumerate() {
        solver.initialize_motion(frame_at(i, start + dt * 1000.0), dt)?;
    }
    let mut output = std::io::BufWriter::new(std::fs::File::options().write(true).create_new(true).open(&args[3])?);
    let started = std::time::Instant::now();
    let mut solve_ns = 0_u128;
    let mut subdivisions = 0;
    for tick in 1..=steps {
        let time = start + tick as f64 * dt * 1000.0;
        for (i, solver) in solvers.iter_mut().enumerate() {
            let before = std::time::Instant::now();
            subdivisions += advance(solver, time-dt*1000.0, time, 0, &|at|frame_at(i, at))?;
            solve_ns += before.elapsed().as_nanos();
            for position in solver.positions() {
                for coordinate in position.to_array() { output.write_all(&coordinate.to_le_bytes())?; }
            }
        }
    }
    output.flush()?;
    println!("{}", serde_json::json!({"source_start_ms":start, "steps":steps,
        "rate_hz":rate, "regions":solvers.len(), "subdivisions":subdivisions, "solve_seconds":solve_ns as f64 / 1e9,
        "wall_seconds":started.elapsed().as_secs_f64(),
        "metrics":solvers.iter().map(|s|s.metrics()).collect::<Vec<_>>(),
        "limitations":"Recorded attachment interpolation; no body contact, rendering, or fractional display solve. Profiling workload only, not source-motion acceptance or a real-time benchmark."}));
    soft_tissue::solver::implicit::profile();
    Ok(())
}
