use super::*;
use crate::cc_render::cloth_solver::{ClothParameters, ClothSolver};
use bevy::math::{Vec2, Vec3};
use bevy_rapier3d::rapier::parry::shape::Ball;

const DT: f64 = 1.0 / 60.0;

fn material(capacity_n: f64) -> ClampMaterial {
    ClampMaterial {
        compliance_m_n: 0.001,
        damping_ratio: 1.0,
        capacity_n,
    }
}

fn triangle() -> ClothSolver {
    let positions = vec![
        Vec3::new(-0.3, 0., -0.2),
        Vec3::new(0.3, 0., -0.2),
        Vec3::new(0., 0., 0.4),
    ];
    ClothSolver::new(
        positions.clone(),
        positions,
        vec![[0, 2, 1]],
        vec![Vec2::ZERO; 3],
        ClothParameters {
            total_mass_kg: 0.8,
            thickness_m: 0.001,
            stretch_compliance: 1e-7,
            bend_compliance: 0.05,
            air_drag_per_second: 0.2,
            spring_damping_ratio: 1.0,
            substeps: 24,
        },
    )
    .unwrap()
}

fn capture(cloth: &ClothSolver, capacity: f64) -> ClothGrip {
    ClothGrip::capture(
        cloth.surface(),
        &Ball::new(0.01),
        &Isometry::translation(0., 0.01, 0.),
        0,
        material(capacity),
    )
    .unwrap()
    .unwrap()
}

fn tick(cloth: &mut ClothSolver, grip: &mut ClothGrip) {
    cloth
        .step_with_grips(DT, -DVec3::Y * 9.81, &[], std::slice::from_mut(grip))
        .unwrap();
}

#[test]
fn clamp_capture_requeries_actual_shape_and_does_not_move_sheet() {
    let cloth = triangle();
    let before = cloth.surface().positions().to_vec();
    for pose in [
        Isometry::translation(0., 0.011, 0.),
        Isometry::translation(2., 0.01, 0.),
    ] {
        assert!(
            ClothGrip::capture(cloth.surface(), &Ball::new(0.01), &pose, 0, material(20.))
                .unwrap()
                .is_none()
        );
    }
    assert!(ClothGrip::capture(
        cloth.surface(),
        &Ball::new(0.01),
        &Isometry::translation(0., 0.01, 0.),
        1,
        material(20.)
    )
    .unwrap()
    .is_none());
    let grip = capture(&cloth, 20.);
    assert_eq!(cloth.surface().positions(), before);
    assert_eq!(cloth.surface().revision(), 0);
    assert!(
        grip.anchor(1.).distance(
            cloth
                .surface()
                .resolve(grip.material_point())
                .unwrap()
                .position
                .as_dvec3()
        ) < 1e-8
    );
}

#[test]
fn compliant_clamp_balances_known_weight_without_a_rigid_pin() {
    let mut cloth = triangle();
    let mut grip = capture(&cloth, 20.);
    let initial = grip.anchor(1.);
    for _ in 0..240 {
        tick(&mut cloth, &mut grip);
    }
    let point = cloth
        .surface()
        .resolve(grip.material_point())
        .unwrap()
        .position
        .as_dvec3();
    let expected_weight = 0.8 * 9.81;
    assert_eq!(grip.state(), ClampState::Held);
    assert!((grip.peak_applied_force_n() - expected_weight).abs() < 0.01);
    assert!((initial.y - point.y - material(20.).compliance_m_n * expected_weight).abs() < 2e-5);
    assert!(cloth.max_speed() < 0.002);
    grip.release();
    let released = point.y;
    for _ in 0..30 {
        tick(&mut cloth, &mut grip);
    }
    assert_eq!(grip.state(), ClampState::Released);
    assert_eq!(grip.peak_applied_force_n(), 0.0);
    assert!(
        cloth
            .surface()
            .resolve(grip.material_point())
            .unwrap()
            .position
            .y
            < released as f32 - 1.
    );
}

#[test]
fn overload_breaks_before_excess_force_and_never_reacquires() {
    let mut cloth = triangle();
    let mut grip = capture(&cloth, 0.5);
    let mut rejected_force = 0.0_f64;
    let mut applied = 0.0_f64;
    for _ in 0..30 {
        tick(&mut cloth, &mut grip);
        rejected_force = rejected_force.max(grip.peak_required_force_n());
        applied = applied.max(grip.peak_applied_force_n());
    }
    assert_eq!(grip.state(), ClampState::Overloaded);
    assert!(rejected_force > 0.5);
    assert!(applied <= 0.5);
    assert!(cloth.surface().positions().iter().all(|p| p.y < -1.));
    grip.observe_pose(&Isometry::translation(0., -10., 0.))
        .unwrap();
    grip.release();
    tick(&mut cloth, &mut grip);
    assert_eq!(grip.state(), ClampState::Overloaded);
    assert_eq!(grip.peak_applied_force_n(), 0.0);
}

