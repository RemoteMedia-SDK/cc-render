use super::*;

fn nodal_cube(attached: bool) -> Tissue {
    cube_with_pressure(attached, PressureDiscretization::AverageNodal)
}

pub(super) fn cube_with_pressure(attached: bool, pressure: PressureDiscretization) -> Tissue {
    let old = super::super::tests::cube(Frame::IDENTITY, attached);
    let material = Material {
        pressure_discretization: pressure,
        viscosity: Some(Viscosity { shear_pa_s: 15., bulk_pa_s: 5000. }),
        strain_damping_per_second: 0.,
        strain_stiffening: 8.,
        time_integration: TimeIntegration::Bdf2,
        iterations: 32,
        ..old.material
    };
    Tissue::new(old.rest.clone(), old.rest, old.tets.iter().map(|t| t.ids).collect(),
        old.attachments, material, Frame::IDENTITY).unwrap()
}

#[test]
fn nodal_pressure_preserves_affine_energy_and_relaxes_nonuniform_volume() {
    let tissue = nodal_cube(false);
    let f = DMat3::from_cols(DVec3::new(1.1, 0.2, 0.), DVec3::Y * 0.95, DVec3::Z * 1.02);
    let mut x: Vec<_> = tissue.rest.iter().map(|p| f * *p).collect();
    let volume: f64 = tissue.tets.iter().map(|t| t.volume).sum();
    let damping = pressure::Damping { history_log_volume: -0.01, bulk: 150000. };
    let energy = |x: &[DVec3]| tissue.pressure_patches.iter()
        .map(|p| p.energy(&tissue.tets, x, tissue.material.lambda(), damping)).sum::<f64>();
    let expected = 0.5 * volume * (tissue.material.lambda() * (f.determinant() - 1.).powi(2)
        + damping.bulk * (f.determinant().ln() + 0.01).powi(2));
    assert!((energy(&x) - expected).abs() < 1e-12);
    let rotation = DQuat::from_rotation_y(0.8);
    let moved: Vec<_> = x.iter().map(|p| rotation * *p + DVec3::new(2., -1., 3.)).collect();
    assert!((energy(&moved) - expected).abs() < 1e-11);
    x[1] += DVec3::new(0.002, -0.001, 0.003);
    let nodal: f64 = tissue.pressure_patches.iter()
        .map(|p| p.energy(&tissue.tets, &x, tissue.material.lambda(), pressure::Damping::default())).sum();
    let element: f64 = tissue.tets.iter().map(|t| 0.5 * t.volume * tissue.material.lambda()
        * (t.deformation(&x).determinant() - 1.).powi(2)).sum();
    assert!(nodal < element * 0.999);
}

#[test]
fn nodal_pressure_full_objective_gradient_includes_viscosity_and_fixed_elimination() {
    for mode in [PressureDiscretization::AverageNodal, PressureDiscretization::AverageNodalBoundary] {
        check_full_objective_gradient(mode, 0.);
    }
}

fn check_full_objective_gradient(mode: PressureDiscretization, stabilization: f64) {
    let mut tissue = cube_with_pressure(true, mode);
    tissue.material.pressure_stabilization_pa = stabilization;
    check_objective_gradient(tissue);
}

pub(super) fn check_objective_gradient(mut tissue: Tissue) {
    tissue.positions[5] += DVec3::new(0.001, -0.002, 0.0003);
    tissue.previous = Some(History { positions: tissue.rest.clone(), velocities: tissue.velocities.clone(), dt: 1./240. });
    let step = IntegrationStep::new(&tissue, 1./300., DVec3::NEG_Y * 9.81);
    let mut x = tissue.positions.clone();
    x[7] += DVec3::new(-0.0002, 0.0001, 0.0008);
    let system = tissue.linearize(&x, &step.predicted, Frame::IDENTITY, step.dt, &[], &step.damping);
    let energy = |p: &[DVec3]| tissue.objective(p, &step.predicted, Frame::IDENTITY, step.dt)
        + step.damping.energy(p, &tissue.tets, &tissue.pressure_patches);
    for node in 0..x.len() {
        if tissue.inverse_mass[node] == 0. {
            assert_eq!(system.gradient[node], DVec3::ZERO);
            continue;
        }
        for axis in 0..3 {
            let eps = 1e-7;
            x[node][axis] += eps;
            let plus = energy(&x);
            x[node][axis] -= 2. * eps;
            let minus = energy(&x);
            x[node][axis] += eps;
            assert!(((plus-minus)/(2.*eps)-system.gradient[node][axis]).abs() < 1e-5);
        }
    }
    let solution = system.direction().unwrap();
    let residual: Vec<_> = system.multiply(&solution).iter().zip(&system.gradient).map(|(p,g)| *p+*g).collect();
    assert!(dot(&residual, &residual) < dot(&system.gradient, &system.gradient) * 1e-16);
}

