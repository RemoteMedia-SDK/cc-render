//! Independent temporal reference cases; these never animate an avatar.
use super::*;

fn oscillator(frequency_hz: f64) -> Tissue {
    let mut tissue = tests::cube(Frame::IDENTITY, false);
    let omega = std::f64::consts::TAU * frequency_hz;
    // Equal acceleration on every vertex makes this a rigid translation of
    // the cube. Elastic strain and edge-strain damping both remain zero.
    tissue.attachments = tissue
        .masses
        .iter()
        .map(|mass| Attachment::Compliant {
            compliance_m_per_n: 1.0 / (mass * omega * omega),
        })
        .collect();
    for position in &mut tissue.positions {
        position.x += 0.01;
    }
    tissue
}

fn oscillator_state(tissue: &Tissue) -> (f64, f64) {
    let count = tissue.positions.len() as f64;
    let x = tissue
        .positions
        .iter()
        .zip(&tissue.rest)
        .map(|(p, r)| p.x - r.x)
        .sum::<f64>()
        / count;
    let v = tissue.velocities.iter().map(|v| v.x).sum::<f64>() / count;
    (x, v)
}

#[test]
fn implicit_euler_matches_closed_form_but_damps_an_undamped_oscillator() {
    let omega = std::f64::consts::TAU * 6.0;
    for rate in [120, 240, 480, 960] {
        let mut tissue = oscillator(6.0);
        let dt = 1.0 / rate as f64;
        for _ in 0..rate / 2 {
            tissue.step(dt, Frame::IDENTITY, DVec3::ZERO).unwrap();
        }
        let (x, v) = oscillator_state(&tissue);
        let phase = (rate / 2) as f64 * (omega * dt).atan();
        let radius = 0.01 * (1.0 + (omega * dt).powi(2)).powf(-(rate as f64) / 4.0);
        assert!((x - radius * phase.cos()).abs() < 2e-7, "rate={rate} x={x}");
        assert!(
            (v + omega * radius * phase.sin()).abs() < 2e-6,
            "rate={rate} v={v}"
        );
        let amplitude = (x * x + (v / omega).powi(2)).sqrt();
        println!(
            "TISSUE_TIME_REFERENCE {}",
            serde_json::json!({"method":"implicit_euler",
            "frequency_hz":6,"duration_s":0.5,"rate_hz":rate,"amplitude_mm":amplitude*1000.,
            "continuous_amplitude_mm":10.,"phase_error_radians":omega*0.5-phase})
        );
    }
}

#[test]
fn isolated_edge_damping_matches_exponential_decay_and_preserves_momentum() {
    for rate in [60, 120, 240, 480] {
        let mut tissue = tests::cube(Frame::IDENTITY, false);
        tissue.edges = vec![[0, 1]];
        tissue.damping_degree = 1.;
        tissue.material.strain_damping_per_second = 4.;
        let mut velocities = vec![DVec3::ZERO; tissue.positions.len()];
        velocities[1] = DVec3::X;
        let momentum = tissue.masses[1];
        for _ in 0..rate {
            tissue.damp_strain(&tissue.positions, &mut velocities, 1. / rate as f64);
        }
        assert!(((velocities[1] - velocities[0]).x - (-4.0_f64).exp()).abs() < 1e-12);
        assert!(
            (tissue.masses[0] * velocities[0].x + tissue.masses[1] * velocities[1].x - momentum)
                .abs()
                < 1e-12
        );
    }
}

