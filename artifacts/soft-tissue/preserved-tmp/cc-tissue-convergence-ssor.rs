#[path="/tmp/cc-tissue-convergence-ssor/mod.rs"] mod soft_tissue;
// Numerical indentation fixture
use bevy::math::DVec3;
use soft_tissue::{
    EmbeddedPoint, Frame, PlaneContact, TissueAsset, VertexBinding,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Sample {
    binding: VertexBinding,
    rest_point: [f64; 3],
    area_m2: f64,
}

#[derive(Deserialize)]
struct Samples {
    tissue_asset_sha256: String,
    source_glb_sha256: String,
    samples: Vec<Sample>,
}

fn indentation(seconds: f64) -> f64 {
    if seconds < 0.5 {
        0.0
    } else if seconds < 1.0 {
        (seconds - 0.5) * 2.0
    } else if seconds < 1.5 {
        1.0
    } else {
        ((2.0 - seconds) * 2.0).max(0.0)
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        (5..=6).contains(&args.len()),
        "tissue_contact_probe asset.json samples.json iterations rate_hz [pressure_stiffness_pa_per_m=1e8]"
    );
    let bytes = std::fs::read(&args[1])?;
    let asset = TissueAsset::from_json(&bytes)?;
    let samples: Samples = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let digest: String = hmac_sha256::Hash::hash(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    anyhow::ensure!(
        digest == samples.tissue_asset_sha256
            && asset.source_sha256.contains(&samples.source_glb_sha256),
        "contact samples belong to a different asset"
    );
    let iterations: usize = args[3].parse()?;
    let rate: usize = args[4].parse()?;
    let pressure_stiffness = args.get(5).map_or(Ok(1e8_f64), |s| s.parse())?;
    anyhow::ensure!(pressure_stiffness.is_finite() && pressure_stiffness > 0.0);
    anyhow::ensure!((1..=64).contains(&iterations) && (60..=480).contains(&rate));
    for (index, region) in asset.regions.iter().enumerate() {
        let mut region = region.clone();
        region.material.iterations = iterations;
        let mut tissue = region.solver(Frame::IDENTITY)?;
        let skin: Vec<_> = samples
            .samples
            .iter()
            .filter(|s| s.binding.region == index)
            .collect();
        anyhow::ensure!(!skin.is_empty());
        let start_z = skin
            .iter()
            .map(|s| s.rest_point[2])
            .fold(f64::NEG_INFINITY, f64::max)
            + 0.003;
        let mut contacts = Vec::new();
        for sample in skin {
            anyhow::ensure!(sample.area_m2.is_finite() && sample.area_m2 > 0.0);
            let binding = &sample.binding;
            let embedded: DVec3 = region.tetrahedra[binding.tetrahedron]
                .iter()
                .zip(binding.barycentric)
                .map(|(i, w)| tissue.positions()[*i] * (binding.influence * w))
                .sum();
            contacts.push(PlaneContact {
                point: EmbeddedPoint {
                    tetrahedron: binding.tetrahedron,
                    barycentric: binding.barycentric,
                    influence: binding.influence,
                    offset: DVec3::from(sample.rest_point) - embedded,
                },
                surface_point: DVec3::Z * start_z,
                normal: DVec3::NEG_Z,
                stiffness_n_per_m: pressure_stiffness * sample.area_m2,
            });
        }
        let start = std::time::Instant::now();
        for tick in 1..=3 * rate {
            let seconds = tick as f64 / rate as f64;
            let plate_z = start_z - 0.015 * indentation(seconds);
            for contact in &mut contacts {
                contact.surface_point.z = plate_z;
            }
            let contact = tissue.step_with_contacts(
                1.0 / rate as f64,
                Frame::IDENTITY,
                DVec3::NEG_Y * 9.81,
                &contacts,
            )?;
            println!(
                "{}",
                serde_json::json!({"region":region.name, "seconds":seconds, "iterations":iterations,
                "rate_hz":rate, "plate_z_m":plate_z, "contact":contact, "tissue":tissue.metrics(),
                "pressure_stiffness_pa_per_m":pressure_stiffness,
                "node_positions":if tick == 3*rate/2 || tick == 3*rate {
                    Some(tissue.positions().iter().map(|p| p.to_array()).collect::<Vec<_>>())
                } else { None },
                "wall_seconds":start.elapsed().as_secs_f64()})
            );
        }
    }
    soft_tissue::solver::implicit::profile();
    Ok(())
}
