//! GPU batch against the CPU reference on shared fixtures, plus kinematic body colliders.
//! Every test prints GPU_SKIPPED and passes trivially when no suitable adapter exists.
use super::*;
use crate::cc_render::cloth_grip::{ClampMaterial, ClothGrip};
use bevy_rapier3d::rapier::parry::{math::Isometry, shape::{Ball, SharedShape}};

pub(crate) fn sheet_params(substeps: usize) -> ClothParameters {
    ClothParameters {
        total_mass_kg: 0.3,
        thickness_m: 0.002,
        stretch_compliance: 1e-7,
        bend_compliance: 0.05,
        air_drag_per_second: 0.2,
        spring_damping_ratio: 1.,
        cloth_static_friction: 0.2,
        cloth_kinetic_friction: 0.2,
        substeps,
    }
}

/// An n x n flat square sheet of side `size`, centred at (0, y, 0).
pub(crate) fn grid(n: usize, size: f32, y: f32, parameters: ClothParameters) -> ClothSolver {
    let mut points = Vec::new();
    let mut uv = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let (u, v) = (i as f32 / (n - 1) as f32, j as f32 / (n - 1) as f32);
            points.push(Vec3::new((u - 0.5) * size, y, (v - 0.5) * size));
            uv.push(Vec2::new(u, v));
        }
    }
    let mut triangles = Vec::new();
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = (j * n + i) as u32;
            let (b, c, d) = (a + 1, a + n as u32, a + n as u32 + 1);
            triangles.push([a, b, d]);
            triangles.push([a, d, c]);
        }
    }
    ClothSolver::new(points.clone(), points, triangles, uv, parameters).unwrap()
}

fn gpu_batch(solvers: &[&ClothSolver]) -> Option<GpuClothBatch> {
    let batch = GpuClothBatch::new(solvers).unwrap();
    if batch.is_none() {
        println!("GPU_SKIPPED no suitable adapter");
    }
    batch
}

fn box_collider(half: [f32; 3], centre: [f32; 3], friction: f64) -> FixedClothCollider {
    FixedClothCollider::new(
        SharedShape::cuboid(half[0], half[1], half[2]),
        Isometry::translation(centre[0], centre[1], centre[2]),
        friction,
    )
    .unwrap()
}

/// Smallest distance from any vertex of `a` to any triangle of `b` (brute force, tests only).
fn layer_gap(a: &ClothSolver, b: &ClothSolver) -> f64 {
    use parry_ccd::{math::Point, query::PointQuery, shape::Triangle};
    let p = |v: DVec3| Point::new(v.x, v.y, v.z);
    let mut best = f64::INFINITY;
    for v in &a.positions {
        for t in &b.triangles {
            let tri = Triangle::new(p(b.positions[t[0]]), p(b.positions[t[1]]), p(b.positions[t[2]]));
            best = best.min(tri.distance_to_local_point(&p(*v), true));
        }
    }
    best
}

fn max_deviation(a: &ClothSolver, b: &ClothSolver) -> f64 {
    a.positions.iter().zip(&b.positions).map(|(p, q)| p.distance(*q)).fold(0., f64::max)
}

#[test]
fn gpu_sheet_drapes_over_a_box_like_the_cpu_reference() {
    let floor = [box_collider([0.3, 0.1, 0.3], [0., -0.1, 0.], 0.7)];
    let mut cpu = grid(21, 1., 0.1, sheet_params(24));
    let mut gpu = grid(21, 1., 0.1, sheet_params(24));
    let Some(mut gbatch) = gpu_batch(&[&gpu]) else { return };
    let mut cbatch = ClothBatch::new(&[&cpu]).unwrap();
    let mut worst_ms = 0f64;
    for _ in 0..120 {
        for (batch_cpu, solver) in [(true, &mut cpu), (false, &mut gpu)] {
            let mut layers = [ClothLayerStep { solver, colliders: &floor, grips: &mut [] }];
            if batch_cpu {
                cbatch.step(&mut layers, 1. / 60., -DVec3::Y * 9.81).unwrap();
            } else {
                gbatch.step(&mut layers, 1. / 60., -DVec3::Y * 9.81).unwrap();
                worst_ms = worst_ms.max(gbatch.last.frame_ms);
            }
        }
    }
    let report = serde_json::json!({
        "max_deviation_m": max_deviation(&cpu, &gpu),
        "mean_height_cpu": cpu.positions.iter().map(|p| p.y).sum::<f64>() / cpu.positions.len() as f64,
        "mean_height_gpu": gpu.positions.iter().map(|p| p.y).sum::<f64>() / gpu.positions.len() as f64,
        "penetration_cpu": cpu.max_particle_penetration(&floor),
        "penetration_gpu": gpu.max_particle_penetration(&floor),
        "stretch_cpu": cpu.max_edge_stretch_ratio(), "stretch_gpu": gpu.max_edge_stretch_ratio(),
        "gpu_worst_frame_ms": worst_ms, "gpu": gbatch.last,
    });
    println!("GPU_PARITY_DRAPE {report}");
    assert!(gpu.max_particle_penetration(&floor) < 1e-5, "{report}");
    assert!(gpu.max_edge_stretch_ratio() < 1.1, "{report}");
    // The drape's buckles need not fold the same way; its height and contact must agree.
    let mean_y = |s: &ClothSolver| s.positions.iter().map(|p| p.y).sum::<f64>() / s.positions.len() as f64;
    assert!((mean_y(&cpu) - mean_y(&gpu)).abs() < 0.005, "{report}");
}

