use super::*;
use serde::Deserialize;

fn parameters() -> ClothParameters {
    ClothParameters {
        total_mass_kg: 0.8,
        thickness_m: 0.001,
        stretch_compliance: 1e-7,
        bend_compliance: 0.05,
        air_drag_per_second: 0.2,
        spring_damping_ratio: 1.0,
        substeps: 24,
    }
}

fn patch(offset: Vec3) -> ClothSolver {
    let rest = vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::X + Vec3::Z];
    let initial = rest.iter().map(|v| *v + offset).collect();
    ClothSolver::new(
        rest,
        initial,
        vec![[0, 2, 1], [1, 2, 3]],
        vec![Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE],
        parameters(),
    )
    .unwrap()
}

#[test]
fn fast_sheet_impact_stays_on_entry_side_then_settles_without_a_velocity_override() {
    let mut cloth = patch(Vec3::Y * 0.1);
    cloth.velocities.fill(-DVec3::Y * 100.);
    let support = FixedClothCollider::new(
        SharedShape::cuboid(2., 0.005, 2.),
        Isometry::translation(0.5, 0., 0.5),
        0.6,
    )
    .unwrap();
    for _ in 0..60 {
        cloth
            .step(1. / 60., -DVec3::Y * 9.81, std::slice::from_ref(&support))
            .unwrap();
        assert!(
            cloth.positions.iter().all(|p| p.y >= 0.00549),
            "fast sheet crossed support"
        );
    }
    assert!(cloth.max_speed() < 0.001);
    assert_eq!(cloth.surface.revision(), 60);
}

#[test]
fn free_fall_matches_semi_implicit_gravity_without_pins() {
    let mut cloth = patch(Vec3::Y);
    cloth.parameters.air_drag_per_second = 0.0;
    let before = cloth.positions.clone();
    let h = 1.0 / 60.0 / cloth.parameters.substeps as f64;
    let n = 60 * cloth.parameters.substeps;
    for _ in 0..60 {
        cloth
            .step(1.0 / 60.0, DVec3::new(0., -9.81, 0.), &[])
            .unwrap();
    }
    let expected = -9.81 * h * h * (n * (n + 1)) as f64 * 0.5;
    for (p, old) in cloth.positions.iter().zip(before) {
        assert!((*p - old - DVec3::Y * expected).length() < 1e-9);
    }
    assert!((cloth.max_speed() - 9.81).abs() < 1e-8);
    assert_eq!(cloth.surface.revision(), 60);
}

#[test]
fn internal_springs_conserve_mass_weighted_centre_without_external_force() {
    let mut cloth = patch(Vec3::ZERO);
    cloth.positions[3].y += 0.2;
    let centre = |c: &ClothSolver| {
        c.positions
            .iter()
            .zip(&c.inverse_mass)
            .map(|(p, w)| *p / *w)
            .sum::<DVec3>()
            / c.parameters.total_mass_kg
    };
    let before = centre(&cloth);
    let mass: f64 = cloth.inverse_mass.iter().map(|w| 1.0 / w).sum();
    assert!((mass - 0.8).abs() < 1e-12);
    for _ in 0..30 {
        cloth.step(1.0 / 60.0, DVec3::ZERO, &[]).unwrap();
    }
    assert!((centre(&cloth) - before).length() < 1e-10);
}

#[test]
fn invalid_steps_and_topology_leave_published_state_unchanged() {
    let mut cloth = patch(Vec3::Y);
    let positions = cloth.positions.clone();
    for dt in [0.0, -0.1, f64::NAN, 1.0] {
        assert!(cloth.step(dt, DVec3::ZERO, &[]).is_err());
        assert_eq!(cloth.positions, positions);
        assert_eq!(cloth.surface.revision(), 0);
    }
    assert!(cloth.step(1.0 / 60.0, DVec3::splat(f64::NAN), &[]).is_err());
    let mut p = parameters();
    p.total_mass_kg = 0.0;
    assert!(p.validate().is_err());
    assert!(FixedClothCollider::new(SharedShape::ball(-0.01), Isometry::identity(), 0.5).is_err());
    let rest = vec![Vec3::ZERO, Vec3::X, Vec3::Z];
    assert!(ClothSolver::new(
        rest.clone(),
        rest,
        vec![[0, 1, 2], [0, 1, 2]],
        vec![Vec2::ZERO; 3],
        parameters()
    )
    .is_err());
}

