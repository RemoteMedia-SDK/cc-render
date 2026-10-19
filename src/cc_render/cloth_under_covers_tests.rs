//! Diagnostic: she lies under the covers and they conform to her.
//!
//! The two-cover bed (top sheet under a duvet) steps on the live GPU batch. Two hands at the
//! duvet's bedside edge (near the head and near the foot) fold it back to the far side; she lies
//! down on her cleared half; the hands take the same edge again, draw the duvet back over her, lay
//! it on what they find (her or the bed) and let go. (A one-hand diagonal turndown leaves the
//! duvet under her legs: she lay on it, and the hand could not pull it out from under her.) Her body is a model-generated supine posture (ARDY, unconstrained,
//! unmodified; owner choice 2026-10-18) skinned on the condo avatar (calibration-v13 renderer
//! geometry, outfit included); the test only places it rigidly on the mattress. The cloth meets
//! her as kinematic capsules fitted to her skinned surface; everything measured is against the
//! skinned surface itself: how much of each body region the duvet covers, how closely it follows
//! her (gap from her top surface to the duvet), whether it sinks into her, stretch, rest and cost.
//! Solver only: no motion, no runtime admission. Run in release with `-- --ignored --nocapture`.
//! Options: CC_COVERS_FRAMES=<file> (layers, hand and capsules every other tick, for previews).
use bevy::math::{DQuat, DVec3, Mat4, Vec3};
use bevy_rapier3d::rapier::parry::{math::Isometry, shape::Ball};

use super::cloth_grip::{ClampMaterial, ClothGrip};
use super::cloth_solver::{ClothLayerStep, ClothSolver, GpuClothBatch, KinematicCollider, KinematicShape};
use super::cloth_turndown_tests as turndown;
use super::office::AvatarGeometry;
use super::renderer::SkeletalPose;
use super::scene_package::ScenePackage;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scenes/toronto_condo");
/// calibration-v13 renderer geometry: file, avatar and profile bindings.
const GEOMETRY_SHA256: &str = "01ed26cfd6f274f72fafc938ddf34a7ffe44fed4f86a6eda44a89e68b30da8ff";
const AVATAR_SHA256: &str = "aaca98681c5c1e950be0830503d8803e8c00d33932e7c49a891559ca94e564c7";
const PROFILE: &str = "7d821e39966c7e518d9bd9d6f9e8a9fbab2e57f0791bd1696679fef1d09f51ef";
/// Where she lies: her half of the mattress (centre x), the top of her head this far from the
/// headboard end of the mattress, her lowest skinned point just above the sheet.
const BODY_CENTRE_X: f32 = 4.65;
const HEAD_TOP_Z: f32 = -4.30;
const BODY_ABOVE_MATTRESS_M: f32 = 0.002;
const BODY_FRICTION: f64 = 0.6;

/// Diagnostics: COVERS_<NAME> overrides a choreography number.
fn knob(name: &str, default: f32) -> f32 {
    std::env::var(format!("COVERS_{name}")).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize)]
enum Region { Head, Torso, Arms, Legs, Feet }

/// SMPL-22 joint pairs the capsules are fitted along, and the region each belongs to.
/// 0 pelvis, 1/2 hips, 3 spine1, 4/5 knees, 6 spine2, 7/8 ankles, 9 spine3, 10/11 feet, 12 neck,
/// 13/14 collars, 15 head, 16/17 shoulders, 18/19 elbows, 20/21 wrists.
const SEGMENTS: [(usize, usize, Region); 21] = [
    (0, 3, Region::Torso), (3, 6, Region::Torso), (6, 9, Region::Torso), (9, 12, Region::Torso),
    (1, 2, Region::Torso), (16, 17, Region::Torso), (12, 15, Region::Head), (15, 15, Region::Head),
    (1, 4, Region::Legs), (4, 7, Region::Legs), (7, 10, Region::Feet),
    (2, 5, Region::Legs), (5, 8, Region::Legs), (8, 11, Region::Feet),
    (16, 18, Region::Arms), (18, 20, Region::Arms), (20, 20, Region::Arms),
    (17, 19, Region::Arms), (19, 21, Region::Arms), (21, 21, Region::Arms), (13, 14, Region::Torso),
];

