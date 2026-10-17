use super::*;

#[test]
fn fast_sheet_cannot_cross_a_thin_fixed_slab_between_clear_endpoints() {
    let collider =
        FixedClothCollider::new(SharedShape::cuboid(2., 0.005, 2.), Isometry::identity(), 0.)
            .unwrap();
    let previous = vec![
        DVec3::new(-0.4, 0.6, -0.4),
        DVec3::new(0.4, 0.6, -0.4),
        DVec3::new(0., 0.6, 0.4),
    ];
    let mut positions: Vec<_> = previous.iter().map(|p| *p - DVec3::Y * 1.2).collect();
    assert!(positions
        .iter()
        .chain(&previous)
        .all(|p| collider.distance_and_normal(*p).0 > 0.5));
    collide_surface(
        &mut GripParticles {
            positions: &mut positions,
            previous: &previous,
            inverse_mass: &[1.; 3],
            h: 1. / 60.,
        },
        &[[0, 1, 2]],
        &[collider],
        0.0005,
    )
    .unwrap();
    assert!(
        positions.iter().all(|p| p.y >= 0.00549),
        "sheet tunneled through slab: {positions:?}"
    );
}

#[test]
fn fast_face_crossing_is_detected_when_all_vertex_paths_miss_the_obstacle() {
    let collider = FixedClothCollider::new(
        SharedShape::cuboid(0.10, 0.005, 0.10),
        Isometry::identity(),
        0.,
    )
    .unwrap();
    let previous = vec![
        DVec3::new(-1., 0.6, -1.),
        DVec3::new(1., 0.6, -1.),
        DVec3::new(0., 0.6, 1.),
    ];
    let mut positions: Vec<_> = previous.iter().map(|p| *p - DVec3::Y * 1.2).collect();
    collide_surface(
        &mut GripParticles {
            positions: &mut positions,
            previous: &previous,
            inverse_mass: &[1.; 3],
            h: 1. / 60.,
        },
        &[[0, 1, 2]],
        std::slice::from_ref(&collider),
        0.0005,
    )
    .unwrap();
    let center = positions[0] * 0.25 + positions[1] * 0.25 + positions[2] * 0.5;
    assert!(
        center.y >= 0.00549,
        "triangle face tunneled through obstacle: {center:?}; vertices={positions:?}"
    );
    assert!(collider
        .triangle_contact([positions[0], positions[1], positions[2]], 0.00049)
        .unwrap()
        .is_none());
}

#[test]
fn swept_faces_respect_rotated_rounded_shapes_and_clear_near_misses() {
    use bevy_rapier3d::rapier::na::Vector3;
    for angle in [0., 0.6, 1.1] {
        let collider = FixedClothCollider::new(
            SharedShape::round_cuboid(0.1, 0.005, 0.1, 0.002),
            Isometry::new(Vector3::new(3., 2., -4.), Vector3::new(0., 0., angle)),
            0.,
        )
        .unwrap();
        for miss in [false, true] {
            let offset = if miss { DVec3::X * 3. } else { DVec3::ZERO };
            let old = [
                DVec3::new(-1., 0.6, -1.),
                DVec3::new(1., 0.6, -1.),
                DVec3::new(0., 0.6, 1.),
            ];
            let previous: Vec<_> = old
                .map(|p| collider.rotation * (p + offset) + collider.center)
                .into();
            let mut positions: Vec<_> = old
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    collider.rotation * (p + offset - DVec3::Y * (1.1 + i as f64 * 0.1))
                        + collider.center
                })
                .collect();
            let predicted = positions.clone();
            collide_surface(
                &mut GripParticles {
                    positions: &mut positions,
                    previous: &previous,
                    inverse_mass: &[1., 2., 3.],
                    h: 1. / 60.,
                },
                &[[0, 1, 2]],
                std::slice::from_ref(&collider),
                0.0005,
            )
            .unwrap();
            if miss {
                assert_eq!(positions, predicted, "clear motion must remain unchanged");
            } else {
                let center = positions[0] * 0.25 + positions[1] * 0.25 + positions[2] * 0.5;
                assert!((collider.rotation.conjugate() * (center - collider.center)).y >= 0.00749);
                assert!(collider
                    .triangle_contact([positions[0], positions[1], positions[2]], 0.00049)
                    .unwrap()
                    .is_none());
            }
        }
    }
}

