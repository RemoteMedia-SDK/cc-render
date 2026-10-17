use super::*;

fn patch(center: DVec3) -> [DVec3; 3] {
    [
        DVec3::new(-0.05, 0., -0.05),
        DVec3::new(0.05, 0., -0.05),
        DVec3::new(0., 0., 0.05),
    ]
    .map(|p| p + center)
}

fn bar() -> FixedClothCollider {
    FixedClothCollider::new(
        SharedShape::cuboid(1., 0.02, 0.02),
        Isometry::identity(),
        0.,
    )
    .unwrap()
}

#[test]
fn translating_slab_hits_stationary_face_between_clear_endpoints() {
    let collider = FixedClothCollider::new(
        SharedShape::cuboid(0.01, 0.005, 0.01),
        Isometry::translation(0., -0.6, 0.),
        0.,
    )
    .unwrap();
    let sweep = RigidClothSweep::new(&collider, DVec3::ZERO, DVec3::Y * 1.2, DVec3::ZERO).unwrap();
    // The triangle's interior hits; all three vertex paths miss the small slab.
    let face = patch(DVec3::ZERO);
    let hit = sweep.triangle(face, face, 0.0005).unwrap().unwrap();
    let expected = (0.6_f32 as f64 - 0.005_f32 as f64 - 0.0005) / 1.2;
    assert!((hit.fraction - expected).abs() < 1e-6, "{hit:?}");
    assert!(hit.normal.dot(DVec3::Y) > 0.999);
    assert!(hit.point.y.abs() < 1e-10);
    assert!((hit.separation - 0.0005).abs() < 2.1e-8);
}

#[test]
fn full_turn_cannot_hide_a_rotating_bar_crossing() {
    let collider = bar();
    let sweep = RigidClothSweep::new(
        &collider,
        DVec3::ZERO,
        DVec3::ZERO,
        DVec3::Z * std::f64::consts::TAU,
    )
    .unwrap();
    let face = patch(DVec3::Y * 0.7);
    for t in [0., 1.] {
        assert!(face
            .iter()
            .all(|p| collider.distance_and_normal(sweep.unmove(*p, t)).0 > 0.6));
    }
    let hit = sweep.triangle(face, face, 0.001).unwrap().unwrap();
    assert!(hit.fraction > 0.2 && hit.fraction < 0.25, "{hit:?}");
    let material_point = face
        .into_iter()
        .zip(hit.weights)
        .map(|(p, w)| p * w)
        .sum::<DVec3>();
    assert!((material_point - hit.point).length() < 1e-12);
    let local_point = sweep.unmove(hit.point, hit.fraction);
    let (distance, normal) = collider.distance_and_normal(local_point);
    assert!((distance - hit.separation).abs() < 1e-12);
    assert!((sweep.turn(hit.fraction) * normal - hit.normal).length() < 1e-12);
}

#[test]
fn offset_collider_orbits_its_body_pivot() {
    let collider = FixedClothCollider::new(
        SharedShape::round_cuboid(0.04, 0.04, 0.04, 0.01),
        Isometry::translation(0.8, 0., 0.),
        0.,
    )
    .unwrap();
    let sweep = RigidClothSweep::new(
        &collider,
        DVec3::ZERO,
        DVec3::ZERO,
        DVec3::Z * std::f64::consts::PI,
    )
    .unwrap();
    let face = patch(DVec3::Y * 0.8);
    let hit = sweep.triangle(face, face, 0.001).unwrap().unwrap();
    assert!(hit.fraction > 0.4 && hit.fraction < 0.5, "{hit:?}");
    assert!(hit.normal.is_normalized());
}

#[test]
fn common_translation_preserves_rotating_contact_fraction() {
    let collider = bar();
    let face = patch(DVec3::Y * 0.7);
    let rotation = DVec3::Z * std::f64::consts::PI;
    let translation = DVec3::new(0.25, -0.5, 0.125);
    let a = RigidClothSweep::new(&collider, DVec3::ZERO, DVec3::ZERO, rotation)
        .unwrap()
        .triangle(face, face, 0.001)
        .unwrap()
        .unwrap();
    let b = RigidClothSweep::new(&collider, DVec3::ZERO, translation, rotation)
        .unwrap()
        .triangle(face, face.map(|p| p + translation), 0.001)
        .unwrap()
        .unwrap();
    assert!((a.fraction - b.fraction).abs() < 1e-10);
    assert!((b.point - a.point - translation * a.fraction).length() < 1e-10);
    assert!((a.normal - b.normal).length() < 1e-10);
}

