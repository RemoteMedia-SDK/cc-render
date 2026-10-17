use super::*;

fn folded_sheet() -> ClothSolver {
    let (columns, rows) = (20, 10);
    let mut rest = Vec::new();
    let mut initial = Vec::new();
    let mut uv = Vec::new();
    let mut triangles = Vec::new();
    for row in 0..=rows {
        for column in 0..=columns {
            let u = column as f32 * 0.1;
            let v = row as f32 * 0.1;
            let radius = 0.1_f32 / 2_f32.sqrt();
            let (x, y) = if column <= 9 {
                (u, 0.08)
            } else if column >= 11 {
                (2. - u, 0.08 + 2. * radius)
            } else {
                (0.9 + radius, 0.08 + radius)
            };
            rest.push(Vec3::new(u, 0., v));
            initial.push(Vec3::new(x, y, v));
            uv.push(Vec2::new(u * 0.5, v));
            if column < columns && row < rows {
                let a = (row * (columns + 1) + column) as u32;
                let b = a + 1;
                let c = a + (columns + 1) as u32;
                triangles.extend([[a, c, b], [b, c, c + 1]]);
            }
        }
    }
    ClothSolver::new(
        rest,
        initial,
        triangles,
        uv,
        ClothParameters {
            total_mass_kg: 0.3,
            thickness_m: 0.001,
            stretch_compliance: 1e-7,
            bend_compliance: 0.05,
            air_drag_per_second: 0.2,
            spring_damping_ratio: 1.,
            substeps: 24,
        },
    )
    .unwrap()
}

fn observed_feature_gap(
    cloth: &ClothSolver,
    observer: &mut self_collision::SelfCollisionTopology,
) -> f64 {
    use parry_ccd::{
        math::{Isometry, Point},
        query::distance,
        shape::{Ball, Segment, Triangle},
    };
    let mut positions = cloth.positions.clone();
    let particles = GripParticles {
        positions: &mut positions,
        previous: &cloth.positions,
        inverse_mass: &cloth.inverse_mass,
        h: 1.,
    };
    observer
        .candidates(
            &particles,
            &cloth.triangles,
            cloth.parameters.thickness_m as f64,
        )
        .iter()
        .map(|pair| {
            let ids = pair.indices();
            let origin = cloth.positions[ids[0]];
            let [a, b, c, d] = ids.map(|i| {
                let p = cloth.positions[i] - origin;
                Point::new(p.x, p.y, p.z)
            });
            match pair {
                pair::FeaturePair::VertexFace(_) => distance(
                    &Isometry::identity(),
                    &Ball::new(0.),
                    &Isometry::identity(),
                    &Triangle::new(b, c, d),
                ),
                pair::FeaturePair::EdgeEdge(_) => distance(
                    &Isometry::identity(),
                    &Segment::new(a, b),
                    &Isometry::identity(),
                    &Segment::new(c, d),
                ),
            }
            .unwrap()
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn folded_connected_sheet_compresses_and_settles_above_a_fixed_floor() {
    let mut cloth = folded_sheet();
    let floor = FixedClothCollider::new(
        SharedShape::cuboid(3., 0.1, 3.),
        Isometry::translation(0., -0.1, 0.),
        0.5,
    )
    .unwrap();
    let mut frames = Vec::new();
    let mut observer = self_collision::SelfCollisionTopology::new(&cloth.triangles);
    let mut minimum_gap = f64::INFINITY;
    for frame in 0..=600 {
        if frame > 0 {
            cloth
                .step(1. / 60., -DVec3::Y * 9.81, std::slice::from_ref(&floor))
                .unwrap_or_else(|error| panic!("folded sheet frame {frame}: {error}"));
        }
        assert!(cloth.max_particle_penetration(std::slice::from_ref(&floor)) < 1e-6);
        let gap = observed_feature_gap(&cloth, &mut observer);
        minimum_gap = minimum_gap.min(gap);
        assert!(
            gap >= 0.000999,
            "self penetration at frame {frame}: gap={gap}"
        );
        if frame % 6 == 0 {
            frames.push(serde_json::json!({"frame":frame,"positions":cloth.surface().positions().iter().map(|p|p.to_array()).collect::<Vec<_>>() }));
        }
    }
    println!(
        "FOLDED_SHEET_REPORT gap={minimum_gap} speed={} stretch={}",
        cloth.max_speed(),
        cloth.max_edge_stretch_ratio()
    );
    assert!(cloth.max_edge_stretch_ratio() < 1.15);
    assert!(cloth.max_speed() < 0.06);
    if let Ok(path) = std::env::var("CC_CLOTH_FOLD_TRACE") {
        let path = std::path::Path::new(&path);
        std::fs::create_dir(path).unwrap();
        std::fs::write(
            path.join("trace.json"),
            serde_json::to_vec(&serde_json::json!({"triangles":cloth.triangles,
            "frames":frames,"floor_y":0.,"thickness_m":cloth.parameters.thickness_m,
            "min_feature_gap_m":minimum_gap,"tail_speed_m_s":cloth.max_speed(),"edge_stretch_ratio":cloth.max_edge_stretch_ratio(),
            "face_materials":cloth.triangles.iter().map(|t| usize::from(t.iter().map(|i|i%21).sum::<usize>()>=30)).collect::<Vec<_>>()}))
            .unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn intersecting_layers_fail_without_publishing_a_partial_step() {
    let points = vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::ZERO, Vec3::X, Vec3::Z];
    let mut cloth = ClothSolver::new(
        points.clone(),
        points,
        vec![[0, 1, 2], [3, 4, 5]],
        vec![Vec2::ZERO; 6],
        ClothParameters {
            total_mass_kg: 0.8,
            thickness_m: 0.001,
            stretch_compliance: 1e-7,
            bend_compliance: 0.05,
            air_drag_per_second: 0.2,
            spring_damping_ratio: 1.,
            substeps: 24,
        },
    )
    .unwrap();
    let before = cloth.positions.clone();
    assert!(matches!(
        cloth.step(1. / 60., -DVec3::Y * 9.81, &[]),
        Err(ClothError::SelfContactUnresolved)
    ));
    assert_eq!(cloth.positions, before);
    assert_eq!(cloth.surface().revision(), 0);
}