#[test]
fn initially_overlapping_fast_face_recovers_its_side_and_can_leave() {
    let collider = FixedClothCollider::new(
        SharedShape::cuboid(0.1, 0.005, 0.1),
        Isometry::identity(),
        0.,
    )
    .unwrap();
    let previous = [
        DVec3::new(-1., 0.00499, -1.),
        DVec3::new(1., 0.00499, -1.),
        DVec3::new(0., 0.00499, 1.),
    ];
    for delta in [-0.6, 0.6] {
        let predicted = previous.map(|p| p + DVec3::Y * delta);
        let mut positions = predicted;
        collide_surface(
            &mut GripParticles {
                positions: &mut positions,
                previous: &previous,
                inverse_mass: &[1.; 3],
                h: 1. / 60.,
            },
            &[[0, 1, 2]],
            std::slice::from_ref(&collider),
            0.0005,
        )
        .unwrap();
        if delta > 0. {
            assert_eq!(positions, predicted);
        } else {
            assert!(positions.iter().all(|p| p.y >= 0.00549), "{positions:?}");
        }
    }
}

#[test]
fn swept_edge_contact_does_not_depend_on_triangle_winding() {
    let collider = FixedClothCollider::new(
        SharedShape::cuboid(0.1, 0.005, 0.1),
        Isometry::identity(),
        0.,
    )
    .unwrap();
    let previous = [
        DVec3::new(0.09, 0.6, -0.2),
        DVec3::new(0.09, 0.6, 0.2),
        DVec3::new(1., 0.6, 0.),
    ];
    for indices in [[0, 1, 2], [2, 1, 0], [1, 2, 0]] {
        let mut positions = previous.map(|p| p - DVec3::Y * 1.2);
        collide_surface(
            &mut GripParticles {
                positions: &mut positions,
                previous: &previous,
                inverse_mass: &[1.; 3],
                h: 1. / 60.,
            },
            &[indices],
            std::slice::from_ref(&collider),
            0.0005,
        )
        .unwrap();
        assert!(positions.iter().all(|p| p.y >= 0.00549), "{positions:?}");
    }
}

#[test]
fn deforming_face_grazing_rounded_mattress_completes_without_search_stall() {
    let collider = FixedClothCollider::new(
        SharedShape::round_cuboid(0.87, 0.11, 1.03, 0.03),
        Isometry::translation(4.2, 0.48, -3.4),
        0.7,
    )
    .unwrap();
    // Measured failing bed substep: the contact region moves almost tangentially
    // while a distant corner approaches much faster. Keep the original values.
    let previous = [
        DVec3::new(5.062660648231077, 0.621667601813614, -3.532750360615234),
        DVec3::new(5.133995322387149, 0.6127062898023652, -3.5327255681263017),
        DVec3::new(5.134007653821076, 0.6126971574680309, -3.5966186290297326),
    ];
    let mut positions = [
        DVec3::new(5.062640188342864, 0.6217598003913574, -3.532743270971675),
        DVec3::new(5.133995373412622, 0.6127066963205535, -3.5327255681263017),
        DVec3::new(5.133916746951582, 0.6122012724202225, -3.5966141351708503),
    ];
    collide_surface(
        &mut GripParticles {
            positions: &mut positions,
            previous: &previous,
            inverse_mass: &[1.; 3],
            h: 1. / 1440.,
        },
        &[[0, 1, 2]],
        std::slice::from_ref(&collider),
        0.0005,
    )
    .unwrap();
    assert!(positions.iter().all(|p| p.is_finite()));
    assert!(collider
        .triangle_contact(positions, 0.00049)
        .unwrap()
        .is_none());
}

