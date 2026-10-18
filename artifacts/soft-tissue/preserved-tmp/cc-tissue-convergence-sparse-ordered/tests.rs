use super::*;
use bevy::math::DMat3;

fn parameters() -> Material {
    Material {
        density_kg_m3: 950.0,
        shear_pa: 1500.0,
        bulk_pa: 30000.0,
        strain_damping_per_second: 60.0,
        iterations: 8,
    }
}

pub(super) fn cube(frame: Frame, attached: bool) -> Tissue {
    let rest: Vec<_> = (0..8)
        .map(|i| DVec3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64) * 0.1)
        .collect();
    let topology = vec![
        [0, 1, 3, 7],
        [0, 3, 2, 7],
        [0, 2, 6, 7],
        [0, 6, 4, 7],
        [0, 4, 5, 7],
        [0, 5, 1, 7],
    ];
    let attachments = rest
        .iter()
        .map(|p| {
            if attached && p.x == 0.0 {
                Attachment::Fixed
            } else {
                Attachment::Free
            }
        })
        .collect();
    Tissue::new(
        rest.clone(),
        rest,
        topology,
        attachments,
        parameters(),
        frame,
    )
    .unwrap()
}

#[test]
fn mass_is_density_times_volume_and_not_sum_of_inverse_tet_masses() {
    let tissue = cube(Frame::IDENTITY, false);
    assert!((tissue.metrics().mass_kg - 0.95).abs() < 1e-12);
    assert!(tissue.masses.iter().all(|mass| *mass > 0.0));
}

#[test]
fn material_gradients_match_finite_differences() {
    let tissue = cube(Frame::IDENTITY, false);
    let mut positions = tissue.positions.clone();
    let deformation = DMat3::from_cols(
        DVec3::new(1.1, 0.2, 0.05),
        DVec3::new(0.03, 0.9, 0.1),
        DVec3::new(0.1, 0.05, 1.03),
    );
    for p in &mut positions {
        *p = deformation * *p;
    }
    let tet = &tissue.tets[0];
    let analytical = tet.constraints(&positions);
    for node in 0..4 {
        for axis in 0..3 {
            let index = tet.ids[node];
            let eps = 1e-7;
            positions[index][axis] += eps;
            let plus = tet.constraints(&positions).values;
            positions[index][axis] -= 2.0 * eps;
            let minus = tet.constraints(&positions).values;
            positions[index][axis] += eps;
            for constraint in 0..2 {
                let measured = (plus[constraint] - minus[constraint]) / (2.0 * eps);
                assert!(
                    (measured - analytical.gradients[constraint][node][axis]).abs() < 1e-6,
                    "constraint {constraint}, node {node}, axis {axis}: {measured}"
                );
            }
        }
    }
}

#[test]
fn coupled_material_preserves_the_unloaded_equilibrium() {
    for attached in [false, true] {
        let mut tissue = cube(Frame::IDENTITY, attached);
        let original = tissue.positions.clone();
        for _ in 0..480 {
            tissue
                .step(1.0 / 240.0, Frame::IDENTITY, DVec3::ZERO)
                .unwrap();
        }
        for (a, b) in tissue.positions.iter().zip(original) {
            assert!(a.distance(b) < 1e-9, "rest drift {a} {b}");
        }
        assert!(tissue.metrics().kinetic_energy_j < 1e-15);
    }
}

#[test]
fn rigid_coordinate_changes_leave_dynamics_equivalent() {
    let frame = Frame {
        translation: DVec3::new(8.0, 2.0, -4.0),
        rotation: DQuat::from_rotation_y(0.7) * DQuat::from_rotation_z(0.4),
    };
    let mut a = cube(Frame::IDENTITY, true);
    let mut b = cube(frame, true);
    for _ in 0..240 {
        a.step(1.0 / 240.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81)
            .unwrap();
        b.step(1.0 / 240.0, frame, frame.rotation * (DVec3::NEG_Y * 9.81))
            .unwrap();
    }
    for (p, q) in a.positions.iter().zip(&b.positions) {
        assert!(frame.point(*p).distance(*q) < 1e-8);
    }
}

#[test]
fn constant_velocity_does_not_drag_free_or_attached_tissue() {
    let velocity = DVec3::new(0.5, -0.2, 0.1);
    for attached in [false, true] {
        let mut tissue = cube(Frame::IDENTITY, attached);
        tissue.velocities.fill(velocity);
        for tick in 1..=480 {
            let frame = Frame {
                translation: velocity * tick as f64 / 240.0,
                ..Frame::IDENTITY
            };
            tissue.step(1.0 / 240.0, frame, DVec3::ZERO).unwrap();
        }
        for (p, rest) in tissue.positions.iter().zip(&tissue.rest) {
            assert!(
                p.distance(*rest + velocity * 2.0) < 1e-8,
                "constant velocity drift"
            );
        }
    }
}