#[test]
fn nodal_pressure_couplings_match_independent_directional_derivatives() {
    let tissue = nodal_cube(false);
    let mut shear_only = tissue.clone();
    shear_only.pressure_patches.clear();
    let step = IntegrationStep::new(&tissue, 1./240., DVec3::ZERO);
    let mut damping = IntegrationStep::new(&tissue, 1./240., DVec3::ZERO).damping;
    damping.pressures.clear();
    let pressure_gradient = |x: &[DVec3]| {
        let full = tissue.linearize(x, &step.predicted, Frame::IDENTITY, step.dt, &[], &step.damping);
        let base = shear_only.linearize(x, &step.predicted, Frame::IDENTITY, step.dt, &[], &damping);
        full.gradient.iter().zip(base.gradient).map(|(a,b)| *a-b).collect::<Vec<_>>()
    };
    let full = tissue.linearize(&tissue.positions, &step.predicted, Frame::IDENTITY, step.dt, &[], &step.damping);
    let base = shear_only.linearize(&tissue.positions, &step.predicted, Frame::IDENTITY, step.dt, &[], &damping);
    let direction: Vec<_> = (0..tissue.positions.len()).map(|i| DVec3::new((i as f64).sin(), (i as f64).cos(), 0.2*i as f64)).collect();
    let a = full.multiply(&direction);
    let b = base.multiply(&direction);
    let eps = 1e-7;
    let plus = pressure_gradient(&tissue.positions.iter().zip(&direction).map(|(p,d)| *p+eps**d).collect::<Vec<_>>());
    let minus = pressure_gradient(&tissue.positions.iter().zip(&direction).map(|(p,d)| *p-eps**d).collect::<Vec<_>>());
    for i in 0..a.len() {
        assert!(((plus[i]-minus[i])/(2.*eps)-(a[i]-b[i])).length() < 1e-5);
    }
    assert!(dot(&direction, &a) > 0.);
}

#[test]
fn compliant_rim_keeps_the_fixed_rim_pressure_partition() {
    let fixed = cube_with_pressure(true, PressureDiscretization::AverageNodalBoundary);
    let attachments: Vec<_> = fixed.attachments.iter().map(|a| match a {
        Attachment::Fixed => Attachment::Compliant { compliance_m_per_n: 1e-6 },
        other => *other,
    }).collect();
    let compliant = Tissue::new(fixed.rest.clone(), fixed.rest.clone(), fixed.tets.iter().map(|t| t.ids).collect(),
        attachments, fixed.material, Frame::IDENTITY).unwrap();
    assert!(fixed.element_pressure.iter().any(|local| *local));
    assert_eq!(compliant.element_pressure, fixed.element_pressure);
    assert_eq!(compliant.pressure_patches.len(), fixed.pressure_patches.len());
}

#[test]
fn fixed_edge_pressure_partition_preserves_homogeneous_material_energy() {
    let tissue = cube_with_pressure(true, PressureDiscretization::AverageNodalBoundary);
    let partition: f64 = tissue.pressure_patches.iter().map(|p| p.volume).sum::<f64>()
        + tissue.tets.iter().zip(&tissue.element_pressure).filter(|(_,local)| **local).map(|(t,_)| t.volume).sum::<f64>();
    let total: f64 = tissue.tets.iter().map(|t| t.volume).sum();
    assert!((partition-total).abs() < 1e-14);
    assert!(tissue.element_pressure.iter().any(|local| *local));
    assert!(tissue.element_pressure.iter().any(|local| !*local));
    for (tet, local) in tissue.tets.iter().zip(&tissue.element_pressure) {
        assert_eq!(*local,tet.ids.iter().filter(|&&i| !matches!(tissue.attachments[i],Attachment::Free)).count() >= 2);
    }
    let f = DMat3::from_cols(DVec3::new(1.05,0.1,0.), DVec3::Y*0.95, DVec3::Z*1.02);
    let x: Vec<_> = tissue.rest.iter().map(|p| f**p).collect();
    let expected: f64 = tissue.tets.iter().map(|t| t.isochoric_energy(&x,tissue.material.shear_pa,tissue.material.strain_stiffening)).sum::<f64>()
        + 0.5*tissue.material.bulk_pa*total*(f.determinant()-1.).powi(2);
    assert!((expected-tissue.elastic_energy(&x)).abs() < 1e-12);
}

