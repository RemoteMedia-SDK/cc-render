//! Diagnostic: can one hand turn the condo bed's covers down far enough to get in?
//!
//! The settled bedding (fitted sheet, top sheet, duvet) steps as one layered batch. A 1.5 cm
//! ball standing in for the hand grips the duvet and the top sheet together near the bedside
//! (east) corner of their head edge, lifts, sweeps diagonally towards the foot and the middle,
//! lays them down and lets go. Reported: how much of the entry area (her half of the mattress
//! top, below the pillows to hip level) the covers still cover, before and after, and the clamp
//! forces, where the gripped duvet point ends relative to where it was laid, the fastest published
//! motion after release, and (TURNDOWN_ENERGY lines) each layer's kinetic, gravity, stretch and bend
//! energy around the release. Solver only: no avatar, no runtime hand admission. Long-running (the
//! moving bedding costs about a second per tick even in release): run with `--release -- --ignored`.
//! Options: TURNDOWN_STILL=1 (control: grip and hold still), TURNDOWN_HOLD_S=<s> (hold the laid
//! corner still that long before letting go), TURNDOWN_PARAMS (counterfactual materials, below),
//! CC_TURNDOWN_TRACE=<dir> (per-10-frame grip trace and final layer positions).
use bevy::math::{DVec3, Vec3};
use bevy_rapier3d::rapier::parry::{math::Isometry, shape::Ball};

use super::cloth_grip::{ClampMaterial, ClothGrip};
use super::cloth_solver::{ClothBatch, ClothLayerStep, ClothSolver};
use super::scene_package::ScenePackage;

const DT: f64 = 1.0 / 60.0;
const HAND_RADIUS_M: f32 = 0.015;
/// Mattress top (bed_mattress: centre y 0.48, half height 0.14).
const MATTRESS_TOP_Y: f32 = 0.62;
/// Her half of the mattress top, below the pillows to hip level (x, z ranges).
const ENTRY: ([f32; 2], [f32; 2]) = ([4.2, 5.1], [-4.0, -2.9]);
/// Where the hand takes the covers: the corner of their head edge on her (bedside) side, the
/// top-facing face furthest towards +x and -z.
const CORNER: [f32; 2] = [1., -1.];

fn covered(positions: &[Vec3], triangles: &[[u32; 3]], x: f32, z: f32) -> bool {
    triangles.iter().any(|t| {
        let [a, b, c] = t.map(|i| positions[i as usize]);
        let d = (b.z - c.z) * (a.x - c.x) + (c.x - b.x) * (a.z - c.z);
        if d.abs() < 1e-9 { return false; }
        let u = ((b.z - c.z) * (x - c.x) + (c.x - b.x) * (z - c.z)) / d;
        let v = ((c.z - a.z) * (x - c.x) + (a.x - c.x) * (z - c.z)) / d;
        let w = 1. - u - v;
        let y = u * a.y + v * b.y + w * c.y;
        u >= 0. && v >= 0. && w >= 0. && y > MATTRESS_TOP_Y - 0.02 && y < MATTRESS_TOP_Y + 0.5
    })
}

/// Share of the entry area's 5 cm grid that `positions` covers.
fn coverage(positions: &[Vec3], triangles: &[[u32; 3]]) -> f64 {
    let ([x0, x1], [z0, z1]) = ENTRY;
    let (mut hits, mut total) = (0, 0);
    let mut x = x0;
    while x <= x1 + 1e-4 {
        let mut z = z0;
        while z <= z1 + 1e-4 {
            total += 1;
            hits += covered(positions, triangles, x, z) as usize;
            z += 0.05;
        }
        x += 0.05;
    }
    hits as f64 / total as f64
}

/// The top-facing triangle of `cloth` on the mattress furthest along `towards` (a corner), or
/// nearest `at` on the ground, and the ball pose resting on it.
fn grip_site(cloth: &ClothSolver, triangles: &[[u32; 3]], towards: Option<[f32; 2]>, at: [f32; 2])
    -> (usize, Isometry<f32>) {
    let positions = cloth.surface().positions();
    let (index, centre, normal) = triangles.iter().enumerate().filter_map(|(i, t)| {
        let [a, b, c] = t.map(|k| positions[k as usize]);
        let centre = (a + b + c) / 3.;
        let mut normal = (b - a).cross(c - a).normalize();
        if normal.y < 0. { normal = -normal; }
        (centre.y > MATTRESS_TOP_Y && normal.y > 0.7).then_some((i, centre, normal))
    }).min_by(|(_, p, _), (_, q, _)| {
        let d = |v: &Vec3| match towards {
            Some([dx, dz]) => -(v.x * dx + v.z * dz),
            None => (v.x - at[0]).hypot(v.z - at[1]),
        };
        d(p).total_cmp(&d(q))
    }).expect("a top-facing triangle");
    let p = centre + normal * HAND_RADIUS_M;
    (index, Isometry::translation(p.x, p.y, p.z))
}