struct Body {
    /// Skinned surface (avatar and outfit), placed on the bed.
    points: Vec<Vec3>,
    region: Vec<Region>,
    /// Capsule end points and radius, placed.
    capsules: Vec<(Vec3, Vec3, f32, Region)>,
}

fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let t = if ab.length_squared() < 1e-12 { 0. } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0., 1.) };
    a + ab * t
}

fn quantile(mut v: Vec<f32>, q: f32) -> f32 {
    if v.is_empty() { return 0.; }
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * q).round() as usize]
}

/// Her skinned surface in the fixture pose, placed rigidly on the mattress (a yaw that points her
/// head at the headboard, and a translation), and capsules fitted to it.
fn body() -> Body {
    let geometry = AvatarGeometry::load_bound(
        &std::path::Path::new(ROOT).join("calibration-v13/renderer-geometry.json"),
        GEOMETRY_SHA256, AVATAR_SHA256, PROFILE).unwrap();
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::path::Path::new(ROOT).join("interaction-authoring/lying/supine-fixture.json")).unwrap()).unwrap();
    assert_eq!(fixture["kind"], "model_generated_posture_fixture");
    let quats: Vec<[f32; 4]> = serde_json::from_value(fixture["joint_quats_xyzw"].clone()).unwrap();
    let pose = SkeletalPose {
        stream: None,
        joint_quats: quats.try_into().unwrap(),
        root_pos: serde_json::from_value(fixture["root_pos"].clone()).unwrap(),
        pts_ms: 0,
    };
    let world: Vec<Mat4> = geometry.world(&pose).unwrap();
    let joints: Vec<Vec3> = (0..22).map(|i| geometry.smpl_joint(&world, i)).collect();
    let skin: Vec<Vec3> = geometry.meshes.iter().flat_map(|m| m.points(&world)).collect();
    // Yaw: pelvis -> head along -z (the headboard).
    let axis = joints[15] - joints[0];
    let yaw = (-axis.x).atan2(-axis.z);
    let rotation = bevy::math::Quat::from_rotation_y(-yaw);
    let turned: Vec<Vec3> = skin.iter().map(|p| rotation * *p).collect();
    let lo = turned.iter().fold(Vec3::splat(f32::MAX), |m, p| m.min(*p));
    let hi = turned.iter().fold(Vec3::splat(f32::MIN), |m, p| m.max(*p));
    let shift = Vec3::new(BODY_CENTRE_X - (lo.x + hi.x) * 0.5,
        turndown::MATTRESS_TOP_Y + BODY_ABOVE_MATTRESS_M - lo.y, HEAD_TOP_Z - lo.z);
    let place = |p: Vec3| rotation * p + shift;
    let points: Vec<Vec3> = skin.iter().map(|p| place(*p)).collect();
    let joints: Vec<Vec3> = joints.iter().map(|p| place(*p)).collect();
    // Segment axes: joint pairs; a single joint (head, hands) extends along its parent bone.
    let axes: Vec<(Vec3, Vec3, Region)> = SEGMENTS.iter().map(|&(a, b, region)| {
        if a != b { return (joints[a], joints[b], region); }
        let parent = match a { 15 => 12, 20 => 18, _ => 19 };
        let dir = (joints[a] - joints[parent]).normalize();
        let reach = if a == 15 { 0.12 } else { 0.09 };
        (joints[a], joints[a] + dir * reach, region)
    }).collect();
    // Each skinned point belongs to its nearest axis.
    let nearest: Vec<usize> = points.iter().map(|p| {
        (0..axes.len()).min_by(|&i, &j| {
            let d = |k: usize| p.distance_squared(closest_on_segment(*p, axes[k].0, axes[k].1));
            d(i).total_cmp(&d(j))
        }).unwrap()
    }).collect();
    let region = nearest.iter().map(|&k| axes[k].2).collect();
    // Each segment's capsules, in its own frame: along the bone (u), up (v: world up made
    // perpendicular to u), across (w). Her points nearest the segment set its thickness (10th to
    // 90th percentile up) and its width (5th to 95th across): capsules of half that thickness, as
    // many side by side as the width needs, along the bone, the bottom kept clear of the
    // mattress. One round capsule per segment stood 9 cm proud of her chest (a torso is wider
    // than it is deep).
    let capsules = axes.iter().enumerate().flat_map(|(k, &(a, b, region))| {
        let mine: Vec<Vec3> = points.iter().zip(&nearest).filter(|(_, n)| **n == k).map(|(p, _)| *p).collect();
        if mine.len() < 10 { return vec![(a, b, 0.02, region)]; }
        let u = (b - a).normalize_or(Vec3::Z);
        let mut v = Vec3::Y - u * u.y;
        if v.length() < 0.3 { v = Vec3::Z - u * u.z; }
        let v = v.normalize();
        let w = u.cross(v);
        let up: Vec<f32> = mine.iter().map(|p| (*p - a).dot(v)).collect();
        let across: Vec<f32> = mine.iter().map(|p| (*p - a).dot(w)).collect();
        let (up_lo, up_hi) = (quantile(up.clone(), 0.1), quantile(up, 0.9));
        let (across_lo, across_hi) = (quantile(across.clone(), 0.05), quantile(across, 0.95));
        let mut r = ((up_hi - up_lo) * 0.5).max(0.015);
        let mut centre_up = (up_hi + up_lo) * 0.5;
        let clearance = a.y.min(b.y) + centre_up * v.y - r - (turndown::MATTRESS_TOP_Y + 0.003);
        if clearance < 0. {
            // Raise the bottom, keep the top.
            let lift = (-clearance).min(r - 0.015) * 0.5;
            r -= lift;
            centre_up += lift;
        }
        let half_width = ((across_hi - across_lo) * 0.5 - r).max(0.);
        let centre_across = (across_hi + across_lo) * 0.5;
        let count = if half_width <= 0. { 1 } else { (half_width / r).ceil() as usize + 1 };
        (0..count).map(|i| {
            let side = if count == 1 { 0. } else { -half_width + 2. * half_width * i as f32 / (count - 1) as f32 };
            let offset = v * centre_up + w * (centre_across + side);
            (a + offset, b + offset, r, region)
        }).collect::<Vec<_>>()
    }).collect();
    Body { points, region, capsules }
}

