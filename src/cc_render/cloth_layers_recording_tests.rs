//! Two independently authored fabric patches; no avatar or authored duvet assets.
use super::*;

fn sheet(upper: bool) -> ClothSolver {
    let mut rest = Vec::new();
    let mut initial = Vec::new();
    let mut uv = Vec::new();
    let mut triangles = Vec::new();
    for row in 0..=8 {
        for col in 0..=8 {
            let (x, z) = (col as f32 / 8., row as f32 / 8.);
            let y = if upper {
                0.15 + 0.04 * (x * std::f32::consts::PI).sin()
            } else {
                0.02
            };
            let offset = if upper { 0.03 } else { 0. };
            rest.push(Vec3::new(x + offset, 0., z + offset));
            initial.push(Vec3::new(x + offset, y, z + offset));
            uv.push(Vec2::new(x, z));
            if row < 8 && col < 8 {
                let a = (row * 9 + col) as u32;
                triangles.extend([[a, a + 9, a + 1], [a + 1, a + 9, a + 10]]);
            }
        }
    }
    ClothSolver::new(
        rest,
        initial,
        triangles,
        uv,
        ClothParameters {
            total_mass_kg: if upper { 0.5 } else { 0.2 },
            thickness_m: if upper { 0.004 } else { 0.001 },
            stretch_compliance: 1e-7,
            bend_compliance: 0.05,
            air_drag_per_second: 0.2,
            spring_damping_ratio: 1.,
            cloth_static_friction: 0.2,
            cloth_kinetic_friction: 0.2,
            substeps: 24,
        },
    )
    .unwrap()
}

pub(super) fn interlayer_gap(a: &ClothSolver, b: &ClothSolver) -> f64 {
    use parry_ccd::{
        math::{Isometry, Point},
        query::distance,
        shape::Triangle,
    };
    let faces = |cloth: &ClothSolver| {
        cloth
            .triangles
            .iter()
            .map(|ids| {
                let [a, b, c] = ids.map(|i| {
                    let p = cloth.positions[i];
                    Point::new(p.x, p.y, p.z)
                });
                Triangle::new(a, b, c)
            })
            .collect::<Vec<_>>()
    };
    let a = faces(a);
    let b = faces(b);
    a.iter()
        .flat_map(|a| {
            b.iter()
                .map(move |b| distance(&Isometry::identity(), a, &Isometry::identity(), b).unwrap())
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn independent_fabric_layers_settle_without_crossing() {
    let mut lower = sheet(false);
    let mut upper = sheet(true);
    let mut batch = ClothBatch::new(&[&lower, &upper]).unwrap();
    let floor = [FixedClothCollider::new(
        SharedShape::cuboid(3., 0.1, 3.),
        Isometry::translation(0., -0.1, 0.),
        0.5,
    )
    .unwrap()];
    let mut frames = Vec::new();
    let mut minimum_gap = f64::INFINITY;
    for frame in 0..=180 {
        if frame > 0 {
            batch
                .step(
                    &mut [
                        ClothLayerStep {
                            solver: &mut lower,
                            colliders: &floor,
                            grips: &mut [],
                        },
                        ClothLayerStep {
                            solver: &mut upper,
                            colliders: &floor,
                            grips: &mut [],
                        },
                    ],
                    1. / 60.,
                    -DVec3::Y * 9.81,
                )
                .unwrap_or_else(|e| panic!("layered fabric frame {frame}: {e}"));
        }
        let gap = interlayer_gap(&lower, &upper);
        minimum_gap = minimum_gap.min(gap);
        assert!(gap >= 0.002499, "layer gap at {frame}: {gap}");
        for cloth in [&lower, &upper] {
            assert!(cloth.max_particle_penetration(&floor) < 1e-6);
            assert!(cloth.max_edge_stretch_ratio() < 1.1);
        }
        if frame % 6 == 0 {
            frames.push(
                serde_json::json!({"frame":frame,"positions":lower.surface.positions().iter()
                .chain(upper.surface.positions()).map(|p|p.to_array()).collect::<Vec<_>>()}),
            );
        }
    }
    let max_speed = lower.max_speed().max(upper.max_speed());
    println!("INDEPENDENT_LAYERS_REPORT min_gap={minimum_gap} final_speed={max_speed}");
    assert!(max_speed < 0.06);
    if let Ok(path) = std::env::var("CC_CLOTH_LAYERS_TRACE") {
        let path = std::path::Path::new(&path);
        std::fs::create_dir(path).unwrap();
        let triangles: Vec<_> = lower
            .triangles
            .iter()
            .copied()
            .chain(
                upper
                    .triangles
                    .iter()
                    .map(|t| t.map(|i| i + lower.positions.len())),
            )
            .collect();
        std::fs::write(path.join("trace.json"), serde_json::to_vec(&serde_json::json!({
            "fixture":"Two separate fabric patches with different mass and thickness; not authored condo bedding",
            "layer_count":2,"triangles":triangles,"frames":frames,"floor_y":0.,
            "layer_thickness_m":[0.001,0.004],"layer_mass_kg":[0.2,0.5],
            "min_interlayer_triangle_gap_m":minimum_gap,"final_max_speed_m_s":max_speed,
            "face_materials":std::iter::repeat_n(0,lower.triangles.len())
                .chain(std::iter::repeat_n(1,upper.triangles.len())).collect::<Vec<_>>()
        })).unwrap()).unwrap();
    }
}
