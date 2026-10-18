use super::*;
use crate::cc_render::soft_tissue::EmbeddedPoint;

fn plane(tissue: &Tissue, node: usize, x: f64) -> PlaneContact {
    let tetrahedron = tissue
        .tets
        .iter()
        .position(|t| t.ids.contains(&node))
        .unwrap();
    PlaneContact {
        point: EmbeddedPoint {
            tetrahedron,
            barycentric: tissue.tets[tetrahedron].ids.map(|i| f64::from(i == node)),
            influence: 1.0,
            offset: DVec3::ZERO,
        },
        surface_point: DVec3::X * x,
        normal: DVec3::NEG_X,
        stiffness_n_per_m: 25_000.0,
    }
}

#[test]
fn separated_contacts_do_not_attract_or_change_unconstrained_dynamics() {
    let mut contact = tests::cube(Frame::IDENTITY, true);
    let mut free = contact.clone();
    let planes = [plane(&contact, 1, 1.0)];
    for _ in 0..60 {
        let metrics = contact
            .step_with_contacts(1.0 / 240.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81, &planes)
            .unwrap();
        free.step(1.0 / 240.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81)
            .unwrap();
        assert_eq!(metrics.active_contacts, 0);
        assert_eq!(metrics.normal_force_n, 0.0);
        assert_eq!(metrics.elastic_energy_j, 0.0);
        assert_eq!(contact.positions(), free.positions());
        assert_eq!(contact.velocities(), free.velocities());
    }
}

#[test]
fn moving_plane_compresses_volume_and_release_recovers_without_moving_attachments() {
    compression_and_recovery(TimeIntegration::ImplicitEuler, None);
}

#[test]
fn bdf2_contact_compression_and_release_preserve_attachments() {
    compression_and_recovery(TimeIntegration::Bdf2, None);
}

#[test]
fn continuum_viscous_contact_compression_and_release_preserve_attachments() {
    compression_and_recovery(TimeIntegration::Bdf2, Some(Viscosity {shear_pa_s:15.,bulk_pa_s:300.}));
}

#[test]
fn boundary_pressure_viscous_compression_and_release_preserve_attachments() {
    let old = tests::cube(Frame::IDENTITY, true);
    let material = Material {
        pressure_discretization: PressureDiscretization::AverageNodalBoundary,
        time_integration: TimeIntegration::Bdf2,
        viscosity: Some(Viscosity { shear_pa_s: 5., bulk_pa_s: 5000. }),
        strain_damping_per_second: 0.,
        strain_stiffening: 8.,
        shear_pa: 500.,
        bulk_pa: 500000.,
        iterations: 32,
        ..old.material
    };
    let tissue = Tissue::new(
        old.rest.clone(), old.rest, old.tets.iter().map(|tet| tet.ids).collect(),
        old.attachments, material, Frame::IDENTITY,
    ).unwrap();
    assert!(!tissue.pressure_patches.is_empty());
    assert!(tissue.element_pressure.iter().any(|local| *local));
    // This nearly incompressible, coarse clamped cube needs a harder penalty
    // plane than the 30 kPa legacy fixture. Keep its 0.1 mm penetration limit.
    check_compression_and_recovery(tissue, 2_500_000.0);
}

fn compression_and_recovery(method: TimeIntegration, viscosity: Option<Viscosity>) {
    let mut tissue = tests::cube(Frame::IDENTITY, true);
    tissue.material.time_integration = method;
    tissue.material.viscosity = viscosity;
    if viscosity.is_some() { tissue.material.strain_damping_per_second = 0.; }
    tissue.material.iterations = 32;
    check_compression_and_recovery(tissue, 250_000.0);
}

