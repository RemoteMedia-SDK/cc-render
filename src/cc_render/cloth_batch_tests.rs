use super::*;

fn patch(y: f32, mass: f64, thickness: f32, substeps: usize) -> ClothSolver {
    let points = vec![
        Vec3::new(-1., y, -1.),
        Vec3::new(1., y, -1.),
        Vec3::new(0., y, 1.),
    ];
    ClothSolver::new(
        points.clone(),
        points,
        vec![[0, 1, 2]],
        vec![Vec2::ZERO; 3],
        ClothParameters {
            total_mass_kg: mass,
            thickness_m: thickness,
            stretch_compliance: 1e-7,
            bend_compliance: 0.05,
            air_drag_per_second: 0.,
            spring_damping_ratio: 1.,
            substeps,
        },
    )
    .unwrap()
}

fn step(
    batch: &mut ClothBatch,
    a: &mut ClothSolver,
    b: &mut ClothSolver,
) -> Result<(), ClothError> {
    batch.step(
        &mut [
            ClothLayerStep {
                solver: a,
                colliders: &[],
                grips: &mut [],
            },
            ClothLayerStep {
                solver: b,
                colliders: &[],
                grips: &mut [],
            },
        ],
        1. / 60.,
        DVec3::ZERO,
    )
}

#[test]
fn independent_layers_keep_mass_and_combined_thickness_during_fast_impact() {
    let mut a = patch(0.1, 1., 0.001, 12);
    let mut b = patch(-0.1, 3., 0.003, 24);
    a.velocities.fill(-DVec3::Y * 100.);
    let centre = |a: &ClothSolver, b: &ClothSolver| {
        (a.positions.iter().map(|p| p.y).sum::<f64>()
            + 3. * b.positions.iter().map(|p| p.y).sum::<f64>())
            / 12.
    };
    let before = centre(&a, &b);
    let mut batch = ClothBatch::new(&[&a, &b]).unwrap();
    step(&mut batch, &mut a, &mut b).unwrap();
    assert!((centre(&a, &b) - before + 25. / 60.).abs() < 1e-9);
    for (a, b) in a.positions.iter().zip(&b.positions) {
        assert!(a.y - b.y >= 0.001999);
    }
    assert_eq!(a.surface.revision(), 1);
    assert_eq!(b.surface.revision(), 1);
}

#[test]
fn a_failed_layer_leaves_every_surface_and_velocity_unchanged() {
    let mut a = patch(0., 1., 0.001, 24);
    let mut b = patch(0., 2., 0.002, 24);
    let before = (
        a.positions.clone(),
        b.positions.clone(),
        a.velocities.clone(),
        b.velocities.clone(),
    );
    let mut batch = ClothBatch::new(&[&a, &b]).unwrap();
    assert!(matches!(
        step(&mut batch, &mut a, &mut b),
        Err(ClothError::SelfContactUnresolved)
    ));
    assert_eq!(
        (a.positions, b.positions, a.velocities, b.velocities),
        before
    );
    assert_eq!(a.surface.revision(), 0);
    assert_eq!(b.surface.revision(), 0);
}

#[test]
fn single_layer_batch_matches_standalone_physics() {
    let mut single = patch(1., 0.8, 0.001, 24);
    let mut grouped = patch(1., 0.8, 0.001, 24);
    let mut batch = ClothBatch::new(&[&grouped]).unwrap();
    for _ in 0..60 {
        single.step(1. / 60., -DVec3::Y * 9.81, &[]).unwrap();
        batch
            .step(
                &mut [ClothLayerStep {
                    solver: &mut grouped,
                    colliders: &[],
                    grips: &mut [],
                }],
                1. / 60.,
                -DVec3::Y * 9.81,
            )
            .unwrap();
        assert_eq!(single.positions, grouped.positions);
        assert_eq!(single.velocities, grouped.velocities);
    }
    assert_eq!(single.surface.revision(), grouped.surface.revision());
}

