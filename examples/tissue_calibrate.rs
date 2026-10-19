//! Offline material calibration probe; never moves the avatar or edits a GLB.
//! Applies material overrides, refits the unloaded reference so standing
//! gravity still reproduces the authored shape, then measures static
//! orientation sag and a jump-landing ringdown in the anchor (body) frame.
use bevy::math::DVec3;
use cc_render_plugin::cc_render::soft_tissue::{Attachment, Frame, RegionAsset, Tissue, TissueAsset};

const G: f64 = 9.81;
/// Gravity in the body frame: +x her left, +y up, +z forward.
const ORIENTATIONS: [(&str, DVec3); 5] = [
    ("face_down", DVec3::new(0., 0., G)),
    ("face_up", DVec3::new(0., 0., -G)),
    ("upside_down", DVec3::new(0., G, 0.)),
    ("left_side_down", DVec3::new(G, 0., 0.)),
    ("right_side_down", DVec3::new(-G, 0., 0.)),
];

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() >= 2,
        "tissue_calibrate asset.json [material.path=value ...] [--no-fit] [--write=fitted.json]");
    let write = args.iter().find_map(|a| a.strip_prefix("--write=")).map(std::path::PathBuf::from);
    anyhow::ensure!(write.as_ref().is_none_or(|p| !p.exists()), "output exists");
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let fit = !args.iter().any(|a| a == "--no-fit");
    let overrides: Vec<_> = args[2..].iter().filter(|a| !a.starts_with("--")).collect();
    for region in json["regions"].as_array_mut().ok_or_else(|| anyhow::anyhow!("no regions"))? {
        for assignment in &overrides {
            let (path, value) = assignment.split_once('=').ok_or_else(|| anyhow::anyhow!("use key=value"))?;
            let mut slot = &mut region["material"];
            for key in path.split('.') {
                slot = &mut slot[key];
            }
            *slot = serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.into()));
        }
    }
    let asset = TissueAsset::from_json(&serde_json::to_vec(&json)?)?;
    let mut regions = Vec::new();
    let mut fitted = asset.clone();
    for (index, region) in asset.regions.iter().enumerate() {
        let started = std::time::Instant::now();
        let mut region = region.clone();
        let fit_error = if fit { refit(&mut region)? } else { f64::NAN };
        fitted.regions[index] = region.clone();
        let mut standing = region.solver(Frame::IDENTITY)?;
        standing.equilibrate(DVec3::NEG_Y * G, 4096)?;
        let rest = standing.positions().to_vec();
        let free: Vec<_> = (0..rest.len()).filter(|&i| matches!(region.attachments[i], Attachment::Free)).collect();
        let nipple = *free.iter().max_by(|&&a, &&b| rest[a].z.total_cmp(&rest[b].z)).unwrap();
        let centroid = |x: &[DVec3]| free.iter().map(|&i| x[i]).sum::<DVec3>() / free.len() as f64;
        let mut statics = serde_json::Map::new();
        for (name, gravity) in ORIENTATIONS {
            let mut solver = standing.clone();
            let result = solver.equilibrate(gravity, 4096);
            let x = solver.positions();
            statics.insert(name.into(), match result {
                Ok(_) => serde_json::json!({
                    "centroid_mm": mm(centroid(x) - centroid(&rest)),
                    "nipple_mm": mm(x[nipple] - rest[nipple]),
                    "volume_ratio": [solver.metrics().minimum_volume_ratio, solver.metrics().maximum_volume_ratio]}),
                Err(error) => serde_json::json!({"error": error.to_string()}),
            });
        }
        regions.push(serde_json::json!({
            "name": region.name, "free_nodes": free.len(), "mass_kg": standing.metrics().mass_kg,
            "fit_error_mm": fit_error * 1e3, "static": statics,
            "landing": landing(&standing, &free, nipple)?,
            "shake_side": shake(&standing, DVec3::X)?, "shake_bounce": shake(&standing, DVec3::Y)?,
            "seconds": started.elapsed().as_secs_f64()}));
    }
    if let Some(path) = write {
        fitted.validate()?;
        std::fs::write(path, serde_json::to_vec(&fitted)?)?;
    }
    println!("{}", serde_json::json!({"overrides": overrides, "fit": fit,
        "material": json["regions"][0]["material"], "regions": regions}));
    Ok(())
}