#[test]
fn gpu_layers_stack_at_their_radii_without_crossing() {
    let params = |thickness: f32| ClothParameters { thickness_m: thickness, ..sheet_params(24) };
    let mut layers = [grid(9, 0.8, 0.02, params(0.001)), grid(9, 0.8, 0.05, params(0.003)), grid(9, 0.8, 0.1, params(0.012))];
    let floor = [box_collider([2., 0.1, 2.], [0., -0.1, 0.], 0.5)];
    let Some(mut batch) = gpu_batch(&layers.iter().collect::<Vec<_>>()) else { return };
    let mut worst_gap_deficit = 0f64;
    let mut flags = (0u32, 0u32);
    for tick in 0..180 {
        let mut steps: Vec<_> = layers
            .iter_mut()
            .map(|solver| ClothLayerStep { solver, colliders: &floor, grips: &mut [] })
            .collect();
        if let Err(e) = batch.step(&mut steps, 1. / 60., -DVec3::Y * 9.81) {
            panic!("tick {tick}: {e} {:?}", batch.last);
        }
        flags.0 += batch.last.substep_motion_exceeded;
        flags.1 += batch.last.margin_exceeded;
        if tick % 10 == 9 {
            for pair in layers.windows(2) {
                let gap = (pair[0].parameters.thickness_m + pair[1].parameters.thickness_m) as f64 * 0.5;
                worst_gap_deficit = worst_gap_deficit.max(gap - layer_gap(&pair[1], &pair[0]));
            }
        }
    }
    let heights: Vec<f64> = layers.iter().map(|l| l.positions.iter().map(|p| p.y).sum::<f64>() / l.positions.len() as f64).collect();
    let report = serde_json::json!({"mean_heights": heights, "expected": [0.0005, 0.0025, 0.010],
        "worst_gap_deficit_m": worst_gap_deficit, "tunnel_flags": flags, "resting": batch.resting()});
    println!("GPU_STACK {report}");
    // f32 positions and averaged contact: the stack holds its gaps to within 50 um.
    assert!(worst_gap_deficit < 5e-5, "{report}");
    for (h, e) in heights.iter().zip([0.0005, 0.0025, 0.010]) {
        assert!((h - e).abs() < 1e-4, "{report}");
    }
}