#[test]
fn foreign_surface_invalid_material_and_failed_steps_are_atomic() {
    let mut first = triangle();
    let second = triangle();
    let mut foreign = capture(&second, 20.);
    let before = first.surface().positions().to_vec();
    assert!(first
        .step_with_grips(DT, DVec3::ZERO, &[], std::slice::from_mut(&mut foreign))
        .is_err());
    let mut grip = capture(&first, 20.);
    let anchor = grip.anchor(1.);
    assert!(grip
        .observe_pose(&Isometry::translation(f32::NAN, 0., 0.))
        .is_err());
    assert_eq!(grip.anchor(1.), anchor);
    assert!(first
        .step_with_grips(0., DVec3::ZERO, &[], std::slice::from_mut(&mut grip))
        .is_err());
    assert_eq!(first.surface().positions(), before);
    assert_eq!(first.surface().revision(), 0);
    assert_eq!(grip.state(), ClampState::Held);
    for invalid in [
        ClampMaterial {
            capacity_n: 0.,
            ..material(20.)
        },
        ClampMaterial {
            compliance_m_n: 0.,
            ..material(20.)
        },
        ClampMaterial {
            damping_ratio: f64::NAN,
            ..material(20.)
        },
    ] {
        assert!(ClothGrip::capture(
            first.surface(),
            &Ball::new(0.01),
            &Isometry::translation(0., 0.01, 0.),
            0,
            invalid
        )
        .is_err());
    }
}

#[test]
fn acquired_frame_follows_collider_rotation_and_release_preserves_momentum() {
    let mut a = triangle();
    let mut b = triangle();
    let mut ga = capture(&a, 100.);
    let mut gb = capture(&b, 100.);
    for frame in 1..=60 {
        let pose = Isometry::translation(0., 0.01 + frame as f32 * 0.002, 0.);
        ga.observe_pose(&pose).unwrap();
        gb.observe_pose(&pose).unwrap();
        tick(&mut a, &mut ga);
        tick(&mut b, &mut gb);
    }
    assert_eq!(ga.state(), ClampState::Held);
    assert_eq!(a.surface().positions(), b.surface().positions());
    ga.release();
    tick(&mut a, &mut ga);
    b.step(DT, -DVec3::Y * 9.81, &[]).unwrap();
    assert_eq!(a.surface().positions(), b.surface().positions());
    assert_eq!(a.max_speed(), b.max_speed());
    let rotated = Isometry::new(
        bevy_rapier3d::rapier::na::Vector3::new(1., 2., 3.),
        bevy_rapier3d::rapier::na::Vector3::new(0., 0., std::f32::consts::FRAC_PI_2),
    );
    gb.observe_pose(&rotated).unwrap();
    // Captured point is 1 cm below the collider centre, so a quarter-turn
    // carries it to the right; no canonical centre or wrist snap is used.
    assert!(gb.anchor(1.).distance(DVec3::new(1.01, 2., 3.)) < 1e-6);
}

#[test]
fn two_clamps_release_independently_and_numerical_failure_rolls_back_both() {
    let mut cloth = triangle();
    let mut grips = [capture(&cloth, 20.), capture(&cloth, 20.)];
    for _ in 0..120 {
        cloth
            .step_with_grips(DT, -DVec3::Y * 9.81, &[], &mut grips)
            .unwrap();
    }
    grips[0].release();
    for _ in 0..120 {
        cloth
            .step_with_grips(DT, -DVec3::Y * 9.81, &[], &mut grips)
            .unwrap();
    }
    assert_eq!(grips[0].state(), ClampState::Released);
    assert_eq!(grips[1].state(), ClampState::Held);
    assert!((grips[1].peak_applied_force_n() - 0.8 * 9.81).abs() < 0.01);
    let before = cloth.surface().positions().to_vec();
    let revision = cloth.surface().revision();
    let force = grips[1].peak_applied_force_n();
    assert!(cloth
        .step_with_grips(DT, DVec3::splat(1e300), &[], &mut grips)
        .is_err());
    assert_eq!(cloth.surface().positions(), before);
    assert_eq!(cloth.surface().revision(), revision);
    assert_eq!(grips[1].state(), ClampState::Held);
    assert_eq!(grips[1].peak_applied_force_n(), force);
}

