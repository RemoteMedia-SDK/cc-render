//! Release-mode cost of the live batch path on authored condo cloth.
//! Run: `cargo test --release --lib bench_authored_cloth_ticks -- --ignored --nocapture`.
//! CC_CLOTH_BENCH_SCENE (default scene-bedding.json), CC_CLOTH_BENCH_TICKS (60).
use super::*;
use crate::cc_render::{cloth_profile, scene_package::ScenePackage};

/// Sorted layer ids of the same selection `authored_layers` returns.
fn authored_ids(scene: &str) -> Vec<String> {
    let package = ScenePackage::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo")
            .join(scene),
    )
    .unwrap();
    let mut ids: Vec<_> = package
        .cloths
        .iter()
        .map(|c| c.id.clone())
        .filter(|id| {
            std::env::var("CC_CLOTH_BENCH_LAYERS")
                .map(|v| v.split(',').any(|x| x == id))
                .unwrap_or(true)
        })
        .collect();
    ids.sort();
    ids
}

pub(crate) fn authored_layers(scene: &str) -> (Vec<ClothSolver>, Vec<Vec<FixedClothCollider>>) {
    let package = ScenePackage::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo")
            .join(scene),
    )
    .unwrap();
    let mut package = package;
    if let Ok(drag) = std::env::var("CC_CLOTH_BENCH_DRAG") {
        for cloth in &mut package.cloths {
            cloth.parameters.air_drag_per_second = drag.parse().unwrap();
        }
    }
    if let Ok(zeta) = std::env::var("CC_CLOTH_BENCH_ZETA") {
        for cloth in &mut package.cloths {
            cloth.parameters.spring_damping_ratio = zeta.parse().unwrap();
        }
    }
    if let Ok(substeps) = std::env::var("CC_CLOTH_BENCH_SUBSTEPS") {
        for cloth in &mut package.cloths {
            cloth.parameters.substeps = substeps.parse().unwrap();
        }
    }
    let mut definitions: Vec<_> = package
        .cloths
        .iter()
        .filter(|c| {
            std::env::var("CC_CLOTH_BENCH_LAYERS")
                .map(|v| v.split(',').any(|id| id == c.id))
                .unwrap_or(true)
        })
        .collect();
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
    // Rendered-surface samples every 6 ticks (10 fps) for offline replay.
    let mut frames = std::env::var("CC_CLOTH_BENCH_FRAMES").ok().map(|path| {
        std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap(),
        )
    });
    let ids = authored_ids(&scene);
    let mut write_frame = |tick: usize, solvers: &[ClothSolver], resting: bool| {
        if let Some(file) = frames.as_mut() {
            use std::io::Write;
            let layers: Vec<_> = ids
                .iter()
                .zip(solvers)
                .map(|(id, s)| {
                    serde_json::json!({"id": id, "positions": s.surface.positions().iter()
                        .map(|p| p.to_array()).collect::<Vec<_>>()})
                })
                .collect();
            writeln!(
                file,
                "{}",
                serde_json::json!({"frame": tick, "resting": resting, "layers": layers})
            )
            .unwrap();
        }
    };
    write_frame(0, &solvers, false);
    let mut batch = ClothBatch::new(&solvers.iter().collect::<Vec<_>>()).unwrap();
    let gravity = bevy_rapier3d::prelude::RapierConfiguration::new(1.)
        .gravity
        .as_dvec3();
    cloth_profile::enable();
    let started = std::time::Instant::now();
    let mut window = std::time::Instant::now();
    let mut window_speed = 0f64;
    let mut history: std::collections::VecDeque<Vec<DVec3>> = Default::default();
    let mut window_drift = 0f64;
    let mut window_stretch = 0f64;
    let mut window_tick_speed = 0f64;
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
        if tick % 6 == 0 {
            write_frame(tick, &solvers, batch.resting());
        }
        window_speed = solvers
            .iter()
            .map(ClothSolver::max_speed)
            .fold(window_speed, f64::max);
        window_stretch = solvers
            .iter()
            .map(ClothSolver::max_edge_stretch_ratio)
            .fold(window_stretch, f64::max);
        let now: Vec<_> = solvers
            .iter()
            .flat_map(|s| s.positions.iter().copied())
            .collect();
        if let (Some(last), Ok(_)) = (history.back(), std::env::var("CC_CLOTH_TICK_JITTER")) {
            let mut moves: Vec<_> = last
                .iter()
                .zip(&now)
                .enumerate()
                .map(|(i, (a, b))| (a.distance(*b) * 60., i, *b))
                .collect();
            moves.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (speed, i, p) in moves.iter().take(5) {
                println!(
                    "TICK_JITTER {tick} vertex={i} tick_speed={speed:.4} at={:?}",
                    p.to_array().map(|x| (x * 1e4).round() / 1e4)
                );
            }
        }
        if let Some(last) = history.back() {
            window_tick_speed = last
                .iter()
                .zip(&now)
                .map(|(a, b)| a.distance(*b) * 60.)
                .fold(window_tick_speed, f64::max);
        }
        if history.len() == 30 {
            let old = history.pop_front().unwrap();
            window_drift = old
                .iter()
                .zip(&now)
                .map(|(a, b)| a.distance(*b))
                .fold(window_drift, f64::max);
        }
        history.push_back(now);
        if let Ok(track) = std::env::var("CC_CLOTH_TRACK") {
            let all: Vec<_> = solvers
                .iter()
                .flat_map(|s| s.positions.iter().zip(&s.velocities))
                .collect();
            let tracked: Vec<_> = track
                .split(',')
                .map(|i| i.parse::<usize>().unwrap())
                .map(|i| {
                    let (p, v) = all[i];
                    serde_json::json!({"vertex": i, "p": p.to_array(), "v": v.to_array()})
                })
                .collect();
            println!("CLOTH_TRACK {tick} {}", serde_json::json!(tracked));
        }
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
                    "window_max_speed": window_speed,
                    "window_max_stretch": window_stretch,
                    "window_max_tick_speed": window_tick_speed,
                    "window_max_drift_over_30_ticks": window_drift,
                    "resting": batch.resting(),
                    "phases_ms": phases.iter().map(|(k, v)| (k.to_string(), serde_json::json!({
                        "ms": v.seconds * 1e3, "calls": v.calls, "items": v.items
                    }))).collect::<serde_json::Map<_, _>>(),
                })
            );
            window_speed = 0.;
            window_drift = 0.;
            window_stretch = 0.;
            window_tick_speed = 0.;
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