#[test]
fn gpu_grip_holds_a_hanging_sheet_like_the_cpu_reference() {
    let clamp = ClampMaterial { compliance_m_n: 0.0002, damping_ratio: 1.0, capacity_n: 40. };
    let make = || {
        let s = grid(11, 0.5, 1.0, sheet_params(24));
        let pose = Isometry::translation(-0.25 + 0.03, 1.0 + 0.015, -0.25 + 0.03);
        let grip = ClothGrip::capture(s.surface(), &Ball::new(0.015), &pose, 0, clamp).unwrap().unwrap();
        (s, grip, pose)
    };
    let (mut cpu, cpu_grip, pose) = make();
    let (mut gpu, gpu_grip, _) = make();
    let Some(mut gbatch) = gpu_batch(&[&gpu]) else { return };
    let mut cbatch = ClothBatch::new(&[&cpu]).unwrap();
    let mut grips = [vec![cpu_grip], vec![gpu_grip]];
    let mut deviation = Vec::new();
    for frame in 0..90 {
        if frame % 10 == 0 {
            deviation.push(max_deviation(&cpu, &gpu));
        }
        // The hand rises 20 cm over the first second, then holds.
        let lift = 0.2 * (frame as f32 / 60.).min(1.);
        let p = Isometry::translation(pose.translation.x, pose.translation.y + lift, pose.translation.z);
        for (k, solver) in [&mut cpu, &mut gpu].into_iter().enumerate() {
            grips[k][0].observe_pose(&p).unwrap();
            let mut layers = [ClothLayerStep { solver, colliders: &[], grips: &mut grips[k] }];
            if k == 0 {
                cbatch.step(&mut layers, 1. / 60., -DVec3::Y * 9.81).unwrap();
            } else {
                gbatch.step(&mut layers, 1. / 60., -DVec3::Y * 9.81).unwrap();
            }
        }
    }
    let report = serde_json::json!({
        "max_deviation_m": max_deviation(&cpu, &gpu), "deviation_every_10": deviation,
        "centroid_gap_m": (cpu.positions.iter().sum::<DVec3>() - gpu.positions.iter().sum::<DVec3>()).length() / cpu.positions.len() as f64,
        "state": [format!("{:?}", grips[0][0].state()), format!("{:?}", grips[1][0].state())],
        "force_n": [grips[0][0].peak_applied_force_n(), grips[1][0].peak_applied_force_n()],
        "stretch": [cpu.max_edge_stretch_ratio(), gpu.max_edge_stretch_ratio()],
    });
    println!("GPU_PARITY_GRIP {report}");
    assert_eq!(grips[1][0].state(), crate::cc_render::cloth_grip::ClampState::Held, "{report}");
    let (fc, fg) = (grips[0][0].peak_applied_force_n(), grips[1][0].peak_applied_force_n());
    assert!((fc - fg).abs() < 0.1 * fc.max(0.1), "{report}");
    // Spring order differs (coloured batches vs sequential Gauss-Seidel), so a freely swinging
    // sheet's vertices drift apart over seconds; its body motion must not.
    let centroid = |s: &ClothSolver| s.positions.iter().sum::<DVec3>() / s.positions.len() as f64;
    assert!(centroid(&cpu).distance(centroid(&gpu)) < 0.02, "{report}");
}

/// Largest depth of any particle inside a capsule along local y (after the cloth radius).
fn capsule_penetration(s: &ClothSolver, centre: DVec3, rotation: DQuat, half_height: f64, radius: f64) -> f64 {
    let r = s.parameters.thickness_m as f64 * 0.5;
    s.positions
        .iter()
        .map(|p| {
            let l = rotation.inverse() * (*p - centre);
            let d = (l - DVec3::new(0., l.y.clamp(-half_height, half_height), 0.)).length();
            (radius + r - d).max(0.)
        })
        .fold(0., f64::max)
}

#[test]
fn gpu_sheet_drapes_over_a_static_capsule() {
    // A horizontal bar (capsule along x) 30 cm above a floor; a sheet falls onto it.
    let rotation = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2);
    let centre = DVec3::new(0., 0.3, 0.);
    let bar = KinematicCollider {
        shape: KinematicShape::Capsule { half_height: 0.4, radius: 0.05 },
        start: (centre, rotation),
        end: (centre, rotation),
        friction: 0.5,
    };
    let floor = [box_collider([2., 0.1, 2.], [0., -0.1, 0.], 0.7)];
    let mut sheet = grid(21, 0.8, 0.45, sheet_params(24));
    let Some(mut batch) = gpu_batch(&[&sheet]) else { return };
    let mut worst = 0f64;
    for _ in 0..180 {
        let mut layers = [ClothLayerStep { solver: &mut sheet, colliders: &floor, grips: &mut [] }];
        batch.step_with_kinematic(&mut layers, 1. / 60., -DVec3::Y * 9.81, &[bar]).unwrap();
        worst = worst.max(capsule_penetration(&sheet, centre, rotation, 0.4, 0.05));
    }
    let top = sheet.positions.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let report = serde_json::json!({"worst_capsule_penetration_m": worst, "highest_point_m": top,
        "floor_penetration_m": sheet.max_particle_penetration(&floor), "gpu": batch.last});
    println!("GPU_CAPSULE_STATIC {report}");
    assert!(worst < 1e-4, "{report}");
    // Hanging over the bar: the middle rests on top of it, the sides reach the floor.
    assert!((top - 0.351).abs() < 0.01, "{report}");
    assert!(sheet.max_particle_penetration(&floor) < 1e-5, "{report}");
}