#[test]
fn collider_support_is_local_and_removal_releases_the_sheet() {
    let mut cloth = patch(Vec3::Y * 0.1);
    let support = FixedClothCollider::new(
        SharedShape::cuboid(2., 0.1, 2.),
        Isometry::translation(0.5, -0.1, 0.5),
        0.6,
    )
    .unwrap();
    for _ in 0..120 {
        cloth
            .step(1.0 / 60.0, -DVec3::Y * 9.81, std::slice::from_ref(&support))
            .unwrap();
    }
    assert!(cloth.positions.iter().all(|p| (p.y - 0.0005).abs() < 2e-6));
    assert!(cloth.max_speed() < 0.001);
    let before = cloth.positions.clone();
    for _ in 0..30 {
        cloth.step(1.0 / 60.0, -DVec3::Y * 9.81, &[]).unwrap();
    }
    assert!(cloth
        .positions
        .iter()
        .zip(before)
        .all(|(p, old)| p.y < old.y - 1.0));
}

#[derive(Deserialize)]
struct SheetFixture {
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    material_uv: Vec<[f32; 2]>,
    edges: Vec<[usize; 2]>,
    rest_edge_lengths_m: Vec<f64>,
}

#[test]
fn condo_sheet_gravity_and_settling_probe() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/interaction-authoring/bedding/top-sheet-surface.json");
    let fixture: SheetFixture = serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
    let rest: Vec<_> = fixture.positions.iter().copied().map(Vec3::from).collect();
    let mut cloth = ClothSolver::new(
        rest.clone(),
        rest,
        fixture.triangles.clone(),
        fixture
            .material_uv
            .iter()
            .copied()
            .map(Vec2::from)
            .collect(),
        parameters(),
    )
    .unwrap();
    // Explicit rounded-box approximation of the authored mattress (1.8 x .28 x 2.12).
    // NOT the old aggregate bed collider, nor a claim of triangle-mesh collision.
    let mattress = FixedClothCollider::new(
        SharedShape::round_cuboid(0.87, 0.11, 1.03, 0.03),
        Isometry::translation(4.2, 0.48, -3.4),
        0.7,
    )
    .unwrap();
    let floor = FixedClothCollider::new(
        SharedShape::cuboid(20., 0.1, 20.),
        Isometry::translation(0., -0.1, 0.),
        0.7,
    )
    .unwrap();
    let colliders = [mattress, floor];
    let mut records = Vec::new();
    let mut snapshots = Vec::new();
    let start = std::time::Instant::now();
    for frame in 0..=1200 {
        if frame > 0 {
            cloth
                .step(1.0 / 60.0, -DVec3::Y * 9.81, &colliders)
                .unwrap();
        }
        records.push(fixture.observe(&cloth, &colliders[0], frame));
        if frame % 6 == 0 {
            snapshots.push(serde_json::json!({"frame":frame,
            "positions":cloth.surface.positions().iter().map(|v|v.to_array()).collect::<Vec<_>>() }));
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let tail = &records[1171..];
    let max = |rows: &[serde_json::Value], field: &str| {
        rows.iter()
            .map(|r| r[field].as_f64().unwrap())
            .fold(0.0, f64::max)
    };
    let support = tail
        .iter()
        .map(|r| r["support_fraction"].as_f64().unwrap())
        .fold(1.0, f64::min);
    let report = serde_json::json!({"scope":"CPU solver prototype; static rounded mattress, no self-collision, hand, dynamic body or live renderer integration",
        "parameters":cloth.parameters,"coordinates":"runtime metres X east, Y up, Z south","frames":1200,"simulation_seconds":20.0,"wall_seconds_including_measurements":elapsed,
        "tail_max_speed_m_s":max(tail,"max_speed_m_s"),"tail_support_fraction":support,
        "max_edge_stretch_ratio":max(&records,"max_edge_stretch_ratio"),
        "max_particle_penetration_m":max(&records,"max_particle_penetration_m")});
    println!("CLOTH_SOLVER_REPORT {report}");
    write_trace(&report, records, snapshots);
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
    assert!(support > 0.4, "{report}");
}

#[test]
fn rounded_box_distance_and_normals_are_consistent_under_rotation() {
    let shape = SharedShape::round_cuboid(0.87, 0.11, 1.03, 0.03);
    let pose = Isometry::translation(4.2, 0.48, -3.4);
    let collider = FixedClothCollider::new(shape, pose, 0.7).unwrap();
    for x in 0..33 {
        for z in 0..37 {
            let p = DVec3::new(
                3.34 + 1.72 * x as f64 / 32.0,
                0.6182,
                -2.38 - 2.04 * z as f64 / 36.0,
            );
            let (distance, normal) = collider.distance_and_normal(p);
            assert!((distance + 0.0018).abs() < 1e-7);
            assert!((normal - DVec3::Y).length() < 1e-12);
        }
    }
    let rotated = FixedClothCollider {
        rotation: DQuat::from_rotation_z(0.7),
        ..collider
    };
    for local in [
        DVec3::ZERO,
        DVec3::new(0.9, 0.14, 1.06),
        DVec3::new(-0.88, 0.12, -1.04),
    ] {
        let p = rotated.center + rotated.rotation * local;
        let (distance, normal) = rotated.distance_and_normal(p);
        assert!((normal.length() - 1.0).abs() < 1e-12);
        let surface = p - normal * distance;
        assert!(rotated.distance_and_normal(surface).0.abs() < 1e-12);
        assert!((rotated.distance_and_normal(surface + normal * 0.001).0 - 0.001).abs() < 1e-12);
    }
}

#[test]
fn simulated_surface_expires_contact_but_preserves_material_coordinates() {
    use bevy_rapier3d::rapier::parry::shape::Ball;
    let mut cloth = patch(Vec3::Y);
    let contact = cloth
        .surface()
        .contacts(&Ball::new(0.02), &Isometry::translation(0.25, 1.019, 0.25))
        .unwrap()
        .remove(0);
    let material = cloth
        .surface()
        .material_point_from_contact(&contact)
        .unwrap();
    let before = cloth.surface().resolve(&material).unwrap();
    cloth.step(1.0 / 60.0, -DVec3::Y * 9.81, &[]).unwrap();
    assert!(!cloth.surface().contact_matches_surface(&contact));
    let after = cloth.surface().resolve(&material).unwrap();
    assert_eq!(before.uv, after.uv);
    assert!(after.position.y < before.position.y);
}

#[test]
fn damped_spring_dissipates_energy_without_moving_the_centre() {
    let inverse_mass = vec![2.0, 2.0];
    let spring = DistanceSpring {
        vertices: [0, 1],
        rest: 1.0,
        compliance: 0.01,
        damping_ratio: 1.0,
        stretch: true,
    };
    let mut positions = vec![DVec3::new(-0.6, 0., 0.), DVec3::new(0.6, 0., 0.)];
    let mut velocities = vec![DVec3::ZERO; 2];
    let h = 1.0 / 1200.0;
    let energy = |p: &[DVec3], v: &[DVec3]| {
        (p[0].distance(p[1]) - 1.0).powi(2) / 0.02
            + 0.25 * (v[0].length_squared() + v[1].length_squared())
    };
    let mut previous_energy = energy(&positions, &velocities);
    for _ in 0..1200 {
        let old = positions.clone();
        for (p, v) in positions.iter_mut().zip(&velocities) {
            *p += *v * h;
        }
        spring.project(&mut positions, &old, &inverse_mass, h);
        for ((v, p), old) in velocities.iter_mut().zip(&positions).zip(old) {
            *v = (*p - old) / h;
        }
        let next = energy(&positions, &velocities);
        assert!(next <= previous_energy + 1e-10);
        assert!((positions[0] + positions[1]).length() < 1e-12);
        previous_energy = next;
    }
    assert!(previous_energy < 1e-8);
}

impl SheetFixture {
    fn observe(
        &self,
        cloth: &ClothSolver,
        collider: &FixedClothCollider,
        frame: usize,
    ) -> serde_json::Value {
        let stretch = self
            .edges
            .iter()
            .zip(&self.rest_edge_lengths_m)
            .map(|([a, b], length)| cloth.positions[*a].distance(cloth.positions[*b]) / length)
            .fold(0.0, f64::max);
        let support = cloth
            .positions
            .iter()
            .filter(|p| {
                p.x > 3.33 && p.x < 5.07 && p.z > -4.43 && p.z < -2.37 && (p.y - 0.62).abs() < 0.008
            })
            .count();
        let penetration = cloth
            .positions
            .iter()
            .map(|p| (0.0005 - collider.distance_and_normal(*p).0).max(0.0))
            .fold(0.0, f64::max);
        serde_json::json!({"frame":frame,"seconds":frame as f64/60.0,
            "max_edge_stretch_ratio":stretch,"max_speed_m_s":cloth.max_speed(),
            "support_fraction":support as f64/cloth.positions.len() as f64,
            "max_particle_penetration_m":penetration})
    }
}

fn write_trace(
    report: &serde_json::Value,
    records: Vec<serde_json::Value>,
    snapshots: Vec<serde_json::Value>,
) {
    if let Ok(directory) = std::env::var("CC_CLOTH_TRACE_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir(&directory).expect("trace directory must be new");
        for (name, data) in [
            ("report.json", report.clone()),
            ("observations.json", serde_json::json!(records)),
            ("positions.json", serde_json::json!(snapshots)),
        ] {
            std::fs::write(directory.join(name), serde_json::to_vec(&data).unwrap()).unwrap();
        }
    }
}