#[test]
#[ignore = "diagnostic: where does end-of-tick velocity come from in a loaded state"]
fn diagnose_authored_cloth_velocity_sources() {
    let scene =
        std::env::var("CC_CLOTH_BENCH_SCENE").unwrap_or_else(|_| "scene-bedding.json".into());
    let warm: usize = std::env::var("CC_CLOTH_BENCH_TICKS")
        .unwrap_or_else(|_| "0".into())
        .parse()
        .unwrap();
    let (mut solvers, colliders) = authored_layers(&scene);
    if let Ok(path) = std::env::var("CC_CLOTH_BENCH_LOAD") {
        load_state(&mut solvers, &path);
    }
    let ranges: Vec<_> = solvers
        .iter()
        .scan(0, |start, s| {
            let r = *start..*start + s.positions.len();
            *start = r.end;
            Some(r)
        })
        .collect();
    let mut batch = ClothBatch::new(&solvers.iter().collect::<Vec<_>>()).unwrap();
    let gravity = bevy_rapier3d::prelude::RapierConfiguration::new(1.)
        .gravity
        .as_dvec3();
    let substeps = solvers.iter().map(|s| s.parameters.substeps).max().unwrap();
    let h = 1. / 60. / substeps as f64;
    for tick in 0..=warm {
        cloth_profile::trace_stages(tick == warm);
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
    }
    let trace = cloth_profile::take_stages();
    cloth_profile::trace_stages(false);
    // Group snapshots per substep: start, predicted, constraints, fixed_first, contacts.
    let names = [
        "start",
        "predicted",
        "constraints",
        "fixed_first",
        "contacts",
    ];
    let substep_stages: Vec<Vec<&Vec<DVec3>>> = trace
        .chunks(names.len())
        .map(|chunk| {
            assert!(
                chunk.iter().zip(names).all(|(c, n)| c.0 == n),
                "stage order"
            );
            chunk.iter().map(|c| &c.2).collect()
        })
        .collect();
    assert_eq!(substep_stages.len(), substeps);
    let layer_of = |i: usize| ranges.iter().position(|r| r.contains(&i)).unwrap();
    let support = |i: usize, p: DVec3| {
        colliders[layer_of(i)]
            .iter()
            .map(|c| c.distance_and_normal(p).0)
            .fold(f64::INFINITY, f64::min)
    };
    // Per stage, per layer: mean |displacement|/h over all substeps and vertices.
    let mut stage_speed = vec![vec![0f64; ranges.len()]; names.len() - 1];
    // Velocity at the end of each substep, and how much each stage contributed
    // along it (projection of stage displacement on the final velocity).
    let mut along = vec![vec![0f64; ranges.len()]; names.len() - 1];
    for stages in &substep_stages {
        let n = stages[0].len();
        for i in 0..n {
            let v = (stages[4][i] - stages[0][i]) / h;
            let dir = v.normalize_or_zero();
            for k in 0..names.len() - 1 {
                let d = (stages[k + 1][i] - stages[k][i]) / h;
                stage_speed[k][layer_of(i)] +=
                    d.length() / (substeps * ranges[layer_of(i)].len()) as f64;
                along[k][layer_of(i)] += d.dot(dir) / (substeps * ranges[layer_of(i)].len()) as f64;
            }
        }
    }
    // Exact sequential kinetic-energy change per stage, by layer and by
    // whether the vertex ends the tick within 3 mm of a fixed collider.
    let inverse_mass: Vec<f64> = solvers
        .iter()
        .flat_map(|s| s.inverse_mass.iter().copied())
        .collect();
    let end = &substep_stages.last().unwrap()[4];
    let near: Vec<bool> = (0..end.len()).map(|i| support(i, end[i]) < 0.003).collect();
    let mut work = vec![[[0f64; 2]; 4]; ranges.len()];
    let mut vertex_work = vec![[0f64; 4]; end.len()];
    let mut velocity: Vec<DVec3> = Vec::new();
    for stages in &substep_stages {
        if velocity.is_empty() {
            // Unknown entry velocity for the first traced substep: skip it.
            velocity = (0..stages[0].len())
                .map(|i| (stages[4][i] - stages[0][i]) / h)
                .collect();
            continue;
        }
        for i in 0..stages[0].len() {
            let m = 1. / inverse_mass[i];
            let mut v = velocity[i];
            // Integration (gravity + drag): whatever the predicted step implies.
            let after = (stages[1][i] - stages[0][i]) / h;
            let mut deltas = [after - v, DVec3::ZERO, DVec3::ZERO, DVec3::ZERO];
            for k in 1..4 {
                deltas[k] = (stages[k + 1][i] - stages[k][i]) / h;
            }
            for (k, d) in deltas.into_iter().enumerate() {
                let next = v + d;
                let gain = 0.5 * m * (next.length_squared() - v.length_squared());
                work[layer_of(i)][k][usize::from(near[i])] += gain;
                vertex_work[i][k] += gain;
                v = next;
            }
            velocity[i] = v;
        }
    }
    // Full mechanical energy (kinetic from the substep's candidate velocity,
    // gravity, stretch and bend springs) after every stage, per layer.
    let energy = |positions: &[DVec3], velocity: &dyn Fn(usize) -> DVec3, layer: usize| {
        let solver = &solvers[layer];
        let r = ranges[layer].clone();
        let local = &positions[r.clone()];
        let kinetic: f64 = (0..local.len())
            .map(|i| 0.5 * velocity(r.start + i).length_squared() / solver.inverse_mass[i])
            .sum();
        let potential: f64 = local
            .iter()
            .zip(&solver.inverse_mass)
            .map(|(p, w)| -gravity.dot(*p) / w)
            .sum();
        let springs: f64 = solver
            .springs
            .iter()
            .map(|sp| {
                let x = local[sp.vertices[0]].distance(local[sp.vertices[1]]) - sp.rest;
                0.5 * x * x / sp.compliance
            })
            .sum();
        kinetic + potential + springs
    };
    let mut stage_energy = vec![[0f64; 4]; ranges.len()];
    let mut entry: Vec<DVec3> = Vec::new();
    for stages in &substep_stages {
        if entry.is_empty() {
            entry = (0..stages[0].len())
                .map(|i| (stages[4][i] - stages[0][i]) / h)
                .collect();
            continue;
        }
        for layer in 0..ranges.len() {
            let mut before = energy(stages[0], &|i| entry[i], layer);
            for k in 1..5 {
                let after = energy(stages[k], &|i| (stages[k][i] - stages[0][i]) / h, layer);
                stage_energy[layer][k - 1] += after - before;
                before = after;
            }
        }
        entry = (0..stages[0].len())
            .map(|i| (stages[4][i] - stages[0][i]) / h)
            .collect();
    }
    // Largest single-stage velocity change per layer, with where it happened.
    for layer in 0..ranges.len() {
        for k in 1..4 {
            let mut best = (0f64, 0usize, 0usize);
            for (substep, stages) in substep_stages.iter().enumerate() {
                for i in ranges[layer].clone() {
                    let dv = ((stages[k + 1][i] - stages[k][i]) / h).length();
                    if dv > best.0 {
                        best = (dv, i, substep);
                    }
                }
            }
            let (dv, i, substep) = best;
            let st = &substep_stages[substep];
            println!(
                "STAGE_MAX_DV {}",
                serde_json::json!({"layer": layer, "stage": names[k + 1], "dv": dv, "vertex": i,
                    "local": i - ranges[layer].start, "substep": substep,
                    "position": st[k + 1][i].to_array(), "support": support(i, st[k + 1][i]),
                    "delta": ((st[k + 1][i] - st[k][i]) / h).to_array()})
            );
        }
    }
    println!(
        "STAGE_ENERGY_CHANGE_J {}",
        serde_json::json!({
            "stages": ["integrate", "materials", "fixed_first", "self_and_repeat"],
            "by_layer": stage_energy,
        })
    );
    println!(
        "STAGE_KINETIC_WORK_J {}",
        serde_json::json!({
            "stages": ["integrate", "materials", "fixed_first", "self_and_repeat"],
            "by_layer_stage_[free,near_fixed]": work,
        })
    );
    for k in 0..4 {
        let mut order: Vec<_> = (0..end.len()).collect();
        order.sort_by(|a, b| {
            vertex_work[*b][k]
                .abs()
                .total_cmp(&vertex_work[*a][k].abs())
        });
        for &i in order.iter().take(25) {
            println!(
                "VERTEX_WORK {}",
                serde_json::json!({"stage": k, "vertex": i, "layer": layer_of(i), "work": vertex_work[i],
                    "position": end[i].to_array(), "support": support(i, end[i])})
            );
        }
    }
    println!(
        "VELOCITY_SOURCES {}",
        serde_json::json!({
            "stages": names[1..],
            "mean_stage_speed_by_layer": stage_speed,
            "mean_along_final_velocity_by_layer": along,
        })
    );
    if let Ok(watch) = std::env::var("CC_CLOTH_WATCH") {
        for i in watch.split(',').map(|v| v.parse::<usize>().unwrap()) {
            for (k, st) in substep_stages.iter().enumerate() {
                let deltas: Vec<_> = (0..names.len() - 1)
                    .map(|s| {
                        ((st[s + 1][i] - st[s][i]) * 1e6)
                            .to_array()
                            .map(|x| x.round())
                    })
                    .collect();
                println!(
                    "WATCH {i} substep {k} end_um {:?} stages_um {:?}",
                    ((st[4][i] - st[0][i]) * 1e6).to_array().map(|x| x.round()),
                    deltas
                );
            }
        }
    }
    let last = substep_stages.last().unwrap();
    let mut fastest: Vec<_> = (0..last[0].len())
        .map(|i| (i, ((last[4][i] - last[0][i]) / h).length()))
        .collect();
    fastest.sort_by(|a, b| b.1.total_cmp(&a.1));
    for &(i, speed) in fastest.iter().take(12) {
        // Stage history of this vertex across every substep of the tick.
        let history: Vec<_> = substep_stages
            .iter()
            .map(|st| {
                (0..names.len() - 1)
                    .map(|k| {
                        ((st[k + 1][i] - st[k][i]) / h)
                            .to_array()
                            .map(|x| (x * 1e3).round() / 1e3)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        println!(
            "FAST_VERTEX {}",
            serde_json::json!({
                "vertex": i, "layer": layer_of(i), "local": i - ranges[layer_of(i)].start,
                "speed": speed, "position": last[4][i].to_array(),
                "support_distance": support(i, last[4][i]),
                "last_substeps": history[history.len().saturating_sub(4)..],
            })
        );
    }
}

#[test]
#[ignore = "benchmark: cost of the bedding test's exact inter-layer distance check"]
fn bench_interlayer_distance_queries() {
    use parry_ccd::{math::Isometry, query::distance, shape::TriMesh};
    let (mut solvers, _) = authored_layers("scene-bedding.json");
    if let Ok(path) = std::env::var("CC_CLOTH_BENCH_LOAD") {
        load_state(&mut solvers, &path);
    }
    let meshes: Vec<_> = solvers
        .iter()
        .map(|s| {
            TriMesh::new(
                s.surface
                    .positions()
                    .iter()
                    .map(|p| parry_ccd::math::Point::new(p.x as f64, p.y as f64, p.z as f64))
                    .collect(),
                s.triangles.iter().map(|t| t.map(|i| i as u32)).collect(),
            )
        })
        .collect();
    for a in 0..meshes.len() {
        for b in a + 1..meshes.len() {
            let started = std::time::Instant::now();
            let gap = distance(
                &Isometry::identity(),
                &meshes[a],
                &Isometry::identity(),
                &meshes[b],
            )
            .unwrap();
            println!(
                "INTERLAYER_DISTANCE {a}-{b} gap={gap:.9} ms={:.1}",
                started.elapsed().as_secs_f64() * 1e3
            );
        }
    }
}

/// Bake a settled drape: simulate the authored cloth of a scene from its
/// authored start (same batch, gravity and tick as live) until the batch
/// falls asleep, then write each layer's surface asset with the settled
/// positions as `initial_positions`. Rest positions, topology and UVs are
/// copied unchanged. CC_CLOTH_BAKE_SCENE, CC_CLOTH_BAKE_OUT (directory).
#[test]
#[ignore = "tool: bakes settled cloth assets for live scenes"]
fn bake_settled_drape() {
    let scene = std::env::var("CC_CLOTH_BAKE_SCENE").expect("CC_CLOTH_BAKE_SCENE");
    let out =
        std::path::PathBuf::from(std::env::var("CC_CLOTH_BAKE_OUT").expect("CC_CLOTH_BAKE_OUT"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/toronto_condo");
    let package = ScenePackage::load(&root.join(&scene)).unwrap();
    let mut definitions: Vec<_> = package.cloths.iter().collect();
    definitions.sort_by(|a, b| a.id.cmp(&b.id));
    let mut solvers: Vec<_> = definitions.iter().map(|d| d.solver().unwrap()).collect();
    let colliders: Vec<_> = definitions
        .iter()
        .map(|d| d.colliders(&package).unwrap())
        .collect();
    let mut batch = super::super::gpu::AnyBatch::from_env(&solvers.iter().collect::<Vec<_>>(), "CC_CLOTH_BAKE_GPU");
    let gravity = bevy_rapier3d::prelude::RapierConfiguration::new(1.)
        .gravity
        .as_dvec3();
    let mut tick = 0;
    while !batch.resting() {
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
        tick += 1;
        assert!(tick <= 3600, "drape did not come to rest within 60 s");
    }
    std::fs::create_dir_all(&out).unwrap();
    for (definition, solver) in definitions.iter().zip(&solvers) {
        let source = root.join(&definition.surface);
        let mut asset = (**definition.loaded.as_ref().unwrap()).clone();
        asset.initial_positions = solver
            .surface
            .positions()
            .iter()
            .map(|p| p.to_array())
            .collect();
        let bytes = serde_json::to_vec(&asset).unwrap();
        let name = source.file_stem().unwrap().to_string_lossy().to_string() + "-settled.json";
        std::fs::write(out.join(&name), &bytes).unwrap();
        println!(
            "BAKED_DRAPE {}",
            serde_json::json!({
                "scene": scene, "id": definition.id, "file": name, "rest_tick": tick,
                "sha256": hmac_sha256::Hash::hash(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            })
        );
    }
}

/// A baked drape (`starts_at_rest`) must really rest under its own authored
/// parameters, not only because the live batch adopts it as asleep. Woken
/// without that adoption, every layer must fall asleep after the first quiet
/// window (0.5 s, limit 1 s) and no particle may drift past the sleep bound.
fn baked_drape_rests_when_simulated(scene: &str) {
    let (mut solvers, colliders) = authored_layers(scene);
    let start: Vec<Vec<DVec3>> = solvers.iter().map(|s| s.positions.clone()).collect();
    let mut batch = super::super::gpu::AnyBatch::from_env(&solvers.iter().collect::<Vec<_>>(), "CC_CLOTH_BAKE_GPU");
    let gravity = bevy_rapier3d::prelude::RapierConfiguration::new(1.)
        .gravity
        .as_dvec3();
    let mut tick = 0;
    while !batch.resting() {
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
        tick += 1;
        assert!(tick <= 60, "{scene}: baked drape still moving after 1 s");
    }
    let drift = solvers
        .iter()
        .zip(&start)
        .flat_map(|(s, a)| s.positions.iter().zip(a).map(|(p, q)| p.distance(*q)))
        .fold(0., f64::max);
    println!("BAKED_REST {}", serde_json::json!({"scene": scene, "rest_tick": tick, "max_drift_m": drift}));
    assert!(drift <= REST_DRIFT_M, "{scene}: baked drape drifted {drift} m");
}

#[test]
#[ignore = "release: the moving bedding costs about a second per tick"]
fn baked_bedding_rests_when_simulated() {
    baked_drape_rests_when_simulated("scene-bedding-settled.json");
}

/// The two-cover bed baked on the GPU (CC_CLOTH_BAKE_GPU=1 to check it there).
#[test]
#[ignore = "release: the moving bedding costs about a second per tick on the CPU"]
fn baked_two_covers_rest_when_simulated() {
    baked_drape_rests_when_simulated("scene-bedding-two-covers-settled.json");
}

#[test]
fn baked_vanity_sheet_rests_when_simulated() {
    baked_drape_rests_when_simulated("scene-vanity-drawers.json");
}