/// Her capsules for a frame in which she moves down from `above.0` to `above.1` metres over her
/// lying place.
fn kinematic(body: &Body, above: (f32, f32)) -> Vec<KinematicCollider> {
    body.capsules.iter().map(|&(a, b, r, _)| {
        let centre = ((a + b) * 0.5).as_dvec3();
        let lift = |h: f32| centre + DVec3::Y * h as f64;
        let axis = (b - a).as_dvec3();
        let half = axis.length() * 0.5;
        let rotation = if half < 1e-6 { DQuat::IDENTITY } else { DQuat::from_rotation_arc(DVec3::Y, axis / (2. * half)) };
        KinematicCollider {
            shape: if half < 1e-6 { KinematicShape::Sphere { radius: r as f64 } }
                else { KinematicShape::Capsule { half_height: half, radius: r as f64 } },
            start: (lift(above.0), rotation),
            end: (lift(above.1), rotation),
            friction: knob("BODY_FRICTION", BODY_FRICTION as f32) as f64,
        }
    }).collect()
}

/// The duvet's height above (x, z), or None if no duvet triangle lies over it.
fn duvet_over(positions: &[Vec3], triangles: &[[u32; 3]], x: f32, z: f32, below: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in triangles {
        let [a, b, c] = t.map(|i| positions[i as usize]);
        if x < a.x.min(b.x).min(c.x) || x > a.x.max(b.x).max(c.x) || z < a.z.min(b.z).min(c.z) || z > a.z.max(b.z).max(c.z) { continue; }
        let d = (b.z - c.z) * (a.x - c.x) + (c.x - b.x) * (a.z - c.z);
        if d.abs() < 1e-9 { continue; }
        let u = ((b.z - c.z) * (x - c.x) + (c.x - b.x) * (z - c.z)) / d;
        let v = ((c.z - a.z) * (x - c.x) + (a.x - c.x) * (z - c.z)) / d;
        let w = 1. - u - v;
        if u < 0. || v < 0. || w < 0. { continue; }
        let y = u * a.y + v * b.y + w * c.y;
        // The layer lying over her, not one under her: from just below her top surface up.
        if y > below && best.map_or(true, |b| y < b) { best = Some(y); }
    }
    best
}

