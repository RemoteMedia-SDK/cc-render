//! Release-mode cost of the live batch path on authored condo cloth.
//! Run: `cargo test --release --lib bench_authored_cloth_ticks -- --ignored --nocapture`.
//! CC_CLOTH_BENCH_SCENE (default scene-bedding.json), CC_CLOTH_BENCH_TICKS (60).
use super::*;
use crate::cc_render::{cloth_profile, scene_package::ScenePackage};

pub(crate) fn authored_layers(scene: &str) -> (Vec<ClothSolver>, Vec<Vec<FixedClothCollider>>) {
    let package = ScenePackage::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo")
            .join(scene),
    )
    .unwrap();
    let mut definitions: Vec<_> = package.cloths.iter().collect();
    definitions.sort_by(|a, b| a.id.cmp(&b.id));
    (
        definitions.iter().map(|d| d.solver().unwrap()).collect(),
        definitions
            .iter()
            .map(|d| d.colliders(&package).unwrap())
            .collect(),
    )
}

#[test]
#[ignore = "benchmark: run in release with --nocapture"]
fn bench_authored_cloth_ticks() {
    let scene =
        std::env::var("CC_CLOTH_BENCH_SCENE").unwrap_or_else(|_| "scene-bedding.json".into());
    let ticks: usize = std::env::var("CC_CLOTH_BENCH_TICKS")
        .unwrap_or_else(|_| "60".into())
        .parse()
        .unwrap();
    let (mut solvers, colliders) = authored_layers(&scene);
    if let Ok(path) = std::env::var("CC_CLOTH_BENCH_LOAD") {
        load_state(&mut solvers, &path);
    }
    let save = std::env::var("CC_CLOTH_BENCH_SAVE").ok();
    let mut batch = ClothBatch::new(&solvers.iter().collect::<Vec<_>>()).unwrap();
    let gravity = DVec3::new(0., -9.81, 0.);
    cloth_profile::enable();
    let started = std::time::Instant::now();
    let mut window = std::time::Instant::now();
    for tick in 1..=ticks {
        let mut layers: Vec<_> = solvers
            .iter_mut()
            .zip(&colliders)
            .map(|(solver, colliders)| ClothLayerStep {
                solver,
                colliders,
                grips: &mut [],
            })
            .collect();
        batch.step(&mut layers, 1. / 60., gravity).unwrap();
        if let Some(path) = &save {
            if tick % 100 == 0 || tick == ticks {
                save_state(&solvers, &format!("{path}.{tick}.json"));
            }
        }
        if tick % 10 == 0 || tick == ticks {
            let phases = cloth_profile::take();
            let span = window.elapsed().as_secs_f64();
            window = std::time::Instant::now();
            println!(
                "CLOTH_BENCH {}",
                serde_json::json!({
                    "scene": scene,
                    "tick": tick,
                    "window_ticks": if tick % 10 == 0 { 10 } else { tick % 10 },
                    "window_ms_per_tick": span * 1e3 / (if tick % 10 == 0 { 10 } else { tick % 10 }) as f64,
                    "wall_seconds": started.elapsed().as_secs_f64(),
                    "max_speeds": solvers.iter().map(ClothSolver::max_speed).collect::<Vec<_>>(),
                    "phases_ms": phases.iter().map(|(k, v)| (k.to_string(), serde_json::json!({
                        "ms": v.seconds * 1e3, "calls": v.calls, "items": v.items
                    }))).collect::<serde_json::Map<_, _>>(),
                })
            );
        }
    }
    let bytes: Vec<u8> = solvers
        .iter()
        .flat_map(|s| {
            s.positions
                .iter()
                .chain(&s.velocities)
                .flat_map(|p| p.to_array())
        })
        .flat_map(f64::to_le_bytes)
        .collect();
    println!(
        "CLOTH_BENCH_FINAL_STATE_SHA256 {}",
        hmac_sha256::Hash::hash(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
}

/// Positions and velocities of every layer, for resuming a bench mid-drape.
pub(crate) fn save_state(solvers: &[ClothSolver], path: &str) {
    let state: Vec<_> = solvers
        .iter()
        .map(|s| {
            serde_json::json!({
                "positions": s.positions.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
                "velocities": s.velocities.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
            })
        })
        .collect();
    std::fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
}

pub(crate) fn load_state(solvers: &mut [ClothSolver], path: &str) {
    let state: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(state.len(), solvers.len());
    for (solver, layer) in solvers.iter_mut().zip(state) {
        let read = |key: &str| -> Vec<DVec3> {
            serde_json::from_value::<Vec<[f64; 3]>>(layer[key].clone())
                .unwrap()
                .into_iter()
                .map(DVec3::from_array)
                .collect()
        };
        solver.positions = read("positions");
        solver.velocities = read("velocities");
        solver
            .surface
            .update_positions(solver.positions.iter().map(|p| p.as_vec3()).collect())
            .unwrap();
    }
}