/// The orientation rig's shakes: 3 cm at 3 Hz for 3 s with 0.5 s ramps.
fn shake(standing: &Tissue, axis: DVec3) -> anyhow::Result<serde_json::Value> {
    let rate = 240.;
    let mut solver = standing.clone();
    let (mut volume, mut offset): ((f64, f64), f64) = ((1., 1.), 0.);
    for step in 1..=(4.0 * rate) as usize {
        let t = step as f64 / rate;
        let edge = (t.min(3. - t).min(0.5) / 0.5).max(0.);
        let wave = 0.03 * (0.5 - 0.5 * (std::f64::consts::PI * edge).cos()) * (2. * std::f64::consts::PI * 3. * t).sin();
        let frame = Frame { translation: axis * if t < 3. { wave } else { 0. }, rotation: bevy::math::DQuat::IDENTITY };
        solver.step(1. / rate, frame, DVec3::NEG_Y * G)?;
        let metrics = solver.metrics();
        volume = (volume.0.min(metrics.minimum_volume_ratio), volume.1.max(metrics.maximum_volume_ratio));
        offset = solver.positions().iter().zip(standing.positions())
            .map(|(p, rest)| (*p - frame.translation - *rest).length()).fold(offset, f64::max);
    }
    Ok(serde_json::json!({"volume_ratio": [volume.0, volume.1], "peak_node_mm": offset * 1e3}))
}

fn mm(v: DVec3) -> [f64; 3] {
    (v * 1e3).to_array().map(|c| (c * 10.).round() / 10.)
}

/// The tissue_fit loop: move the free unloaded reference until standing
/// equilibrium reproduces the authored shape within 0.25 mm.
fn refit(region: &mut RegionAsset) -> anyhow::Result<f64> {
    for _ in 0..40 {
        let mut solver = region.solver(Frame::IDENTITY)?;
        solver.equilibrate(DVec3::NEG_Y * G, 4096)?;
        let errors: Vec<_> = region.initial_positions.iter().zip(solver.positions())
            .map(|(target, actual)| DVec3::from(*target) - *actual).collect();
        let maximum = errors.iter().map(|d| d.length()).fold(0.0, f64::max);
        if maximum < 0.00025 {
            return Ok(maximum);
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
    anyhow::bail!("{} reference fit did not converge", region.name)
}

/// A 4 cm minimum-jerk drop over 0.15 s (a jump landing), then 2 s held still.
/// Ringdown of the nipple's vertical offset from standing gives the frequency
/// and, from successive peaks, the damping ratio.
fn landing(standing: &Tissue, free: &[usize], nipple: usize) -> anyhow::Result<serde_json::Value> {
    let rate: f64 = std::env::var("CAL_RATE").map_or(Ok(240.), |r| r.parse())?;
    let (drop, over) = (0.04, 0.15);
    let mut solver = standing.clone();
    let start = standing.positions()[nipple];
    let mut trace = Vec::new();
    let mut centroid_trace = Vec::new();
    let mut peak_speed: f64 = 0.;
    for step in 1..=((over + 2.0) * rate) as usize {
        let t = step as f64 / rate;
        let s = (t / over).min(1.);
        let frame = Frame { translation: DVec3::NEG_Y * drop * s * s * s * (10. - 15. * s + 6. * s * s),
            rotation: bevy::math::DQuat::IDENTITY };
        solver.step(1. / rate, frame, DVec3::NEG_Y * G)?;
        trace.push((t, (solver.positions()[nipple] - frame.translation - start).y));
        let centroid = free.iter().map(|&i| solver.positions()[i] - standing.positions()[i]).sum::<DVec3>() / free.len() as f64;
        centroid_trace.push(((centroid - frame.translation).y * 1e4).round() / 10.);
        peak_speed = peak_speed.max(free.iter().map(|&i| solver.velocities()[i].length()).fold(0., f64::max));
    }
    let after: Vec<_> = trace.iter().filter(|(t, _)| *t >= over).copied().collect();
    let peaks: Vec<(f64, f64)> = after.windows(3).filter(|w| w[1].1.abs() >= w[0].1.abs() && w[1].1.abs() > w[2].1.abs()
        && w[1].1.signum() == w[0].1.signum()).map(|w| w[1]).filter(|p| p.1.abs() > 1e-5).collect();
    let lows: Vec<_> = peaks.iter().filter(|p| p.1 < 0.).collect();
    let period = (lows.len() >= 2).then(|| (lows[lows.len().min(4) - 1].0 - lows[0].0) / (lows.len().min(4) - 1) as f64);
    let damping = (lows.len() >= 2).then(|| {
        let delta = (lows[0].1 / lows[1].1).ln();
        delta / (4. * std::f64::consts::PI * std::f64::consts::PI + delta * delta).sqrt()
    });
    Ok(serde_json::json!({
        "nipple_lowest_mm": trace.iter().map(|p| p.1).fold(0., f64::min) * 1e3,
        "nipple_highest_mm": trace.iter().map(|p| p.1).fold(0., f64::max) * 1e3,
        "frequency_hz": period.map(|p| 1. / p), "damping_ratio": damping,
        "residual_after_1s_mm": after.iter().filter(|(t, _)| *t >= over + 1.).map(|p| p.1.abs()).fold(0., f64::max) * 1e3,
        "peak_speed_m_s": peak_speed,
        "nipple_trace_mm": trace.iter().map(|p| (p.1 * 1e4).round() / 10.).collect::<Vec<_>>(),
        "centroid_trace_mm": centroid_trace}))
}