#[test]
fn bdf2_oscillator_converges_to_the_continuous_solution_with_variable_steps() {
    let omega = std::f64::consts::TAU * 6.0;
    for variable in [false, true] {
        let mut errors = Vec::new();
        for rate in [240, 480, 960] {
            let mut tissue = oscillator(6.0);
            tissue.material.time_integration = TimeIntegration::Bdf2;
            let mut time = 0.;
            let mut maximum = 0.0_f64;
            for step in 0..rate / 2 {
                let scale = if !variable {
                    1.
                } else if step % 2 == 0 {
                    0.8
                } else {
                    1.2
                };
                let dt = scale / rate as f64;
                tissue.step(dt, Frame::IDENTITY, DVec3::ZERO).unwrap();
                time += dt;
                let (x, v) = oscillator_state(&tissue);
                let error = ((x - 0.01 * (omega * time).cos()).powi(2)
                    + (v / omega + 0.01 * (omega * time).sin()).powi(2))
                .sqrt();
                maximum = maximum.max(error);
            }
            errors.push(maximum);
            let (x, v) = oscillator_state(&tissue);
            println!(
                "TISSUE_TIME_REFERENCE {}",
                serde_json::json!({"method":"bdf2",
                "frequency_hz":6,"duration_s":time,"rate_hz":rate,"variable_steps":variable,
                "amplitude_mm":(x*x+(v/omega).powi(2)).sqrt()*1000.,"continuous_amplitude_mm":10.,
                "maximum_state_error_mm":maximum*1000.})
            );
        }
        assert!(
            errors[1] < errors[0] * 0.3 && errors[2] < errors[1] * 0.3,
            "{errors:?}"
        );
        assert!(errors[2] < 0.00015, "{errors:?}");
    }
}

#[test]
fn rejected_bdf2_steps_preserve_derivative_history_and_growth_restarts_only_the_method() {
    let mut tissue = oscillator(6.0);
    tissue.material.time_integration = TimeIntegration::Bdf2;
    tissue
        .step(1. / 480., Frame::IDENTITY, DVec3::ZERO)
        .unwrap();
    let old = tissue.clone();
    assert!(tissue
        .step(1. / 480., Frame::IDENTITY, DVec3::splat(f64::NAN))
        .is_err());
    assert_eq!(tissue.positions, old.positions);
    assert_eq!(tissue.velocities, old.velocities);
    assert_eq!(
        tissue.previous.as_ref().unwrap().positions,
        old.previous.as_ref().unwrap().positions
    );
    let mut invalid_prediction = tissue.clone();
    invalid_prediction.velocities[7] = DVec3::splat(-1e5);
    let before = invalid_prediction.clone();
    assert!(matches!(
        invalid_prediction.step(1. / 480., Frame::IDENTITY, DVec3::ZERO),
        Err(TissueError::InvertedTet(_))
    ));
    assert_eq!(invalid_prediction.positions, before.positions);
    assert_eq!(invalid_prediction.velocities, before.velocities);
    assert_eq!(
        invalid_prediction.previous.as_ref().unwrap().positions,
        before.previous.as_ref().unwrap().positions
    );
    let restart = IntegrationStep::new(&tissue, 1. / 120., DVec3::ZERO);
    assert_eq!(restart.dt, 1. / 120.);
    assert_eq!(restart.baseline, tissue.positions);
    let mut without_history = tissue.clone();
    without_history.previous = None;
    tissue
        .step(1. / 120., Frame::IDENTITY, DVec3::ZERO)
        .unwrap();
    without_history
        .step(1. / 120., Frame::IDENTITY, DVec3::ZERO)
        .unwrap();
    assert_eq!(tissue.positions, without_history.positions);
    assert_eq!(tissue.velocities, without_history.velocities);
}

#[test]
fn exhausted_nonlinear_budget_rejects_without_committing_state_or_history() {
    let mut tissue = tests::cube(Frame::IDENTITY, true);
    tissue.material.time_integration = TimeIntegration::Bdf2;
    tissue
        .step(1. / 480., Frame::IDENTITY, DVec3::NEG_Y * 9.81)
        .unwrap();
    tissue.material.iterations = 1;
    let before = tissue.clone();
    let result = tissue.step(
        1. / 60.,
        Frame {
            translation: DVec3::Y * 0.002,
            ..Frame::IDENTITY
        },
        DVec3::NEG_Y * 9.81,
    );
    assert!(
        matches!(result, Err(TissueError::NonlinearSolveFailed { .. })),
        "{result:?}"
    );
    assert_eq!(tissue.positions, before.positions);
    assert_eq!(tissue.velocities, before.velocities);
    assert_eq!(tissue.frame.translation, before.frame.translation);
    assert_eq!(tissue.frame.rotation, before.frame.rotation);
    let history = tissue.previous.as_ref().unwrap();
    let old = before.previous.as_ref().unwrap();
    assert_eq!(history.positions, old.positions);
    assert_eq!(history.velocities, old.velocities);
    assert_eq!(history.dt, old.dt);
}