#[test]
fn strain_damping_preserves_rigid_velocity_and_dissipates_deformation() {
    let tissue = cube(Frame::IDENTITY, false);
    let omega = DVec3::new(0.0, 0.0, 0.7);
    let rigid: Vec<_> = tissue
        .positions
        .iter()
        .map(|p| DVec3::X * 0.5 + omega.cross(*p))
        .collect();
    let mut velocities = rigid.clone();
    tissue.damp_strain(&tissue.positions, &mut velocities, 1.0 / 240.0);
    for (a, b) in velocities.iter().zip(&rigid) {
        assert!(a.distance(*b) < 1e-12);
    }
    velocities[7] += DVec3::new(0.2, -0.3, 0.1);
    let momentum = |v: &[DVec3]| {
        v.iter()
            .zip(&tissue.masses)
            .map(|(v, m)| *v * *m)
            .sum::<DVec3>()
    };
    let energy = |v: &[DVec3]| {
        v.iter()
            .zip(&tissue.masses)
            .map(|(v, m)| 0.5 * *m * v.length_squared())
            .sum::<f64>()
    };
    let before = energy(&velocities);
    let p = momentum(&velocities);
    tissue.damp_strain(&tissue.positions, &mut velocities, 1.0 / 240.0);
    assert!(energy(&velocities) < before);
    assert!(momentum(&velocities).distance(p) < 1e-12);
}

#[test]
fn gravitational_sag_preserves_volume_and_settles() {
    let mut tissue = cube(Frame::IDENTITY, true);
    for _ in 0..1440 {
        tissue
            .step(1.0 / 240.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81)
            .unwrap();
    }
    let metrics = tissue.metrics();
    let deflection = tissue.positions[7] - tissue.rest[7];
    assert!(
        deflection.y < -0.001 && deflection.y > -0.06,
        "deflection {deflection:?}"
    );
    assert!(
        metrics.minimum_volume_ratio > 0.95 && metrics.maximum_volume_ratio < 1.05,
        "{metrics:?}"
    );
    assert!(metrics.maximum_speed_m_s < 0.002, "{metrics:?}");
    for i in [0, 2, 4, 6] {
        assert_eq!(tissue.positions[i], tissue.rest[i]);
    }
}

#[test]
fn invalid_topology_and_failed_step_are_rejected_without_mutating_state() {
    let mut tissue = cube(Frame::IDENTITY, true);
    let old_positions = tissue.positions.clone();
    let old_velocities = tissue.velocities.clone();
    assert!(tissue
        .step(
            1.0 / 240.0,
            Frame {
                translation: DVec3::X * 10.0,
                ..Frame::IDENTITY
            },
            DVec3::ZERO
        )
        .is_err());
    assert_eq!(tissue.positions, old_positions);
    assert_eq!(tissue.velocities, old_velocities);
    assert_eq!(tissue.frame.translation, DVec3::ZERO);
    assert!(tissue.step(f64::NAN, Frame::IDENTITY, DVec3::ZERO).is_err());
    assert!(Tissue::new(
        tissue.rest.clone(),
        tissue.rest.clone(),
        vec![[0, 0, 1, 2]],
        tissue.attachments.clone(),
        parameters(),
        Frame::IDENTITY
    )
    .is_err());
}

#[test]
fn bulk_and_shear_are_independent_material_responses() {
    let tissue = cube(Frame::IDENTITY, false);
    let tet = &tissue.tets[0];
    let shear = DMat3::from_cols(DVec3::X, DVec3::new(0.3, 1.0, 0.0), DVec3::Z);
    let shear_positions: Vec<_> = tissue.positions.iter().map(|p| shear * *p).collect();
    let shear_a = tet.energy(&shear_positions, 1500.0, 30000.0);
    let shear_b = tet.energy(&shear_positions, 1500.0, 60000.0);
    assert!((shear_a - shear_b).abs() < 1e-12);
    let compressed: Vec<_> = tissue.positions.iter().map(|p| 0.9 * *p).collect();
    assert!(tet.energy(&compressed, 1500.0, 60000.0) > tet.energy(&compressed, 1500.0, 30000.0));
}