#[test]
fn gpu_moving_capsule_ploughs_a_sheet_without_passing_through() {
    // A sheet resting on a table; an upright capsule (a forearm standing on the table) sweeps
    // 0.6 m along +x through the sheet's edge in one second and pushes the cloth ahead of it.
    let floor = [box_collider([1., 0.1, 1.], [0., -0.1, 0.], 0.3)];
    let mut sheet = grid(21, 0.6, 0.001, sheet_params(24));
    let Some(mut batch) = gpu_batch(&[&sheet]) else { return };
    let (radius, half_height) = (0.04, 0.1);
    let pose = |t: f64| DVec3::new(-0.5 + 0.6 * t.min(1.), half_height - 0.02, 0.);
    let start_centre: f64 = sheet.positions.iter().map(|p| p.x).sum::<f64>() / sheet.positions.len() as f64;
    let mut worst = 0f64;
    let mut flags = (0u32, 0u32);
    for frame in 0..90 {
        let (t0, t1) = (frame as f64 / 60., (frame + 1) as f64 / 60.);
        let capsule = KinematicCollider {
            shape: KinematicShape::Capsule { half_height, radius },
            start: (pose(t0), DQuat::IDENTITY),
            end: (pose(t1), DQuat::IDENTITY),
            friction: 0.5,
        };
        let mut layers = [ClothLayerStep { solver: &mut sheet, colliders: &floor, grips: &mut [] }];
        batch.step_with_kinematic(&mut layers, 1. / 60., -DVec3::Y * 9.81, &[capsule]).unwrap();
        worst = worst.max(capsule_penetration(&sheet, pose(t1), DQuat::IDENTITY, half_height, radius));
        flags.0 += batch.last.substep_motion_exceeded;
        flags.1 += batch.last.margin_exceeded;
    }
    let centre: f64 = sheet.positions.iter().map(|p| p.x).sum::<f64>() / sheet.positions.len() as f64;
    let report = serde_json::json!({"worst_capsule_penetration_m": worst, "sheet_centre_shift_m": centre - start_centre,
        "floor_penetration_m": sheet.max_particle_penetration(&floor), "stretch": sheet.max_edge_stretch_ratio(),
        "tunnel_flags": flags, "gpu": batch.last});
    println!("GPU_CAPSULE_MOVING {report}");
    assert!(worst < 1e-4, "{report}");
    assert!(sheet.max_particle_penetration(&floor) < 1e-5, "{report}");
    assert!(sheet.max_edge_stretch_ratio() < 1.1, "{report}");
    // The capsule ends 0.3 m inside the sheet's original edge: the cloth ahead was pushed along.
    assert!(centre - start_centre > 0.05, "{report}");
}