#[test]
fn moving_bar_certifies_a_clear_rotating_path() {
    let collider = bar();
    let sweep = RigidClothSweep::new(
        &collider,
        DVec3::ZERO,
        DVec3::ZERO,
        DVec3::Z * std::f64::consts::TAU,
    )
    .unwrap();
    let face = patch(DVec3::Y * 1.2);
    assert!(sweep.triangle(face, face, 0.001).unwrap().is_none());
    assert!(sweep.triangle(face, face, f64::NAN).is_err());
    assert!(RigidClothSweep::new(
        &collider,
        DVec3::ZERO,
        DVec3::ZERO,
        DVec3::splat(f64::INFINITY)
    )
    .is_err());
}

#[test]
fn surface_contact_velocity_includes_translation_and_rotation_about_offset_pivot() {
    let collider = bar();
    let origin = DVec3::new(0.1, -0.2, 0.3);
    let translation = DVec3::new(0.2, 0.3, -0.1);
    let rotation = DVec3::Z * 0.7;
    let sweep = RigidClothSweep::new(&collider, origin, translation, rotation).unwrap();
    let witness = Witness {
        point: DVec3::new(0.5, 0.021, 0.),
        normal: DVec3::Y,
        distance: 0.001,
    };
    let hit = sweep.hit(0.4, witness, [1., 0., 0.]);
    let surface = DVec3::new(0.5, 0.02, 0.);
    let position = |t| origin + sweep.turn(t) * (surface - origin) + translation * t;
    let epsilon = 1e-5;
    let duration = 1. / 60.;
    let finite_difference =
        (position(0.4 + epsilon) - position(0.4 - epsilon)) / (2. * epsilon * duration);
    let velocity = sweep.contact_velocity(&hit, duration).unwrap();
    assert!((velocity - finite_difference).length() < 1e-8);
    assert!((velocity - translation / duration).length() > 1.);
    assert!(sweep.contact_velocity(&hit, 0.).is_err());
}

#[test]
fn moving_slab_queries_deforming_face_at_its_first_corner_contact() {
    let collider =
        FixedClothCollider::new(SharedShape::cuboid(1., 0.01, 1.), Isometry::identity(), 0.)
            .unwrap();
    let sweep = RigidClothSweep::new(&collider, DVec3::ZERO, DVec3::Y * 0.3, DVec3::ZERO).unwrap();
    let face = patch(DVec3::Y * 0.5);
    let end = [
        face[0] - DVec3::Y * 0.2,
        face[1] - DVec3::Y * 0.4,
        face[2] - DVec3::Y,
    ];
    let hit = sweep.triangle(face, end, 0.001).unwrap().unwrap();
    let expected = (0.5 - 0.01_f32 as f64 - 0.001) / 1.3;
    assert!((hit.fraction - expected).abs() < 1e-6, "{hit:?}");
    assert!((hit.point - face[2].lerp(end[2], hit.fraction)).length() < 1e-6);
    assert!((sweep.contact_velocity(&hit, 1. / 60.).unwrap() - DVec3::Y * 18.).length() < 1e-12);
}

#[test]
fn arbitrary_world_rotation_preserves_contact_time_point_and_normal() {
    let original = bar();
    let transformed = FixedClothCollider::new(
        SharedShape::cuboid(1., 0.02, 0.02),
        Isometry::new(
            bevy_rapier3d::rapier::na::Vector3::new(1.2, -0.4, 2.3),
            bevy_rapier3d::rapier::na::Vector3::new(0.4, -0.8, 0.3),
        ),
        0.,
    )
    .unwrap();
    let q = transformed.rotation;
    let origin = transformed.center;
    let translation = DVec3::new(0.1, 0.2, -0.1);
    let rotation = DVec3::Z * std::f64::consts::PI;
    let face = patch(DVec3::Y * 0.7);
    let a = RigidClothSweep::new(&original, DVec3::ZERO, translation, rotation)
        .unwrap()
        .triangle(face, face, 0.001)
        .unwrap()
        .unwrap();
    let other_face = face.map(|p| origin + q * p);
    let b = RigidClothSweep::new(&transformed, origin, q * translation, q * rotation)
        .unwrap()
        .triangle(other_face, other_face, 0.001)
        .unwrap()
        .unwrap();
    assert!((a.fraction - b.fraction).abs() < 1e-9, "{a:?} {b:?}");
    assert!((b.point - origin - q * a.point).length() < 1e-9);
    assert!((b.normal - q * a.normal).length() < 1e-9);
}