#[test]
fn each_layer_uses_its_own_fixed_supports_and_material_identity() {
    use bevy_rapier3d::rapier::parry::shape::Ball;
    let mut supported = patch(0.01, 1., 0.002, 24);
    let mut falling = patch(0.01, 1., 0.002, 24);
    for p in &mut falling.positions {
        p.x += 10.;
    }
    falling
        .surface
        .update_positions(falling.positions.iter().map(|p| p.as_vec3()).collect())
        .unwrap();
    let contact = supported
        .surface
        .contacts(&Ball::new(0.01), &Isometry::translation(0., 0.02, 0.))
        .unwrap()
        .remove(0);
    let material = supported
        .surface
        .material_point_from_contact(&contact)
        .unwrap();
    let floor = FixedClothCollider::new(
        SharedShape::cuboid(20., 0.1, 20.),
        Isometry::translation(0., -0.1, 0.),
        0.5,
    )
    .unwrap();
    let mut batch = ClothBatch::new(&[&supported, &falling]).unwrap();
    for _ in 0..30 {
        batch
            .step(
                &mut [
                    ClothLayerStep {
                        solver: &mut supported,
                        colliders: std::slice::from_ref(&floor),
                        grips: &mut [],
                    },
                    ClothLayerStep {
                        solver: &mut falling,
                        colliders: &[],
                        grips: &mut [],
                    },
                ],
                1. / 60.,
                -DVec3::Y * 9.81,
            )
            .unwrap();
    }
    assert!(supported
        .positions
        .iter()
        .all(|p| (p.y - 0.001).abs() < 1e-8));
    assert!(falling.positions.iter().all(|p| p.y < -1.));
    assert!(supported.surface.resolve(&material).is_some());
    assert!(falling.surface.resolve(&material).is_none());
    assert!(!supported.surface.contact_matches_surface(&contact));
}

#[test]
fn invalid_later_layer_grip_cannot_commit_an_earlier_layer() {
    use crate::cc_render::cloth_grip::ClampMaterial;
    use bevy_rapier3d::rapier::parry::shape::Ball;
    let mut a = patch(0., 1., 0.001, 24);
    let mut b = patch(1., 1., 0.001, 24);
    let grip = ClothGrip::capture(
        a.surface(),
        &Ball::new(0.01),
        &Isometry::translation(0., 0.01, 0.),
        0,
        ClampMaterial {
            compliance_m_n: 0.001,
            damping_ratio: 1.,
            capacity_n: 20.,
        },
    )
    .unwrap()
    .unwrap();
    let mut wrong_layer_grip = [grip];
    let before = (a.positions.clone(), b.positions.clone());
    let mut batch = ClothBatch::new(&[&a, &b]).unwrap();
    let result = batch.step(
        &mut [
            ClothLayerStep {
                solver: &mut a,
                colliders: &[],
                grips: &mut [],
            },
            ClothLayerStep {
                solver: &mut b,
                colliders: &[],
                grips: &mut wrong_layer_grip,
            },
        ],
        1. / 60.,
        -DVec3::Y * 9.81,
    );
    assert!(result.is_err());
    assert_eq!((a.positions, b.positions), before);
    assert_eq!(a.surface.revision(), 0);
    assert_eq!(b.surface.revision(), 0);
    assert_eq!(wrong_layer_grip[0].peak_applied_force_n(), 0.);
}

#[path = "cloth_layers_recording_tests.rs"]
mod recording;

#[test]
fn three_distinct_layers_stack_at_their_authored_radii() {
    let mut layers = [
        patch(0.01, 0.2, 0.001, 12),
        patch(0.03, 0.4, 0.003, 24),
        patch(0.08, 0.8, 0.005, 24),
    ];
    // Use the authored bedding drag; the ballistic patch fixture has zero drag.
    for layer in &mut layers {
        layer.parameters.air_drag_per_second = 0.2;
    }
    let mut batch = ClothBatch::new(&layers.iter().collect::<Vec<_>>()).unwrap();
    let floor = [FixedClothCollider::new(
        SharedShape::cuboid(3., 0.1, 3.),
        Isometry::translation(0., -0.1, 0.),
        0.5,
    )
    .unwrap()];
    for _ in 0..1800 {
        let mut steps: Vec<_> = layers
            .iter_mut()
            .map(|solver| ClothLayerStep {
                solver,
                colliders: &floor,
                grips: &mut [],
            })
            .collect();
        batch.step(&mut steps, 1. / 60., -DVec3::Y * 9.81).unwrap();
        for pair in layers.windows(2) {
            let gap =
                (pair[0].parameters.thickness_m + pair[1].parameters.thickness_m) as f64 * 0.5;
            assert!(recording::interlayer_gap(&pair[0], &pair[1]) >= gap - 1e-7);
            for (lower, upper) in pair[0].positions.iter().zip(&pair[1].positions) {
                assert!(upper.y - lower.y >= gap - 1e-7);
            }
        }
    }
    for (cloth, expected_y) in layers.iter().zip([0.0005, 0.0025, 0.0065]) {
        println!(
            "STACK_LAYER expected_y={expected_y} positions={:?} speed={}",
            cloth.positions,
            cloth.max_speed()
        );
        assert!(cloth
            .positions
            .iter()
            .all(|p| (p.y - expected_y).abs() < 1e-6));
        // The settled stack sleeps: it stops publishing long before 30 s.
        assert!(cloth.surface.revision() < 1800);
        assert!(cloth.max_speed() < 0.001);
    }
    assert!(batch.resting());
}