/// Release cost and contact flags of the moving authored bedding on the GPU (CC_CLOTH_BENCH_*
/// as for the CPU bench). Run: `--release -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark: run in release with --nocapture"]
fn bench_gpu_cloth_ticks() {
    let scene = std::env::var("CC_CLOTH_BENCH_SCENE").unwrap_or_else(|_| "scene-bedding.json".into());
    let ticks: usize = std::env::var("CC_CLOTH_BENCH_TICKS").unwrap_or_else(|_| "300".into()).parse().unwrap();
    let (mut solvers, colliders) = super::batch::bench::authored_layers(&scene);
    let Some(mut batch) = gpu_batch(&solvers.iter().collect::<Vec<_>>()) else { return };
    println!("GPU_ADAPTER {}", gpu_adapter().unwrap_or("none"));
    println!("GPU_LAYOUT {}", batch.layout_summary());
    let mut window = Vec::new();
    let mut flags = (0u32, 0u32);
    let mut published_speed = 0f64;
    for tick in 1..=ticks {
        let before: Vec<Vec<DVec3>> = solvers.iter().map(|s| s.positions.clone()).collect();
        let mut layers: Vec<_> = solvers
            .iter_mut()
            .zip(&colliders)
            .map(|(solver, colliders)| ClothLayerStep { solver, colliders, grips: &mut [] })
            .collect();
        batch.step(&mut layers, 1. / 60., -DVec3::Y * 9.81).unwrap();
        published_speed = published_speed.max(solvers.iter().zip(&before)
            .flat_map(|(s, b)| s.positions.iter().zip(b).map(|(p, q)| p.distance(*q) * 60.)).fold(0., f64::max));
        window.push(batch.last.frame_ms);
        flags.0 += batch.last.substep_motion_exceeded;
        flags.1 += batch.last.margin_exceeded;
        if tick % 30 == 0 {
            window.sort_by(f64::total_cmp);
            println!("GPU_BENCH {}", serde_json::json!({
                "scene": scene, "tick": tick, "median_ms": window[window.len() / 2], "max_ms": window[window.len() - 1],
                "candidates": batch.last.candidates, "contacts": batch.last.contacts, "resting": batch.resting(), "colliders": batch.last.colliders,
                "max_published_speed": published_speed,
                "tunnel_flags_substep_motion": flags.0, "tunnel_flags_margin": flags.1,
                "max_stretch": solvers.iter().map(|s| s.max_edge_stretch_ratio()).fold(0., f64::max),
                "max_penetration_m": solvers.iter().zip(&colliders).map(|(s, c)| s.max_particle_penetration(c)).fold(0., f64::max),
                "max_speed": solvers.iter().map(|s| s.max_speed()).fold(0., f64::max),
                "worst": solvers.iter().map(|s| {
                    let (ratio, at) = s.springs.iter().filter(|e| e.stretch).map(|e| {
                        let [a, b] = e.vertices.map(|i| s.positions[i]);
                        (a.distance(b) / e.rest, ((a + b) * 0.5).to_array().map(|x| (x * 100.).round() / 100.))
                    }).fold((0., [0.; 3]), |m, x| if x.0 > m.0 { x } else { m });
                    let fast = s.velocities.iter().zip(&s.positions).fold((0., [0.; 3]), |m, (v, p)| {
                        let sp = v.length();
                        if sp > m.0 { (sp, p.to_array().map(|x| (x * 100.).round() / 100.)) } else { m }
                    });
                    serde_json::json!({"stretch": ratio, "at": at, "speed": fast.0, "fast_at": fast.1})
                }).collect::<Vec<_>>(),
            }));
            window.clear();
            published_speed = 0.;
        }
    }
}

#[test]
fn gpu_shader_is_valid_wgsl() {
    use wgpu_pin::naga;
    let module = naga::front::wgsl::parse_str(include_str!("cloth_gpu.wgsl"))
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(include_str!("cloth_gpu.wgsl"))));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(include_str!("cloth_gpu.wgsl"))));
}

