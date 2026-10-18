use super::*;

#[test]
fn mean_ratio_quality_is_scale_and_rigid_invariant_and_detects_flat_cells() {
    let points = [DVec3::ZERO, DVec3::X,
        DVec3::new(0.5, 3_f64.sqrt()/2., 0.),
        DVec3::new(0.5, 3_f64.sqrt()/6., (2_f64/3.).sqrt())];
    assert!((pressure::mean_ratio_quality(points)-1.).abs() < 1e-14);
    for scale in [0.001, 4.2] {
        let rotated = points.map(|p| DQuat::from_rotation_y(0.8)*(scale*p)+DVec3::new(2., -1., 3.));
        assert!((pressure::mean_ratio_quality(rotated)-1.).abs() < 1e-10);
    }
    let flat = points.map(|p| p*DVec3::new(1., 1., 0.001));
    assert!(pressure::mean_ratio_quality(flat) < 0.02);
}

#[test]
fn quality_partition_uses_authored_geometry_and_preserves_affine_bulk_energy() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),
        "/assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json")).unwrap();
    let asset = crate::cc_render::soft_tissue::TissueAsset::from_json(&bytes).unwrap();
    for (region, additional) in asset.regions.iter().zip([8, 7]) {
        let tissue = region.solver(Frame::IDENTITY).unwrap();
        let mut original = tissue.material;
        original.element_pressure_quality_cutoff = 0.;
        let base = pressure::element_partition(&tissue.tets, &tissue.attachment_positions, &tissue.attachments, original);
        assert_eq!(tissue.element_pressure.iter().zip(base).filter(|(now, old)| **now && !old).count(), additional);
        let changed_rest = tissue.rest.iter().map(|p| *p*DVec3::new(0.93, 1.02, 1.01)).collect();
        let changed = Tissue::new(changed_rest, tissue.attachment_positions.clone(),
            tissue.tets.iter().map(|t| t.ids).collect(), tissue.attachments.clone(), tissue.material,
            Frame { translation: DVec3::new(2., -1., 3.), rotation: DQuat::from_rotation_y(0.8) }).unwrap();
        assert_eq!(tissue.element_pressure, changed.element_pressure);
        let f = DMat3::from_cols(DVec3::new(1.05, 0.1, 0.), DVec3::Y*0.95, DVec3::Z*1.02);
        let x: Vec<_> = tissue.rest.iter().map(|p| f**p).collect();
        let expected: f64 = tissue.tets.iter().map(|t|
            t.isochoric_energy(&x, tissue.material.shear_pa, tissue.material.strain_stiffening)
            + 0.5*t.volume*tissue.material.bulk_pa*(f.determinant()-1.).powi(2)).sum();
        assert!((expected-tissue.elastic_energy(&x)).abs() < 1e-11);
    }
}

#[test]
fn combined_pressure_has_consistent_full_objective_forces() {
    let base = super::pressure_tests::cube_with_pressure(true, PressureDiscretization::AverageNodalBoundary);
    let mut initial = base.rest.clone();
    initial[7].z = 0.06;
    let material = Material { element_pressure_quality_cutoff: 0.7,
        pressure_stabilization_pa: 8000., ..base.material };
    let tissue = Tissue::new(base.rest, initial, base.tets.iter().map(|t| t.ids).collect(),
        base.attachments, material, Frame::IDENTITY).unwrap();
    assert_eq!(tissue.element_pressure, vec![true, true, true, true, true, false]);
    super::pressure_tests::check_objective_gradient(tissue);
}

#[test]
fn quality_cutoff_rejects_invalid_modes_and_defaults_to_zero() {
    let material = super::pressure_tests::cube_with_pressure(true, PressureDiscretization::AverageNodalBoundary).material;
    for value in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
        assert!(Material { element_pressure_quality_cutoff: value, ..material }.validate().is_err());
    }
    for mode in [PressureDiscretization::Element, PressureDiscretization::AverageNodal] {
        assert!(Material { pressure_discretization: mode, element_pressure_quality_cutoff: 0.1, ..material }.validate().is_err());
    }
    for value in [0., 1.] {
        assert!(Material { element_pressure_quality_cutoff: value, ..material }.validate().is_ok());
    }
    let mut encoded = serde_json::to_value(material).unwrap();
    encoded.as_object_mut().unwrap().remove("element_pressure_quality_cutoff");
    let decoded: Material = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.element_pressure_quality_cutoff, 0.);
}