#[test]
fn settled_layers_sleep_without_publishing_and_wake_on_any_disturbance() {
    let mut sheet = patch(0.0006, 0.5, 0.001, 24);
    let slab = |y: f32| {
        [FixedClothCollider::new(
            SharedShape::cuboid(2., 0.1, 2.),
            Isometry::translation(0., y - 0.1, 0.),
            0.5,
        )
        .unwrap()]
    };
    let mut batch = ClothBatch::new(&[&sheet]).unwrap();
    let step = |batch: &mut ClothBatch, sheet: &mut ClothSolver, floor: &[FixedClothCollider]| {
        batch
            .step(
                &mut [ClothLayerStep {
                    solver: sheet,
                    colliders: floor,
                    grips: &mut [],
                }],
                1. / 60.,
                -DVec3::Y * 9.81,
            )
            .unwrap()
    };
    let floor = slab(0.);
    let mut ticks = 0;
    while !batch.resting() {
        step(&mut batch, &mut sheet, &floor);
        ticks += 1;
        assert!(ticks <= 120, "a sheet lying on a floor must come to rest");
    }
    // Asleep: time advances, nothing is simulated or published.
    let (revision, positions) = (sheet.surface.revision(), sheet.positions.clone());
    assert!(sheet.velocities.iter().all(|v| *v == DVec3::ZERO));
    for _ in 0..120 {
        step(&mut batch, &mut sheet, &floor);
    }
    assert!(batch.resting());
    assert_eq!(sheet.surface.revision(), revision);
    assert_eq!(sheet.positions, positions);
    // A changed collider snapshot wakes it: the floor drops away.
    let lowered = slab(-0.01);
    step(&mut batch, &mut sheet, &lowered);
    assert!(!batch.resting());
    assert_eq!(sheet.surface.revision(), revision + 1);
    assert!(sheet.positions.iter().all(|p| p.y < 0.0006));
    // Back at rest on the lower floor, a surface published outside the
    // batch also wakes it rather than being silently ignored.
    while !batch.resting() {
        step(&mut batch, &mut sheet, &lowered);
    }
    let moved: Vec<_> = sheet.positions.iter().map(|p| p.as_vec3()).collect();
    sheet.surface.update_positions(moved).unwrap();
    let revision = sheet.surface.revision();
    step(&mut batch, &mut sheet, &lowered);
    assert!(!batch.resting());
    assert_eq!(sheet.surface.revision(), revision + 1);
}

#[test]
fn baked_rest_starts_asleep_and_still_wakes_when_disturbed() {
    let mut sheet = patch(0.0005, 0.5, 0.001, 24);
    let floor = |y: f32| {
        [FixedClothCollider::new(
            SharedShape::cuboid(2., 0.1, 2.),
            Isometry::translation(0., y - 0.1, 0.),
            0.5,
        )
        .unwrap()]
    };
    let mut batch = ClothBatch::new(&[&sheet]).unwrap();
    batch.start_at_rest();
    let step = |batch: &mut ClothBatch, sheet: &mut ClothSolver, floor: &[FixedClothCollider]| {
        batch
            .step(
                &mut [ClothLayerStep {
                    solver: sheet,
                    colliders: floor,
                    grips: &mut [],
                }],
                1. / 60.,
                -DVec3::Y * 9.81,
            )
            .unwrap()
    };
    let resting = floor(0.);
    let positions = sheet.positions.clone();
    for _ in 0..30 {
        step(&mut batch, &mut sheet, &resting);
        assert!(batch.resting());
    }
    assert_eq!(sheet.surface.revision(), 0);
    assert_eq!(sheet.positions, positions);
    // The same wake rule applies to a baked rest: the floor drops away.
    step(&mut batch, &mut sheet, &floor(-0.01));
    assert!(!batch.resting());
    assert_eq!(sheet.surface.revision(), 1);
}