#[test]
#[ignore = "diagnostic: release, --nocapture"]
fn she_lies_under_the_covers_and_they_conform() {
    let package = ScenePackage::load(&std::path::Path::new(ROOT).join("scene-bedding-two-covers-settled.json")).unwrap();
    let layer = |id: &str| package.cloths.iter().position(|c| c.id == id).unwrap();
    let (top, duvet) = (layer("bed_top_sheet"), layer("bed_duvet"));
    let mut solvers: Vec<ClothSolver> = package.cloths.iter().map(|c| c.solver().unwrap()).collect();
    let colliders: Vec<_> = package.cloths.iter().map(|c| c.colliders(&package).unwrap()).collect();
    let triangles: Vec<Vec<[u32; 3]>> = package.cloths.iter().map(|c| c.loaded.as_ref().unwrap().triangles.clone()).collect();
    let radii: Vec<f32> = package.cloths.iter().map(|c| c.parameters.thickness_m * 0.5).collect();
    let Some(mut batch) = GpuClothBatch::new(&solvers.iter().collect::<Vec<_>>()).unwrap() else {
        println!("GPU_SKIPPED no suitable adapter");
        return;
    };
    batch.start_at_rest();
    let body = body();
    let resting_body = kinematic(&body, (0., 0.));
    println!("UNDER_COVERS_BODY {}", serde_json::json!({
        "skinned_points": body.points.len(),
        "extent": [body.points.iter().fold(Vec3::splat(f32::MAX), |m, p| m.min(*p)).to_array(),
                   body.points.iter().fold(Vec3::splat(f32::MIN), |m, p| m.max(*p)).to_array()],
        "capsules": body.capsules.iter().map(|(a, b, r, g)| serde_json::json!({"a": a.to_array(), "b": b.to_array(), "r": r, "region": g})).collect::<Vec<_>>(),
    }));

    let clamp = ClampMaterial { compliance_m_n: 0.0002, damping_ratio: 1.0, capacity_n: knob("CAPACITY", 40.) as f64 };
    let ball = Ball::new(turndown::HAND_RADIUS_M);
    // Two hands at the duvet's bedside edge, near the head and near the foot: the top-facing
    // duvet face on the mattress furthest towards her side within 12 cm of each.
    let edge_site = |solver: &ClothSolver, z0: f32| -> usize {
        let positions = solver.surface().positions();
        (0..triangles[duvet].len()).filter(|&i| {
            let [a, b, c] = triangles[duvet][i].map(|k| positions[k as usize]);
            let centre = (a + b + c) / 3.;
            centre.y > turndown::MATTRESS_TOP_Y && (b - a).cross(c - a).normalize().y.abs() > 0.7 && (centre.z - z0).abs() < 0.12
        }).max_by(|&i, &j| {
            let x = |t: usize| triangles[duvet][t].iter().map(|k| positions[*k as usize].x).sum::<f32>();
            x(i).total_cmp(&x(j))
        }).expect("a duvet face at the bedside edge")
    };
    // The pose of a ball resting on a face's upward side.
    let on_face = |solver: &ClothSolver, t: usize| -> Isometry<f32> {
        let [a, b, c] = triangles[duvet][t].map(|k| solver.surface().positions()[k as usize]);
        let mut n = (b - a).cross(c - a).normalize();
        if n.y < 0. { n = -n; }
        let p = (a + b + c) / 3. + n * turndown::HAND_RADIUS_M;
        Isometry::translation(p.x, p.y, p.z)
    };
    let faces = [edge_site(&solvers[duvet], -3.7), edge_site(&solvers[duvet], -2.6)];
    let homes: Vec<Vec3> = faces.iter().map(|&t| Vec3::from(on_face(&solvers[duvet], t).translation.vector)).collect();
    let take = |solvers: &[ClothSolver]| -> Vec<ClothGrip> {
        faces.iter().map(|&t| ClothGrip::capture(solvers[duvet].surface(), &ball, &on_face(&solvers[duvet], t), t, clamp)
            .unwrap().expect("grip")).collect()
    };
    let mut grips: Vec<Vec<ClothGrip>> = vec![Vec::new(); solvers.len()];
    grips[duvet] = take(&solvers);

    let mut frames_out = std::env::var("CC_COVERS_FRAMES").ok()
        .map(|path| std::io::BufWriter::new(std::fs::File::create(path).unwrap()));
    let mut gpu_ms = Vec::new();
    let mut flags = (0u64, 0u64);
    let clock = std::time::Instant::now();
    let mut step = |solvers: &mut Vec<ClothSolver>, grips: &mut Vec<Vec<ClothGrip>>, hands: Option<&[Vec3]>, bodies: Option<&[KinematicCollider]>, frame: u32| {
        let lying = bodies.is_some();
        if let Some(hands) = hands {
            for (g, h) in grips[duvet].iter_mut().zip(hands) { g.observe_pose(&Isometry::translation(h.x, h.y, h.z)).unwrap(); }
        }
        let mut layers: Vec<ClothLayerStep> = solvers.iter_mut().zip(&colliders).zip(grips.iter_mut())
            .map(|((solver, colliders), grips)| ClothLayerStep { solver, colliders, grips }).collect();
        let none: [KinematicCollider; 0] = [];
        batch.step_with_kinematic(&mut layers, turndown::DT, -DVec3::Y * 9.81, bodies.unwrap_or(&none))
            .unwrap_or_else(|e| panic!("frame {frame}: {e} {:?}", batch.last));
        gpu_ms.push(batch.last.frame_ms);
        flags.0 += batch.last.substep_motion_exceeded as u64;
        flags.1 += batch.last.margin_exceeded as u64;
        if let Some(file) = frames_out.as_mut().filter(|_| frame % 2 == 0) {
            use std::io::Write;
            let layers: Vec<Vec<[f32; 3]>> = solvers.iter().map(|s| s.surface().positions().iter()
                .map(|p| p.to_array().map(|x| (x * 1000.).round() / 1000.)).collect()).collect();
            let held = grips[duvet].iter().any(|g| g.state() == super::cloth_grip::ClampState::Held);
            writeln!(file, "{}", serde_json::json!({"frame": frame, "held": hands.is_some() && held,
                "hand": hands.map_or(Vec3::ZERO, |h| h[0]).to_array(),
                "hands": hands.map_or(vec![], |h| h.iter().map(|p| p.to_array()).collect::<Vec<_>>()),
                "layers": layers, "lying": lying})).unwrap();
        }
        batch.resting()
    };
    let ease = |s: f32| { let s = s.clamp(0., 1.); s * s * (3. - 2. * s) };
    // A hand move: up by `lift` (0.4 s), across to `to` at that height (`sweep` s), down to the
    // height `lay(to)` gives once it gets there (0.4 s), hold 0.3 s, let go (0.25 s).
    let stretch = |solvers: &[ClothSolver]| solvers.iter().map(|s| s.max_edge_stretch_ratio()).fold(0., f64::max);
    let mut frame = 0u32;
    type Step<'s> = dyn FnMut(&mut Vec<ClothSolver>, &mut Vec<Vec<ClothGrip>>, Option<&[Vec3]>, Option<&[KinematicCollider]>, u32) -> bool + 's;
    let carry = |step: &mut Step, solvers: &mut Vec<ClothSolver>, grips: &mut Vec<Vec<ClothGrip>>, to: &[Vec3], lift: f32, sweep: f32,
                 bodies: Option<&[KinematicCollider]>, frame: &mut u32, label: &str| {
        let from: Vec<Vec3> = grips[duvet].iter().map(|g| g.anchor(1.).as_vec3()).collect();
        let up: Vec<Vec3> = from.iter().map(|p| *p + Vec3::Y * lift).collect();
        let over: Vec<Vec3> = to.iter().zip(&up).map(|(t, u)| Vec3::new(t.x, u.y, t.z)).collect();
        let mut down: Option<Vec<Vec3>> = None;
        let total = ((0.4 + sweep + 0.4 + 0.3) * 60.) as u32;
        let mut peak = 0f64;
        for k in 1..=total + 20 {
            *frame += 1;
            let t = k as f32 / 60.;
            if down.is_none() && t >= 0.4 + sweep {
                // Lay each gripped point on the highest of the bed's cloth there and her
                // skinned surface within 8 cm (when she is in bed).
                down = Some(to.iter().zip(&grips[duvet]).map(|(p, g)| {
                    let gripped = solvers[duvet].surface().resolve(g.material_point()).unwrap().position;
                    let cloth = turndown::laying_hand_y(solvers, &radii, (duvet, gripped), [p.x, p.z]);
                    let her = if bodies.is_some() {
                        body.points.iter().filter(|q| (q.x - p.x).hypot(q.z - p.z) < 0.08)
                            .map(|q| q.y + radii[duvet] + turndown::LAID_CLEARANCE_M + turndown::HAND_RADIUS_M).fold(f32::MIN, f32::max)
                    } else { f32::MIN };
                    Vec3::new(p.x, cloth.max(her), p.z)
                }).collect());
            }
            let hands: Vec<Vec3> = (0..from.len()).map(|i| {
                if t < 0.4 { from[i].lerp(up[i], ease(t / 0.4)) }
                else if t < 0.4 + sweep { up[i].lerp(over[i], ease((t - 0.4) / sweep)) }
                else { over[i].lerp(down.as_ref().unwrap()[i], ease((t - 0.4 - sweep) / 0.4)) }
            }).collect();
            if k == total { for g in grips[duvet].iter_mut() { g.let_go(turndown::LET_GO_S); } }
            step(solvers, grips, Some(&hands), bodies, *frame);
            peak = grips[duvet].iter().map(|g| g.peak_required_force_n()).fold(peak, f64::max);
        }
        println!("UNDER_COVERS_PHASE {label}: grips {:?}, peak {:.1} N, stretch {:.3}",
            grips[duvet].iter().map(|g| g.state()).collect::<Vec<_>>(), peak, stretch(solvers));
    };

    // 1. Fold the duvet back off her half: both hands carry its bedside edge over to the far
    // side (mirrored across the bed's centre line), lay it there and let go; the bed settles.
    let across: Vec<Vec3> = homes.iter().map(|h| Vec3::new(4.2 - 0.95 * (h.x - 4.2), h.y, h.z)).collect();
    carry(&mut step, &mut solvers, &mut grips, &across, 0.35, 1.2, None, &mut frame, "folded back");
    for _ in 0..90 { frame += 1; step(&mut solvers, &mut grips, None, None, frame); }

    // 2. She lies down on her half: her body comes down from 0.3 m above its lying place over
    // 1 s (eased), then lies still while the bed settles around her.
    let height = |k: u32| 0.3 * (1. - ease(k as f32 / 60.));
    for k in 0..90 {
        frame += 1;
        let moving = kinematic(&body, (height(k), height(k + 1)));
        step(&mut solvers, &mut grips, None, Some(&moving), frame);
    }
    println!("UNDER_COVERS_PHASE lying: stretch {:.3}", stretch(&solvers));

    // 3. Both hands take the same edge again and draw the duvet back over her: up, across to
    // where the edge lay before, down onto what they find there (her or the bed), let go.
    grips[duvet] = take(&solvers);
    carry(&mut step, &mut solvers, &mut grips, &homes, knob("LIFT", 0.5), knob("SWEEP_S", 1.5), Some(&resting_body), &mut frame, "drawn back");

    // 4. Settle: the batch rests once it has stopped (or 1.5 s after the last movement).
    let mut rest_frame = None;
    for k in 1..=240 {
        frame += 1;
        if step(&mut solvers, &mut grips, None, Some(&resting_body), frame) && rest_frame.is_none() { rest_frame = Some(k); }
    }

    // Her top surface on a 2 cm grid (the highest skinned point in each cell), by region; the
    // duvet over each cell, gap from her surface to the duvet's underside.
    let mut cells: std::collections::HashMap<(i32, i32), (f32, Region)> = Default::default();
    for (p, r) in body.points.iter().zip(&body.region) {
        let key = ((p.x / 0.02).floor() as i32, (p.z / 0.02).floor() as i32);
        let e = cells.entry(key).or_insert((f32::MIN, *r));
        if p.y > e.0 { *e = (p.y, *r); }
    }
    let positions = solvers[duvet].surface().positions();
    let mut by_region: std::collections::BTreeMap<String, (usize, usize, Vec<f32>)> = Default::default();
    for (&(i, j), &(y, region)) in &cells {
        let (x, z) = ((i as f32 + 0.5) * 0.02, (j as f32 + 0.5) * 0.02);
        let entry = by_region.entry(format!("{region:?}")).or_default();
        entry.0 += 1;
        if let Some(d) = duvet_over(positions, &triangles[duvet], x, z, y - 0.05) {
            entry.1 += 1;
            entry.2.push(d - radii[duvet] - y);
        }
    }
    let report_regions: serde_json::Map<String, serde_json::Value> = by_region.into_iter().map(|(k, (n, covered, gaps))| {
        let sunk = gaps.iter().filter(|g| **g < -0.005).count();
        (k, serde_json::json!({"cells": n, "covered": covered as f64 / n as f64,
            "gap_median_m": quantile(gaps.clone(), 0.5), "gap_p90_m": quantile(gaps.clone(), 0.9),
            "sunk_in_over_5mm": sunk as f64 / covered.max(1) as f64, "deepest_m": -quantile(gaps, 0.0)}))
    }).collect();
    let capsule_depth = |p: Vec3, r: f32| body.capsules.iter().map(|(a, b, cr, _)| cr + r - p.distance(closest_on_segment(p, *a, *b))).fold(0f32, f32::max);
    gpu_ms.sort_by(f64::total_cmp);
    let capsule_penetration = [top, duvet].map(|i| solvers[i].surface().positions().iter()
        .map(|p| capsule_depth(*p, radii[i])).fold(0f32, f32::max));
    let report = serde_json::json!({
        "regions": report_regions,
        "rest_frame_after_let_go": rest_frame,
        "max_capsule_penetration_m": capsule_penetration,
        "max_edge_stretch_ratio": solvers.iter().map(|s| s.max_edge_stretch_ratio()).fold(0., f64::max),
        "max_particle_penetration_m": solvers.iter().zip(&colliders).map(|(s, c)| s.max_particle_penetration(c)).fold(0., f64::max),
        "frame_ms_median": gpu_ms[gpu_ms.len() / 2], "frame_ms_max": gpu_ms[gpu_ms.len() - 1],
        "tunnel_flags": [flags.0, flags.1], "wall_s": clock.elapsed().as_secs_f64(),
    });
    println!("UNDER_COVERS_REPORT {report}");
    // Under the covers: her legs and feet covered, most of her torso, not her head; the duvet
    // follows her (median gap from her surface to its underside within 6 cm, bridging between
    // legs and beside arms) without sinking into her (at most 15% of covered cells more than
    // 5 mm in: she is capsules to the cloth, skin to this measure), and it rests.
    let region = |name: &str, key: &str| report["regions"][name][key].as_f64().unwrap();
    assert!(region("Legs", "covered") >= 0.9 && region("Feet", "covered") >= 0.9, "{report}");
    assert!(region("Torso", "covered") >= 0.5, "{report}");
    assert!(region("Head", "covered") <= 0.2, "{report}");
    for name in ["Legs", "Feet", "Torso", "Arms"] {
        if region(name, "covered") > 0. {
            assert!(region(name, "gap_median_m") <= 0.06, "{name}: {report}");
            assert!(region(name, "sunk_in_over_5mm") <= 0.15, "{name}: {report}");
        }
    }
    assert!(capsule_penetration.iter().all(|d| *d < 1e-6), "{report}");
    assert!(report["max_edge_stretch_ratio"].as_f64().unwrap() < 1.1, "{report}");
    assert!(rest_frame.is_some(), "{report}");
    assert!(report["frame_ms_median"].as_f64().unwrap() <= 8., "{report}");
}