#[test]
fn condo_sheet_contact_clamp_lifts_releases_and_settles() {
    use crate::cc_render::scene_package::ScenePackage;
    let package = ScenePackage::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo/scene-cloth.json"),
    )
    .unwrap();
    let mut cloth = package.cloths[0].solver().unwrap();
    let colliders = package.cloths[0].colliders(&package).unwrap();
    for _ in 0..1200 {
        cloth.step(DT, -DVec3::Y * 9.81, &colliders).unwrap();
    }
    // Diagnostic tool fixture only. This chosen foot-edge material face and
    // tool trajectory never enter the avatar/production command paths.
    let face = package.cloths[0].loaded.as_ref().unwrap().triangles[32];
    let vertices = face.map(|i| cloth.surface().positions()[i as usize]);
    let center = vertices.iter().copied().sum::<Vec3>() / 3.;
    let normal = (vertices[1] - vertices[0])
        .cross(vertices[2] - vertices[0])
        .normalize();
    let position = center + normal * 0.01;
    let mut pose = Isometry::translation(position.x, position.y, position.z);
    let config = ClampMaterial {
        compliance_m_n: 0.0002,
        ..material(25.)
    };
    let mut grip = ClothGrip::capture(cloth.surface(), &Ball::new(0.01), &pose, 32, config)
        .unwrap()
        .unwrap();
    let mut records = Vec::new();
    let mut snapshots = Vec::new();
    for frame in 0..=780 {
        if frame > 0 {
            let fraction = (frame as f32 / 120.).min(1.);
            pose.translation.vector.y = position.y + 0.4 * fraction;
            grip.observe_pose(&pose).unwrap();
            if frame == 181 {
                grip.release();
            }
            cloth
                .step_with_grips(
                    DT,
                    -DVec3::Y * 9.81,
                    &colliders,
                    std::slice::from_mut(&mut grip),
                )
                .unwrap();
        }
        records.push(grip_record(&cloth, &grip, &colliders, frame));
        if frame % 6 == 0 {
            snapshots.push(serde_json::json!({"frame":frame,
                "positions":cloth.surface().positions().iter().map(|p| p.to_array()).collect::<Vec<_>>() }));
        }
    }
    check_condo_trace(records, snapshots, config);
}

fn grip_record(
    cloth: &ClothSolver,
    grip: &ClothGrip,
    colliders: &[crate::cc_render::cloth_solver::FixedClothCollider],
    frame: u32,
) -> serde_json::Value {
    let point = cloth
        .surface()
        .resolve(grip.material_point())
        .unwrap()
        .position
        .as_dvec3();
    serde_json::json!({"frame":frame,"state":grip.state(),"point":point.to_array(),
        "anchor":grip.anchor(1.).to_array(),"compliance_error_m":point.distance(grip.anchor(1.)),
        "required_force_n":grip.peak_required_force_n(),"applied_force_n":grip.peak_applied_force_n(),
        "max_speed_m_s":cloth.max_speed(),"max_edge_stretch_ratio":cloth.max_edge_stretch_ratio(),
        "max_particle_penetration_m":cloth.max_particle_penetration(colliders)})
}

fn check_condo_trace(
    records: Vec<serde_json::Value>,
    snapshots: Vec<serde_json::Value>,
    config: ClampMaterial,
) {
    let max = |rows: &[serde_json::Value], key: &str| {
        rows.iter()
            .map(|r| r[key].as_f64().unwrap())
            .fold(0.0, f64::max)
    };
    let y = |frame: usize| records[frame]["point"][1].as_f64().unwrap();
    let report = serde_json::json!({
        "scope":"CPU solver with a diagnostic kinematic contact clamp; no avatar or runtime hand admission; no self/swept/dynamic body collision",
        "coordinates":"runtime metres X east, Y up, Z south", "pre_settle_seconds":20.,
        "simulation_seconds":13., "clamp_material":config, "triangle":32,
        "lift_m":y(180)-y(0), "released_drop_m":y(180)-y(780),
        "held_until_release":records[..181].iter().all(|r| r["state"] == "held"),
        "released_after_command":records[181..].iter().all(|r| r["state"] == "released"),
        "max_required_force_n":max(&records,"required_force_n"),
        "max_applied_force_n":max(&records,"applied_force_n"),
        "max_held_compliance_error_m":max(&records[..181],"compliance_error_m"),
        "max_edge_stretch_ratio":max(&records,"max_edge_stretch_ratio"),
        "max_particle_penetration_m":max(&records,"max_particle_penetration_m"),
        "tail_max_speed_m_s":max(&records[751..],"max_speed_m_s")
    });
    println!("CLOTH_GRIP_REPORT {report}");
    if let Ok(path) = std::env::var("CC_CLOTH_GRIP_TRACE") {
        let path = std::path::Path::new(&path);
        std::fs::create_dir(path).unwrap();
        for (file, value) in [
            ("report.json", report.clone()),
            ("positions.json", serde_json::json!(snapshots)),
            ("observations.json", serde_json::json!(records)),
        ] {
            std::fs::write(path.join(file), serde_json::to_vec(&value).unwrap()).unwrap();
        }
    }
    assert_eq!(report["held_until_release"], true, "{report}");
    assert_eq!(report["released_after_command"], true, "{report}");
    assert!(report["lift_m"].as_f64().unwrap() > 0.38, "{report}");
    assert!(
        report["released_drop_m"].as_f64().unwrap() > 0.3,
        "{report}"
    );
    assert!(
        report["max_edge_stretch_ratio"].as_f64().unwrap() < 1.1,
        "{report}"
    );
    assert!(
        report["max_particle_penetration_m"].as_f64().unwrap() < 0.001,
        "{report}"
    );
    assert!(
        report["tail_max_speed_m_s"].as_f64().unwrap() < 0.05,
        "{report}"
    );
}