fn check_compression_and_recovery(mut tissue: Tissue, stiffness_n_per_m: f64) {
    let initial = tissue.positions.clone();
    let mut measured = ContactMetrics::default();
    for tick in 1..=120 {
        let x = 0.11 - 0.02 * (tick as f64 / 60.0).min(1.0);
        let planes: Vec<_> = [1, 3, 5, 7]
            .map(|i| PlaneContact {
                stiffness_n_per_m,
                ..plane(&tissue, i, x)
            })
            .into();
        measured = tissue
            .step_with_contacts(1.0 / 240.0, Frame::IDENTITY, DVec3::ZERO, &planes)
            .unwrap();
    }
    assert!(measured.active_contacts > 0 && measured.normal_force_n > 0.1);
    assert!(measured.maximum_penetration_m < 0.0001, "{measured:?}");
    assert!(tissue.positions[7].x < 0.091);
    assert!(tissue.metrics().minimum_volume_ratio > 0.85);
    for i in [0, 2, 4, 6] {
        assert_eq!(tissue.positions[i], initial[i]);
    }
    let release: Vec<_> = [1, 3, 5, 7].map(|i| plane(&tissue, i, 0.14)).into();
    for _ in 0..480 {
        let metrics = tissue
            .step_with_contacts(1.0 / 240.0, Frame::IDENTITY, DVec3::ZERO, &release)
            .unwrap();
        assert_eq!(metrics.normal_force_n, 0.0);
    }
    let recovery = tissue
        .positions
        .iter()
        .zip(&initial)
        .map(|(a, b)| a.distance(*b))
        .fold(0.0, f64::max);
    assert!(recovery < 0.0005, "recovery error {recovery}");
}

#[test]
fn splitting_contact_quadrature_preserves_response_and_force() {
    let mut whole = tests::cube(Frame::IDENTITY, true);
    let mut split = whole.clone();
    let contact = plane(&whole, 7, 0.098);
    let half = PlaneContact {
        stiffness_n_per_m: contact.stiffness_n_per_m / 2.0,
        ..contact
    };
    for _ in 0..30 {
        let a = whole
            .step_with_contacts(1.0 / 240.0, Frame::IDENTITY, DVec3::ZERO, &[contact])
            .unwrap();
        let b = split
            .step_with_contacts(1.0 / 240.0, Frame::IDENTITY, DVec3::ZERO, &[half, half])
            .unwrap();
        assert!((a.normal_force_n - b.normal_force_n).abs() < 1e-8);
        for (a, b) in whole.positions.iter().zip(&split.positions) {
            assert!(a.distance(*b) < 1e-10);
        }
    }
}

#[test]
fn contact_respects_rigid_coordinate_changes_and_embedded_skin_share() {
    let transform = Frame {
        translation: DVec3::new(0.7, -1.0, 2.0),
        rotation: DQuat::from_rotation_y(0.8) * DQuat::from_rotation_z(0.3),
    };
    let mut a = tests::cube(Frame::IDENTITY, true);
    let mut b = tests::cube(transform, true);
    let mut contact = plane(&a, 7, 0.098);
    contact.point.influence = 0.6;
    contact.point.offset = a.positions[7] * 0.4;
    let mut rotated = contact;
    rotated.point.offset = transform.rotation * contact.point.offset + transform.translation * 0.4;
    rotated.surface_point = transform.point(contact.surface_point);
    rotated.normal = transform.rotation * contact.normal;
    for _ in 0..60 {
        let ma = a
            .step_with_contacts(1.0 / 240.0, Frame::IDENTITY, DVec3::NEG_Y, &[contact])
            .unwrap();
        let mb = b
            .step_with_contacts(
                1.0 / 240.0,
                transform,
                transform.rotation * DVec3::NEG_Y,
                &[rotated],
            )
            .unwrap();
        assert!((ma.normal_force_n - mb.normal_force_n).abs() < 1e-6);
        for (a, b) in a.positions.iter().zip(&b.positions) {
            assert!(transform.point(*a).distance(*b) < 1e-8);
        }
    }
}

#[test]
fn invalid_contacts_leave_state_and_attachment_frame_unchanged() {
    let mut tissue = tests::cube(Frame::IDENTITY, true);
    let initial = tissue.positions.clone();
    let valid = plane(&tissue, 7, 0.098);
    let mut cases = vec![valid; 5];
    cases[0].normal *= 2.0;
    cases[1].point.tetrahedron = usize::MAX;
    cases[2].point.barycentric[0] = f64::NAN;
    cases[3].point.influence = 1.01;
    cases[4].stiffness_n_per_m = -1.0;
    for contact in cases {
        assert!(tissue
            .step_with_contacts(
                1.0 / 240.0,
                Frame {
                    translation: DVec3::X * 0.001,
                    ..Frame::IDENTITY
                },
                DVec3::ZERO,
                &[contact]
            )
            .is_err());
        assert_eq!(tissue.positions, initial);
        assert!(tissue.velocities.iter().all(|v| *v == DVec3::ZERO));
        assert_eq!(tissue.frame.translation, DVec3::ZERO);
    }
}