fn rounded() -> FixedClothCollider {
    FixedClothCollider::new(
        SharedShape::round_cuboid(0.5, 0.1, 0.5, 0.1),
        Isometry::identity(),
        0.,
    )
    .unwrap()
}

#[test]
fn witness_roundoff_is_bounded_and_does_not_admit_invalid_material_coordinates() {
    let weights = witness_weights([-2.83e-10, 1., 2.83e-10]).unwrap();
    assert!(weights.iter().all(|w| *w >= 0.));
    assert!((weights.iter().sum::<f64>() - 1.).abs() <= f64::EPSILON);
    for invalid in [[-0.001, 1.001, 0.], [0., 0., 0.], [f64::NAN, 0., 1.]] {
        assert!(witness_weights(invalid).is_err());
    }
}

#[test]
fn triangle_contact_detects_interior_overlap_with_clear_vertices() {
    let collider = rounded();
    let points = [
        DVec3::new(-1., 0.15, -1.),
        DVec3::new(1., 0.15, -1.),
        DVec3::new(0., 0.15, 1.),
    ];
    assert!(points
        .iter()
        .all(|p| collider.distance_and_normal(*p).0 > 0.));
    let (weights, depth, normal) = collider.triangle_contact(points, 0.0005).unwrap().unwrap();
    assert!((weights.iter().sum::<f64>() - 1.).abs() < 1e-6);
    assert!((depth - 0.0505).abs() < 1e-5, "{depth}");
    assert!(normal.distance(DVec3::Y) < 1e-5);
}

#[test]
fn triangle_contact_respects_rotation_and_rejects_clear_or_remote_faces() {
    use bevy_rapier3d::rapier::na::Vector3;
    let collider = FixedClothCollider::new(
        SharedShape::round_cuboid(0.5, 0.1, 0.5, 0.1),
        Isometry::new(Vector3::new(0.3, 0.4, -0.5), Vector3::new(0., 0., 0.7)),
        0.,
    )
    .unwrap();
    let local = [
        DVec3::new(-1., 0.15, -1.),
        DVec3::new(1., 0.15, -1.),
        DVec3::new(0., 0.15, 1.),
    ];
    let points = local.map(|p| collider.rotation * p + collider.center);
    let (_, depth, normal) = collider.triangle_contact(points, 0.0005).unwrap().unwrap();
    assert!((depth - 0.0505).abs() < 1e-5);
    assert!(normal.distance(collider.rotation * DVec3::Y) < 1e-5);
    assert!(collider
        .triangle_contact(points.map(|p| p + normal * 0.06), 0.0005)
        .unwrap()
        .is_none());
    assert!(collider
        .triangle_contact(points.map(|p| p + DVec3::splat(100.)), 0.0005)
        .unwrap()
        .is_none());
}

#[test]
fn triangle_contact_moves_only_touching_material_and_clears_dense_samples() {
    let collider = rounded();
    let mut positions = vec![
        DVec3::new(0.5, 0.201, 0.),
        DVec3::new(0.601, 0.1, 0.),
        DVec3::new(0.5, 0.201, 0.2),
        DVec3::new(4., 5., 6.),
    ];
    let previous = positions.clone();
    assert!(positions
        .iter()
        .all(|p| collider.distance_and_normal(*p).0 > 0.0005));
    let mass = [1., 2., 3., 1.];
    for _ in 0..12 {
        collider
            .project_triangle(
                [0, 1, 2],
                &mut GripParticles {
                    positions: &mut positions,
                    previous: &previous,
                    inverse_mass: &mass,
                    h: 1. / 1440.,
                },
                0.0005,
            )
            .unwrap();
    }
    assert_eq!(positions[3], previous[3]);
    for i in 0..=40 {
        for j in 0..=40 - i {
            let u = i as f64 / 40.;
            let v = j as f64 / 40.;
            let p = positions[0] * u + positions[1] * v + positions[2] * (1. - u - v);
            assert!(collider.distance_and_normal(p).0 >= 0.00049, "{p:?}");
        }
    }
}
