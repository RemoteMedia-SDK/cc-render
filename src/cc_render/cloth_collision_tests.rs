use super::*;

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
