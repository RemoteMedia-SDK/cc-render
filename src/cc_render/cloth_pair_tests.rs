use super::*;

fn check(pair: FeaturePair, old: [DVec3; 4], mut end: [DVec3; 4], expected: bool) {
    let inverse_mass = [1., 2., 0.5, 1.5];
    let centre = |p: [DVec3; 4]| {
        p.into_iter()
            .zip(inverse_mass)
            .map(|(p, m)| p / m)
            .sum::<DVec3>()
    };
    let before = centre(end);
    assert_eq!(
        pair.resolved(
            &GripParticles {
                positions: &mut end,
                previous: &old,
                inverse_mass: &inverse_mass,
                h: 1. / 60.,
            },
            0.001,
            DVec3::ZERO,
        )
        .unwrap(),
        !expected
    );
    let changed = pair
        .project(
            &mut GripParticles {
                positions: &mut end,
                previous: &old,
                inverse_mass: &inverse_mass,
                h: 1. / 60.,
            },
            0.001,
        )
        .unwrap();
    assert_eq!(changed, expected);
    assert!(
        centre(end).distance(before) < 1e-10,
        "contact changed centre of mass"
    );
    if changed {
        let mut settled = false;
        for _ in 0..64 {
            if !pair
                .project(
                    &mut GripParticles {
                        positions: &mut end,
                        previous: &old,
                        inverse_mass: &inverse_mass,
                        h: 1. / 60.,
                    },
                    0.001,
                )
                .unwrap()
            {
                settled = true;
                break;
            }
            assert!(centre(end).distance(before) < 1e-10);
        }
        assert!(settled, "feature response did not converge");
        assert!(pair.witness(end).unwrap().distance >= 0.000999, "{:?}", end);
    }
}

#[test]
fn contact_certificate_rejects_submicrometre_overlap() {
    let points = [
        DVec3::new(0., 0.0009999, 0.),
        DVec3::new(-1., 0., -1.),
        DVec3::new(1., 0., -1.),
        DVec3::new(0., 0., 1.),
    ];
    let mut end = points;
    assert!(!FeaturePair::VertexFace([0, 1, 2, 3])
        .resolved(
            &GripParticles {
                positions: &mut end,
                previous: &points,
                inverse_mass: &[1.; 4],
                h: 1. / 60.
            },
            0.001,
            DVec3::Y,
        )
        .unwrap());
}

#[test]
fn vertex_face_sweep_catches_crossing_with_clear_endpoints() {
    let old = [
        DVec3::new(0., 0.6, 0.),
        DVec3::new(-1., 0., -1.),
        DVec3::new(1., 0., -1.),
        DVec3::new(0., 0., 1.),
    ];
    let mut end = old;
    end[0].y = -0.6;
    check(FeaturePair::VertexFace([0, 1, 2, 3]), old, end, true);
    let mut miss = old;
    miss[0].x = 3.;
    let mut end = miss;
    end[0].y = -0.6;
    check(FeaturePair::VertexFace([0, 1, 2, 3]), miss, end, false);
}

#[test]
fn edge_edge_sweep_catches_interiors_when_endpoint_paths_miss() {
    let old = [
        DVec3::new(-1., 0.6, 0.),
        DVec3::new(1., 0.6, 0.),
        DVec3::new(0., 0., -1.),
        DVec3::new(0., 0., 1.),
    ];
    let mut end = old;
    end[0].y = -0.6;
    end[1].y = -0.6;
    check(FeaturePair::EdgeEdge([0, 1, 2, 3]), old, end, true);
}

#[test]
fn common_translation_has_no_contact_impulse() {
    let old = [
        DVec3::new(-1., 0.6, 0.),
        DVec3::new(1., 0.6, 0.),
        DVec3::new(0., 0., -1.),
        DVec3::new(0., 0., 1.),
    ];
    check(
        FeaturePair::EdgeEdge([0, 1, 2, 3]),
        old,
        old.map(|p| p + DVec3::new(100., -300., 200.)),
        false,
    );
}