/// Diagnostic: the authored bedding's first ticks on the GPU, with per-tick inter-layer
/// distances (parry mesh distance, as the live drape test) and vertex-face gap violations.
#[test]
#[ignore = "diagnostic: release, --nocapture"]
fn gpu_bedding_layer_separation_trace() {
    use parry_ccd::{math::{Isometry as I, Point}, query::{distance, PointQuery}, shape::{TriMesh, Triangle}};
    let (mut solvers, colliders) = super::batch::bench::authored_layers("scene-bedding.json");
    let Some(mut batch) = gpu_batch(&solvers.iter().collect::<Vec<_>>()) else { return };
    let ticks: usize = std::env::var("CC_TRACE_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    for tick in 1..=ticks {
        let mut layers: Vec<_> = solvers.iter_mut().zip(&colliders)
            .map(|(solver, colliders)| ClothLayerStep { solver, colliders, grips: &mut [] }).collect();
        if let Err(e) = batch.step(&mut layers, 1. / 60., -DVec3::Y * 9.81) {
            println!("GPU_SEPARATION_ERROR tick {tick} {e} {}", serde_json::json!(batch.last));
            return;
        }
        let every: usize = std::env::var("CC_TRACE_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        if tick % every != 0 { continue; }
        let p = |v: DVec3| Point::new(v.x, v.y, v.z);
        let mesh = |s: &ClothSolver| TriMesh::new(s.positions.iter().map(|v| p(*v)).collect(),
            s.triangles.iter().map(|t| t.map(|i| i as u32)).collect());
        let mut line = Vec::new();
        for a in 0..solvers.len() {
            for b in a + 1..solvers.len() {
                let gap = distance(&I::identity(), &mesh(&solvers[a]), &I::identity(), &mesh(&solvers[b])).unwrap();
                let need = (solvers[a].parameters.thickness_m + solvers[b].parameters.thickness_m) as f64 * 0.5;
                // Vertices of either layer closer to the other's faces than the gap.
                let mut violations = 0;
                for (x, y) in [(a, b), (b, a)] {
                    for v in &solvers[x].positions {
                        for t in &solvers[y].triangles {
                            let [q0, q1, q2] = t.map(|i| solvers[y].positions[i]);
                            if (q0.min(q1).min(q2) - *v).max_element() > need || (*v - q0.max(q1).max(q2)).max_element() > need { continue; }
                            if Triangle::new(p(q0), p(q1), p(q2)).distance_to_local_point(&p(*v), true) < need - 1e-5 { violations += 1; }
                        }
                    }
                }
                line.push(serde_json::json!({"pair": [a, b], "mesh_gap_m": gap, "needed_m": need, "vertex_face_violations": violations}));
            }
        }
        // On the flat mattress top (y > 0.6, away from its edges) the layers' order is known:
        // fitted (1) under top (2). Count top vertices below the fitted surface and their depth.
        let (fitted, top) = (&solvers[1], &solvers[2]);
        let (mut below, mut deepest, mut shallow) = (0, 0f64, 0);
        for v in &top.positions {
            if v.y < 0.6 || v.x < 3.45 || v.x > 4.95 || v.z < -4.3 || v.z > -2.5 { continue; }
            let mut best: Option<(f64, f64)> = None;
            for t in &fitted.triangles {
                let [q0, q1, q2] = t.map(|i| fitted.positions[i]);
                if (q0.min(q1).min(q2) - *v).max_element() > 0.05 || (*v - q0.max(q1).max(q2)).max_element() > 0.05 { continue; }
                let tri = Triangle::new(p(q0), p(q1), p(q2));
                let d = tri.distance_to_local_point(&p(*v), true);
                let mut n = (q1 - q0).cross(q2 - q0).normalize();
                if n.y < 0. { n = -n; }
                let h = (*v - q0).dot(n);
                if best.map_or(true, |b| d < b.0) { best = Some((d, h)); }
            }
            if let Some((d, h)) = best {
                if h < 0. { below += 1; deepest = deepest.max(-h); }
                else if d < 0.001 - 1e-5 { shallow += 1; }
            }
        }
        line.push(serde_json::json!({"top_below_fitted_on_mattress_top": below, "deepest_m": deepest, "top_within_gap": shallow}));
        // Actual triangle-triangle intersections between fitted and top, with where they are.
        if std::env::var_os("CC_TRACE_INTERSECTIONS").is_some() {
            use parry_ccd::query::intersection_test;
            let mut hits = Vec::new();
            for ta in &fitted.triangles {
                let a = ta.map(|i| fitted.positions[i]);
                let (alo, ahi) = (a[0].min(a[1]).min(a[2]), a[0].max(a[1]).max(a[2]));
                for tb in &top.triangles {
                    let b = tb.map(|i| top.positions[i]);
                    if (b[0].min(b[1]).min(b[2]) - ahi).max_element() > 0. || (alo - b[0].max(b[1]).max(b[2])).max_element() > 0. { continue; }
                    let (x, y) = (Triangle::new(p(a[0]), p(a[1]), p(a[2])), Triangle::new(p(b[0]), p(b[1]), p(b[2])));
                    if intersection_test(&I::identity(), &x, &I::identity(), &y).unwrap_or(false) {
                        hits.push(((a[0] + a[1] + a[2]) / 3.).to_array().map(|c| (c * 100.).round() / 100.));
                    }
                }
            }
            line.push(serde_json::json!({"fitted_top_triangle_intersections": hits.len(), "at": hits.iter().take(12).collect::<Vec<_>>()}));
        }
        println!("GPU_SEPARATION tick {tick} {}", serde_json::json!(line));
    }
}

/// Triangle pairs of `a` and `b` that intersect (parry exact test after an AABB filter), and
/// the centroid of the first few.
pub(crate) fn layer_intersections(a: &ClothSolver, b: &ClothSolver) -> (usize, Vec<[f64; 3]>) {
    use parry_ccd::{math::{Isometry as I, Point}, query::intersection_test, shape::Triangle};
    let p = |v: DVec3| Point::new(v.x, v.y, v.z);
    let boxes = |s: &ClothSolver| -> Vec<(DVec3, DVec3)> {
        s.triangles.iter().map(|t| {
            let [x, y, z] = t.map(|i| s.positions[i]);
            (x.min(y).min(z), x.max(y).max(z))
        }).collect()
    };
    let (ba, bb) = (boxes(a), boxes(b));
    let mut count = 0;
    let mut at = Vec::new();
    for (ta, (alo, ahi)) in a.triangles.iter().zip(&ba) {
        for (tb, (blo, bhi)) in b.triangles.iter().zip(&bb) {
            if (*blo - *ahi).max_element() > 0. || (*alo - *bhi).max_element() > 0. { continue; }
            let x = ta.map(|i| a.positions[i]);
            let y = tb.map(|i| b.positions[i]);
            if intersection_test(&I::identity(), &Triangle::new(p(x[0]), p(x[1]), p(x[2])),
                &I::identity(), &Triangle::new(p(y[0]), p(y[1]), p(y[2]))).unwrap_or(false) {
                count += 1;
                if at.len() < 4 {
                    at.push(((x[0] + x[1] + x[2]) / 3.).to_array().map(|c| (c * 100.).round() / 100.));
                }
            }
        }
    }
    (count, at)
}

/// Diagnostic: authored bedding on the GPU, every CC_TRACE_EVERY ticks: inter-layer triangle
/// intersections, worst stretch per layer and where, fastest published motion, cost (frame and
/// GPU wall time) and flags; with CC_GPU_PROFILE=1, GPU milliseconds per stage of the last frame.
#[test]
#[ignore = "diagnostic: release, --nocapture"]
fn gpu_bedding_quality_trace() {
    let scene = std::env::var("CC_TRACE_SCENE").unwrap_or_else(|_| "scene-bedding.json".into());
    let (mut solvers, colliders) = super::batch::bench::authored_layers(&scene);
    let Some(mut batch) = gpu_batch(&solvers.iter().collect::<Vec<_>>()) else { return };
    let ticks: usize = std::env::var("CC_TRACE_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let every: usize = std::env::var("CC_TRACE_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let mut window_ms = Vec::new();
    let mut window_gpu_ms = Vec::new();
    let mut rest_tick: Option<usize> = None;
    let mut fastest = (0usize, [0f64; 3]);
    let mut flags = (0u64, 0u64);
    let mut speed = 0f64;
    for tick in 1..=ticks {
        let before: Vec<Vec<DVec3>> = solvers.iter().map(|s| s.positions.clone()).collect();
        let mut layers: Vec<_> = solvers.iter_mut().zip(&colliders)
            .map(|(solver, colliders)| ClothLayerStep { solver, colliders, grips: &mut [] }).collect();
        if let Err(e) = batch.step(&mut layers, 1. / 60., -DVec3::Y * 9.81) {
            println!("GPU_QUALITY_ERROR tick {tick} {e} {}", serde_json::json!(batch.last));
            return;
        }
        window_ms.push(batch.last.frame_ms);
        window_gpu_ms.push(batch.last.gpu_ms);
        if batch.resting() && rest_tick.is_none() {
            rest_tick = Some(tick);
        }
        flags.0 += batch.last.substep_motion_exceeded as u64;
        flags.1 += batch.last.margin_exceeded as u64;
        for (layer, (s, b)) in solvers.iter().zip(&before).enumerate() {
            for (p, q) in s.positions.iter().zip(b) {
                let v = p.distance(*q) * 60.;
                if v > speed {
                    speed = v;
                    fastest = (layer, p.to_array().map(|x| (x * 100.).round() / 100.));
                }
            }
        }
        if let Ok(path) = std::env::var("CC_TRACE_DUMP") {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
            writeln!(f, "{}", serde_json::json!({"tick": tick, "layers": solvers.iter().map(|s|
                s.positions.iter().map(|p| p.to_array()).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "triangles": if tick == 1 { solvers.iter().map(|s| s.triangles.clone()).collect::<Vec<_>>() } else { vec![] }})).unwrap();
        }
        if tick % every != 0 { continue; }
        let mut pairs = Vec::new();
        for a in 0..solvers.len() {
            for b in a + 1..solvers.len() {
                let (n, at) = layer_intersections(&solvers[a], &solvers[b]);
                pairs.push(serde_json::json!({"pair": [a, b], "intersections": n, "at": at}));
            }
        }
        // Collider penetration per layer, millimetres beyond the half-thickness: particles, and
        // faces by the exact triangle witness (where on the face it is deepest).
        let penetration: Vec<_> = solvers.iter().zip(&colliders).map(|(s, cs)| {
            let radius = s.parameters.thickness_m as f64 * 0.5;
            let mut face = (0f64, [0f64; 3]);
            for t in &s.triangles {
                let v = t.map(|i| s.positions[i]);
                for c in cs.iter() {
                    if let Ok(Some((w, depth, _))) = c.triangle_contact(v, radius) {
                        if depth > face.0 {
                            let at = v[0] * w[0] + v[1] * w[1] + v[2] * w[2];
                            face = (depth, at.to_array().map(|x| (x * 100.).round() / 100.));
                        }
                    }
                }
            }
            serde_json::json!({"particle_mm": (s.max_particle_penetration(cs) * 1e4).round() / 10.,
                "face_mm": (face.0 * 1e4).round() / 10., "face_at": face.1})
        }).collect();
        let stretch: Vec<_> = solvers.iter().map(|s| {
            let (r, at) = s.springs.iter().filter(|e| e.stretch).map(|e| {
                let [x, y] = e.vertices.map(|i| s.positions[i]);
                (x.distance(y) / e.rest, ((x + y) * 0.5).to_array().map(|c| (c * 100.).round() / 100.))
            }).fold((0., [0.; 3]), |m, x| if x.0 > m.0 { x } else { m });
            serde_json::json!([(r * 1000.).round() / 1000., at])
        }).collect();
        window_ms.sort_by(f64::total_cmp);
        window_gpu_ms.sort_by(f64::total_cmp);
        println!("GPU_QUALITY {}", serde_json::json!({
            "gpu_median_ms": window_gpu_ms[window_gpu_ms.len() / 2],
            "profile_ms": batch.profile.iter().map(|(l, ms)| (l, (ms * 1e3).round() / 1e3)).collect::<Vec<_>>(),
            "tick": tick, "resting": batch.resting(), "rest_tick": rest_tick, "intersections": pairs,
            "stretch": stretch, "penetration": penetration,
            "published_speed": speed, "fastest": fastest, "median_ms": window_ms[window_ms.len() / 2],
            "p95_ms": window_ms[(window_ms.len() * 95 / 100).min(window_ms.len() - 1)],
            "max_ms": window_ms[window_ms.len() - 1], "flags": [flags.0, flags.1],
        }));
        window_ms.clear();
        window_gpu_ms.clear();
        speed = 0.;
    }
}

/// Two covers on the authored bed, a top sheet under a duvet (cotton friction, no fitted
/// sheet). The sheet carries the duvet's weight into its mattress friction, so the pair rests
/// instead of gliding toward the foot under its hanging ends: with friction from the sheet's own
/// weight alone, both glided together at ~4 mm/s and never rested.
#[test]
fn gpu_two_covers_rest_on_the_mattress() {
    let (mut solvers, colliders) = super::batch::bench::authored_layers("scene-bedding-two-covers.json");
    let Some(mut batch) = gpu_batch(&solvers.iter().collect::<Vec<_>>()) else { return };
    let mut rest_tick = None;
    let mut at_3s = Vec::new();
    for tick in 1..=300 {
        let mut layers: Vec<_> = solvers.iter_mut().zip(&colliders)
            .map(|(solver, colliders)| ClothLayerStep { solver, colliders, grips: &mut [] }).collect();
        batch.step(&mut layers, 1. / 60., -DVec3::Y * 9.81).unwrap();
        if batch.resting() && rest_tick.is_none() {
            rest_tick = Some(tick);
        }
        if tick == 180 {
            at_3s = solvers.iter().map(|s| s.positions.clone()).collect();
        }
    }
    assert!(rest_tick.is_some(), "the covers did not rest within 5 s");
    for ((s, before), cs) in solvers.iter().zip(&at_3s).zip(&colliders) {
        // On the mattress top, from 3 s to 5 s.
        let drift = s.positions.iter().zip(before).filter(|(_, b)| b.y > 0.6)
            .map(|(p, b)| p.distance(*b)).fold(0f64, f64::max);
        let stretch = s.springs.iter().filter(|e| e.stretch)
            .map(|e| s.positions[e.vertices[0]].distance(s.positions[e.vertices[1]]) / e.rest)
            .fold(0f64, f64::max);
        println!("two covers: rest tick {rest_tick:?}, drift 3-5 s {:.2} mm, stretch {stretch:.3}", drift * 1e3);
        assert!(drift < 0.003, "covers still moving on the mattress: {:.1} mm in 2 s", drift * 1e3);
        assert!(stretch < 1.1, "stretch {stretch}");
        assert!(s.max_particle_penetration(cs) < 1e-6);
    }
    assert_eq!(layer_intersections(&solvers[0], &solvers[1]).0, 0);
}