#[test]
fn continuum_viscosity_converges_to_an_independent_nonlinear_damped_oscillator() {
    use bevy::math::DVec2;
    let length = 0.1;
    let rest = vec![
        DVec3::ZERO,
        DVec3::X * length,
        DVec3::Y * length,
        DVec3::Z * length,
    ];
    let mut material = tests::cube(Frame::IDENTITY, false).material;
    material.time_integration = TimeIntegration::Bdf2;
    material.shear_pa = 1000.;
    material.bulk_pa = 3000.;
    material.strain_damping_per_second = 0.;
    material.viscosity = Some(Viscosity {
        shear_pa_s: 10.,
        bulk_pa_s: 40.,
    });
    material.iterations = 32;
    let original = Tissue::new(
        rest.clone(),
        rest,
        vec![[0, 1, 2, 3]],
        vec![
            Attachment::Fixed,
            Attachment::Free,
            Attachment::Fixed,
            Attachment::Fixed,
        ],
        material,
        Frame::IDENTITY,
    )
    .unwrap();
    // Fixed base + axial apex motion is exactly one degree of freedom.
    // Corotated elasticity is linear here. With s=1+x/L, the isochoric
    // Cauchy-Green diagonal is (s^(4/3), s^(-2/3), s^(-2/3)); log J = log s.
    // Differentiating these scalar coordinates gives the damping below.
    // RK4 resolves the independent ODE x'' = -w² x - c(s)/m x'
    // at <=10 microseconds; it does not call any tissue force implementation.
    let volume = length.powi(3) / 6.;
    let omega2 = volume * (material.lambda() + 2. * material.shear_pa)
        / (length * length * original.masses[1]);
    let damping_scale = volume / (length * length * original.masses[1]);
    let derivative = |p: DVec2| {
        let s = 1. + p.x / length;
        let damping = damping_scale
            * (10. * (8. / 9. * s.powf(2. / 3.) + 4. / 9. * s.powf(-10. / 3.)) + 40. / (s * s));
        DVec2::new(p.y, -omega2 * p.x - damping * p.y)
    };
    let mut errors = Vec::new();
    for rate in [240, 480, 960] {
        let mut tissue = original.clone();
        tissue.positions[1].x += 0.001;
        let mut reference = DVec2::new(0.001, 0.);
        let dt = 1. / rate as f64;
        let n = (dt / 1e-5).ceil() as usize;
        let h = dt / n as f64;
        let mut maximum = 0.0_f64;
        for _ in 0..rate / 2 {
            for _ in 0..n {
                let a = derivative(reference);
                let b = derivative(reference + 0.5 * h * a);
                let c = derivative(reference + 0.5 * h * b);
                let d = derivative(reference + h * c);
                reference += h / 6. * (a + 2. * b + 2. * c + d);
            }
            tissue.step(dt, Frame::IDENTITY, DVec3::ZERO).unwrap();
            let error = DVec2::new(
                tissue.positions[1].x - length - reference.x,
                (tissue.velocities[1].x - reference.y) / omega2.sqrt(),
            )
            .length();
            maximum = maximum.max(error);
            assert!(tissue.positions[1].y.abs() + tissue.positions[1].z.abs() < 1e-12);
        }
        errors.push(maximum);
        println!(
            "TISSUE_VISCOUS_REFERENCE {}",
            serde_json::json!({"rate_hz":rate,"maximum_state_error_mm":maximum*1000.})
        );
    }
    assert!(
        errors[1] < 0.35 * errors[0] && errors[2] < 0.35 * errors[1],
        "{errors:?}"
    );
    assert!(errors[2] < 2e-6, "{errors:?}");
}