#[test]
fn stabilized_pressure_preserves_affine_energy_and_penalizes_only_volume_variance() {
    for mode in [PressureDiscretization::AverageNodal, PressureDiscretization::AverageNodalBoundary] {
        let baseline = cube_with_pressure(true, mode);
        let mut stabilized = baseline.clone();
        stabilized.material.pressure_stabilization_pa = 2000.;
        let f = DMat3::from_cols(DVec3::new(1.05, 0.1, 0.), DVec3::Y * 0.95, DVec3::Z * 1.02);
        let mut x: Vec<_> = baseline.rest.iter().map(|p| f * *p).collect();
        assert!((stabilized.elastic_energy(&x) - baseline.elastic_energy(&x)).abs() < 1e-12);
        x[7] += DVec3::new(0.004, -0.002, 0.003);
        // Independently sum the weighted variance of element Jacobians in
        // each nodal patch. Fixed-edge elements are outside this partition.
        let mut variance = 0.;
        for node in 0..x.len() {
            let incident: Vec<_> = baseline.tets.iter().zip(&baseline.element_pressure)
                .filter(|(tet, local)| !**local && tet.ids.contains(&node)).collect();
            let weight: f64 = incident.iter().map(|(tet, _)| tet.volume / 4.).sum();
            if weight == 0. { continue; }
            let mean: f64 = incident.iter().map(|(tet, _)| tet.volume / 4.
                * tet.deformation(&x).determinant()).sum::<f64>() / weight;
            variance += incident.iter().map(|(tet, _)| tet.volume / 4.
                * (tet.deformation(&x).determinant() - mean).powi(2)).sum::<f64>();
        }
        let extra = stabilized.elastic_energy(&x) - baseline.elastic_energy(&x);
        assert!(extra > 1e-8);
        assert!((extra - 0.5 * 2000. * variance).abs() < 1e-12);
        let rotation = DQuat::from_rotation_y(0.8);
        let moved: Vec<_> = x.iter().map(|p| rotation * *p + DVec3::new(2., -1., 3.)).collect();
        assert!((stabilized.elastic_energy(&moved) - stabilized.elastic_energy(&x)).abs() < 1e-10);
        stabilized.material.pressure_stabilization_pa = stabilized.material.bulk_pa;
        let fully_local: f64 = stabilized.tets.iter().map(|tet|
            tet.isochoric_energy(&x, stabilized.material.shear_pa, stabilized.material.strain_stiffening)
            + 0.5 * tet.volume * stabilized.material.bulk_pa
                * (tet.deformation(&x).determinant() - 1.).powi(2)).sum();
        assert!((stabilized.elastic_energy(&x) - fully_local).abs() < 1e-12);
    }
}

#[test]
fn stabilized_pressure_gradient_includes_both_partitions_and_viscosity() {
    for mode in [PressureDiscretization::AverageNodal, PressureDiscretization::AverageNodalBoundary] {
        for stabilization in [2000., 30000.] {
            check_full_objective_gradient(mode, stabilization);
        }
    }
}

#[test]
fn pressure_stabilization_rejects_invalid_shares_and_defaults_to_zero() {
    let material = nodal_cube(false).material;
    for value in [-1., material.bulk_pa + 1., f64::NAN, f64::INFINITY] {
        assert!(Material { pressure_stabilization_pa: value, ..material }.validate().is_err());
    }
    assert!(Material { pressure_discretization: PressureDiscretization::Element,
        pressure_stabilization_pa: 1., ..material }.validate().is_err());
    for value in [0., material.bulk_pa] {
        assert!(Material { pressure_stabilization_pa: value, ..material }.validate().is_ok());
    }
    let mut encoded = serde_json::to_value(material).unwrap();
    encoded.as_object_mut().unwrap().remove("pressure_stabilization_pa");
    let decoded: Material = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.pressure_stabilization_pa, 0.);
}