/// The hand's offset from where it gripped, at `t` seconds: up 0.35 m, then over to the corner's
/// mirror image across the fold line (from the middle of the head edge to the bedside edge 0.9 m
/// down), then down onto the covers: the corner folded back over them.
fn hand_offset(start: Vec3, t: f32) -> Vec3 {
    let ease = |s: f32| { let s = s.clamp(0., 1.); s * s * (3. - 2. * s) };
    let lift = Vec3::Y * 0.35;
    let fold = Vec3::new(4.2, 0., start.z + 0.9);
    let laid = Vec3::new(fold.x, start.y + 0.35, fold.z) - start;
    let down = Vec3::new(fold.x, MATTRESS_TOP_Y + 0.16, fold.z) - start;
    if t < 0.4 { lift * ease(t / 0.4) }
    else if t < 1.6 { lift.lerp(laid, ease((t - 0.4) / 1.2)) }
    else { laid.lerp(down, ease((t - 1.6) / 0.4)) }
}

#[test]
#[ignore]
fn one_hand_turns_the_covers_down_far_enough_to_get_into_bed() {
    let mut package = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-bedding-settled.json")).unwrap();
    // Counterfactual materials (diagnostic only): TURNDOWN_PARAMS=<layer id or *>.<parameter>=<json
    // value>,... overrides authored cloth parameters before the solvers are built. The baked drape
    // then need not be at rest under them; compare with a TURNDOWN_STILL control.
    if let Ok(spec) = std::env::var("TURNDOWN_PARAMS") {
        for item in spec.split(',').filter(|s| !s.is_empty()) {
            let (key, value) = item.split_once('=').expect("<layer>.<parameter>=<value>");
            let (id, field) = key.split_once('.').expect("<layer>.<parameter>");
            for cloth in package.cloths.iter_mut().filter(|c| id == "*" || c.id == id) {
                let mut parameters = serde_json::to_value(cloth.parameters).unwrap();
                assert!(parameters.get(field).is_some(), "unknown cloth parameter {field}");
                parameters[field] = serde_json::from_str(value).unwrap();
                cloth.parameters = serde_json::from_value(parameters).unwrap();
            }
        }
    }
    let layer = |id: &str| package.cloths.iter().position(|c| c.id == id).unwrap();
    let (fitted, top, duvet) = (layer("bed_fitted_sheet"), layer("bed_top_sheet"), layer("bed_duvet"));
    let mut solvers: Vec<ClothSolver> = package.cloths.iter().map(|c| c.solver().unwrap()).collect();
    let colliders: Vec<_> = package.cloths.iter().map(|c| c.colliders(&package).unwrap()).collect();
    let triangles: Vec<Vec<[u32; 3]>> = package.cloths.iter()
        .map(|c| c.loaded.as_ref().unwrap().triangles.clone()).collect();
    let mut batch = ClothBatch::new(&solvers.iter().collect::<Vec<_>>()).unwrap();
    batch.start_at_rest();
    let cover = |solvers: &[ClothSolver], i: usize| coverage(solvers[i].surface().positions(), &triangles[i]);
    let before = (cover(&solvers, top), cover(&solvers, duvet));

    let clamp = ClampMaterial { compliance_m_n: 0.0002, damping_ratio: 1.0, capacity_n: 40. };
    let (duvet_triangle, duvet_pose) = grip_site(&solvers[duvet], &triangles[duvet], Some(CORNER), [0., 0.]);
    let duvet_start = Vec3::from(duvet_pose.translation.vector);
    let (top_triangle, top_pose) = grip_site(&solvers[top], &triangles[top], None, [duvet_start.x, duvet_start.z]);
    let ball = Ball::new(HAND_RADIUS_M);
    let mut grips: Vec<Vec<ClothGrip>> = vec![Vec::new(); solvers.len()];
    for (i, triangle, pose) in [(duvet, duvet_triangle, duvet_pose), (top, top_triangle, top_pose)] {
        let grip = ClothGrip::capture(solvers[i].surface(), &ball, &pose, triangle, clamp).unwrap();
        grips[i].push(grip.unwrap_or_else(|| panic!("no contact to grip {}", package.cloths[i].id)));
    }
    let starts: Vec<Option<Vec3>> = (0..solvers.len()).map(|i| match i {
        i if i == duvet => Some(duvet_start),
        i if i == top => Some(Vec3::from(top_pose.translation.vector)),
        _ => None,
    }).collect();

    // Control (TURNDOWN_STILL=1): the hand grips and holds still, to tell the turndown from the
    // bedding settling once woken.
    let still = std::env::var("TURNDOWN_STILL").as_deref() == Ok("1");
    let mut frames = Vec::new();
    let clock = std::time::Instant::now();
    // TURNDOWN_HOLD_S: seconds held still on the covers after laying the corner down, before letting go.
    let hold_s: f32 = std::env::var("TURNDOWN_HOLD_S").ok().and_then(|v| v.parse().ok()).unwrap_or(0.);
    let release_frame = 121 + (hold_s * 60.) as u32;
    let last_frame = release_frame + 179;
    let mut peak_force = 0f64;
    let mut held_through = true;
    // The gripped duvet point where the hand lays it down (the last held tick), its largest
    // excursion from there after release, and the fastest cloth after release.
    let mut laid = Vec3::ZERO;
    let mut holding_force = 0f64;
    let mut max_excursion = 0f32;
    let mut max_speed_after = 0f64;
    // Published motion: the largest particle displacement between consecutive ticks over the
    // tick. Unlike the last substep's velocity (`max_speed`), it does not count the sub-mm
    // exchange between material and contact projections in a stack that is not moving.
    let mut max_tick_speed_after = 0f64;
    let mut last_published: Vec<Vec<Vec3>> = solvers.iter().map(|s| s.surface().positions().to_vec()).collect();
    for frame in 1..=last_frame {
        let t = frame as f32 / 60.;
        for (i, layer_grips) in grips.iter_mut().enumerate() {
            for grip in layer_grips.iter_mut() {
                let p = starts[i].unwrap() + if still { Vec3::ZERO } else { hand_offset(duvet_start, t) };
                grip.observe_pose(&Isometry::translation(p.x, p.y, p.z)).unwrap();
                if frame == release_frame { grip.release(); }
            }
        }
        let mut layers: Vec<ClothLayerStep> = solvers.iter_mut().zip(&colliders).zip(grips.iter_mut())
            .map(|((solver, colliders), grips)| ClothLayerStep { solver, colliders, grips }).collect();
        batch.step(&mut layers, DT, -DVec3::Y * 9.81).unwrap();
        for grip in grips.iter().flatten() {
            peak_force = peak_force.max(grip.peak_applied_force_n());
            if frame < release_frame { held_through &= grip.state() == super::cloth_grip::ClampState::Held; }
        }
        let point = solvers[duvet].surface().resolve(grips[duvet][0].material_point()).unwrap().position;
        if frame + 1 == release_frame {
            laid = point;
            holding_force = grips[duvet][0].peak_applied_force_n();
        }
        let published: Vec<Vec<Vec3>> = solvers.iter().map(|s| s.surface().positions().to_vec()).collect();
        let tick_speed = published.iter().zip(&last_published)
            .flat_map(|(a, b)| a.iter().zip(b).map(|(p, q)| p.distance(*q) as f64 / DT))
            .fold(0., f64::max);
        last_published = published;
        if frame >= release_frame {
            max_excursion = max_excursion.max(point.distance(laid));
            max_speed_after = max_speed_after.max(solvers.iter().map(|s| s.max_speed()).fold(0., f64::max));
            max_tick_speed_after = max_tick_speed_after.max(tick_speed);
        }
        if frame % 10 == 0 {
            let grip = &grips[duvet][0];
            let point = solvers[duvet].surface().resolve(grip.material_point()).unwrap().position;
            frames.push(serde_json::json!({"frame": frame, "state": grip.state(),
                "hand": (duvet_start + if still { Vec3::ZERO } else { hand_offset(duvet_start, t) }).to_array(),
                "duvet_point": point.to_array(), "anchor": grip.anchor(1.).to_array(),
                "max_edge_stretch_ratio": solvers.iter().map(|s| s.max_edge_stretch_ratio()).fold(0., f64::max),
                "max_speed_m_s": solvers.iter().map(|s| s.max_speed()).fold(0., f64::max)}));
        }
        // Energy budget around the release: where does the spring-back's kinetic energy come
        // from? Per layer [kinetic, gravity, stretch, bend] in joules, and the duvet's speed.
        if frame + 10 >= release_frame && frame <= release_frame + 90 {
            let gravity = -DVec3::Y * 9.81;
            let point = solvers[duvet].surface().resolve(grips[duvet][0].material_point()).unwrap().position;
            let energies = [fitted, top, duvet].map(|i| solvers[i].energy_terms(gravity));
            let speeds = [fitted, top, duvet].map(|i| solvers[i].max_speed());
            println!("TURNDOWN_ENERGY {}", serde_json::json!({
                "frame": frame, "since_release_s": (frame as f64 - release_frame as f64) / 60.,
                "layers": energies, "speeds": speeds, "duvet_point": point.to_array(),
            }));
        }
        if frame % 30 == 0 {
            eprintln!("turndown frame {frame}/{last_frame} after {:.0} s", clock.elapsed().as_secs_f64());
        }
    }
    let after = (cover(&solvers, top), cover(&solvers, duvet));
    let final_point = solvers[duvet].surface().resolve(grips[duvet][0].material_point()).unwrap().position;
    // Largest particle displacement from the baked drape, over every layer.
    let max_drift = solvers.iter().zip(&package.cloths)
        .flat_map(|(s, c)| s.surface().positions().iter().zip(&c.loaded.as_ref().unwrap().initial_positions)
            .map(|(p, q)| p.distance(Vec3::from(*q))))
        .fold(0f32, f32::max);
    let either = |s: &[ClothSolver]| {
        let ([x0, x1], [z0, z1]) = ENTRY;
        let (mut hits, mut total) = (0, 0);
        let mut x = x0;
        while x <= x1 + 1e-4 {
            let mut z = z0;
            while z <= z1 + 1e-4 {
                total += 1;
                hits += [top, duvet].iter().any(|&i| covered(s[i].surface().positions(), &triangles[i], x, z)) as usize;
                z += 0.05;
            }
            x += 0.05;
        }
        hits as f64 / total as f64
    };
    let report = serde_json::json!({
        "entry_area_m": {"x": ENTRY.0, "z": ENTRY.1},
        "covered_before": {"top_sheet": before.0, "duvet": before.1},
        "covered_after": {"top_sheet": after.0, "duvet": after.1, "either": either(&solvers)},
        "grip": {"duvet_at": duvet_start.to_array(), "held_until_release": held_through,
                 "peak_applied_force_n": peak_force, "capacity_n": clamp.capacity_n,
                 "duvet_force_last_held_tick_n": holding_force},
        "laid_point": laid.to_array(), "final_point": final_point.to_array(),
        "final_distance_from_laid_m": final_point.distance(laid),
        "max_excursion_from_laid_m": max_excursion, "max_speed_after_release_m_s": max_speed_after,
        "max_published_speed_after_release_m_s": max_tick_speed_after,
        "parameters": package.cloths.iter().map(|c| (c.id.clone(), c.parameters))
            .collect::<std::collections::BTreeMap<_, _>>(),
        "max_edge_stretch_ratio": solvers.iter().map(|s| s.max_edge_stretch_ratio()).fold(0., f64::max),
        "max_particle_penetration_m": solvers.iter().zip(&colliders)
            .map(|(s, c)| s.max_particle_penetration(c)).fold(0., f64::max),
        "max_speed_m_s": solvers.iter().map(|s| s.max_speed()).fold(0., f64::max),
        "max_drift_from_start_m": max_drift,
        "wall_seconds": clock.elapsed().as_secs_f64(), "hand_still": still,
    });
    println!("TURNDOWN_REPORT {report}");
    if let Ok(dir) = std::env::var("CC_TURNDOWN_TRACE") {
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();
        let layers: Vec<_> = [fitted, top, duvet].iter().map(|&i| serde_json::json!({
            "id": package.cloths[i].id, "triangles": triangles[i],
            "positions": solvers[i].surface().positions().iter().map(|p| p.to_array()).collect::<Vec<_>>(),
            "initial": package.cloths[i].loaded.as_ref().unwrap().initial_positions,
        })).collect();
        std::fs::write(dir.join("turndown.json"), serde_json::to_vec(&serde_json::json!({
            "report": report, "layers": layers, "frames": frames })).unwrap()).unwrap();
    }
    assert!(held_through, "{report}");
}