#[test]
fn initial_feature_intersection_is_explicitly_rejected() {
    let points = [
        DVec3::new(-1., 0., 0.),
        DVec3::new(1., 0., 0.),
        DVec3::new(0., 0., -1.),
        DVec3::new(0., 0., 1.),
    ];
    assert!(matches!(
        FeaturePair::EdgeEdge([0, 1, 2, 3]).sweep(points, points, 0.001),
        Err(ClothError::SelfContactUnresolved)
    ));
}

#[test]
fn contact_is_invariant_under_rotation_and_vertex_order() {
    let rotation = DQuat::from_euler(bevy::math::EulerRot::XYZ, 0.4, -0.7, 1.2);
    let old = [
        DVec3::new(0., 0.6, 0.),
        DVec3::new(-1., 0., -1.),
        DVec3::new(1., 0., -1.),
        DVec3::new(0., 0., 1.),
    ];
    let mut end = old;
    end[0].y = -0.6;
    let old = old.map(|p| rotation * p + DVec3::new(4., 2., -3.));
    let end = end.map(|p| rotation * p + DVec3::new(4., 2., -3.));
    check(FeaturePair::VertexFace([0, 1, 2, 3]), old, end, true);
    check(FeaturePair::VertexFace([0, 3, 2, 1]), old, end, true);
}

fn check_supported_face(half_width: f32) {
    let old = [
        DVec3::new(0., 0.2, 0.),
        DVec3::new(-1., 0.0005, -1.),
        DVec3::new(1., 0.0005, -1.),
        DVec3::new(0., 0.0005, 1.),
    ];
    let mut end = old;
    end[0].y = 0.0007;
    let floor = FixedClothCollider::new(
        SharedShape::cuboid(half_width, 0.1, half_width),
        bevy_rapier3d::rapier::parry::math::Isometry::translation(0., -0.1, 0.),
        0.,
    )
    .unwrap();
    let mut axis = DVec3::ZERO;
    assert!(FeaturePair::VertexFace([0, 1, 2, 3])
        .project_with_axis(
            &mut GripParticles {
                positions: &mut end,
                previous: &old,
                inverse_mass: &[1.; 4],
                h: 1. / 60.
            },
            0.001,
            &[floor],
            &mut axis
        )
        .unwrap());
    assert_eq!(&end[1..], &old[1..]);
    assert!(end[0].y >= 0.00149999);
}

#[test]
fn fixed_floor_reaction_keeps_supported_face_out_of_the_floor() {
    check_supported_face(3.);
}

#[test]
fn fixed_support_at_face_interior_participates_without_vertex_contact() {
    check_supported_face(0.1);
}

#[test]
fn fixed_support_at_edge_interior_participates_without_vertex_contact() {
    let old = [
        DVec3::new(-1., 0.2, 0.),
        DVec3::new(1., 0.2, 0.),
        DVec3::new(0., 0.0005, -1.),
        DVec3::new(0., 0.0005, 1.),
    ];
    let mut end = old;
    end[0].y = 0.0007;
    end[1].y = 0.0007;
    let floor = FixedClothCollider::new(
        SharedShape::cuboid(0.1, 0.1, 0.1),
        bevy_rapier3d::rapier::parry::math::Isometry::translation(0., -0.1, 0.),
        0.,
    )
    .unwrap();
    let pair = FeaturePair::EdgeEdge([0, 1, 2, 3]);
    assert!(pair
        .project_with_axis(
            &mut GripParticles {
                positions: &mut end,
                previous: &old,
                inverse_mass: &[1.; 4],
                h: 1. / 60.,
            },
            0.001,
            &[floor],
            &mut DVec3::ZERO,
        )
        .unwrap());
    assert_eq!(&end[2..], &old[2..]);
    assert!(pair.witness(end).unwrap().distance >= 0.00099999);
}
