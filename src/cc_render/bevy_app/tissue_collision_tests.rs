use super::*;

fn plane(z: f64) -> [DVec3; 3] {
    [
        DVec3::new(-1.0, -1.0, z),
        DVec3::new(1.0, -1.0, z),
        DVec3::new(0.0, 1.0, z),
    ]
}

#[test]
fn tissue_sweep_detects_crossing_with_clear_endpoints_and_is_frame_covariant() {
    let [a, b, c] = plane(0.1);
    let old = [DVec3::ZERO, a, b, c];
    let end = [
        DVec3::ZERO,
        a - DVec3::Z * 0.2,
        b - DVec3::Z * 0.2,
        c - DVec3::Z * 0.2,
    ];
    let hit = sweep(old, end, CLEARANCE).unwrap().unwrap();
    // Conservative advancement stops at most SWEEP_TOLERANCE before contact.
    assert!(hit.distance >= CLEARANCE - 2e-8 && hit.distance <= CLEARANCE + SWEEP_TOLERANCE);
    assert!(hit.normal.dot(DVec3::NEG_Z) > 0.999999);
    let rotation = bevy::math::DQuat::from_rotation_y(0.7);
    let transform = |p: DVec3| rotation * p + DVec3::new(18.0, -24.0, 30.0);
    let moved = sweep(old.map(transform), end.map(transform), CLEARANCE)
        .unwrap()
        .unwrap();
    assert!((moved.distance - hit.distance).abs() < 1e-10);
    assert!(moved.normal.distance(rotation * hit.normal) < 1e-8);
}

#[test]
fn tissue_sweep_separates_parallel_motion_and_rejects_ambiguous_initial_intersection() {
    let [a, b, c] = plane(0.1);
    let old = [DVec3::ZERO, a, b, c];
    assert!(sweep(old, old.map(|p| p + DVec3::X), CLEARANCE)
        .unwrap()
        .is_none());
    let [a, b, c] = plane(0.0);
    let invalid = [DVec3::ZERO, a, b, c];
    assert!(sweep(invalid, invalid, CLEARANCE)
        .unwrap_err()
        .is::<ContactUnresolved>());
}

fn contact_fixture(free_weight: f64) -> (TissueAsset, BodyContacts) {
    // The original penalty tests run uncapped; the pressure cap has its own.
    contact_fixture_with(free_weight, ContactPolicy { max_pressure_pa: f64::INFINITY, strict: false })
}

fn contact_fixture_with(free_weight: f64, policy: ContactPolicy) -> (TissueAsset, BodyContacts) {
    let mut asset = super::super::tissue::tests::fixture_asset();
    asset.regions[0].material.strain_damping_per_second = 0.0;
    asset.regions[0].material.iterations = 8;
    let contact = BodyContacts {
        triangles: vec![[1, 2, 3]],
        samples: vec![SkinSample {
            vertex: 0,
            area: 1e-4,
            binding: VertexBinding {
                vertex: 0,
                region: 0,
                tetrahedron: 0,
                barycentric: [1.0 - free_weight, 0.0, 0.0, free_weight],
                influence: 1.0,
            },
        }],
        policy,
    };
    (asset, contact)
}

fn contact_pose(z: f64) -> ContactPose {
    ContactPose {
        frames: vec![Frame::IDENTITY],
        skin: [vec![DVec3::Z * 0.1], plane(z).to_vec()].concat(),
    }
}

#[test]
fn tissue_moving_skinned_triangle_compresses_embedded_tissue_with_bounded_overlap() {
    let (asset, contact) = contact_fixture(1.0);
    let original = vec![asset.regions[0].solver(Frame::IDENTITY).unwrap()];
    let unchanged = original[0].positions().to_vec();
    let mut candidate = original.clone();
    candidate[0]
        .step(1.0 / 120.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81)
        .unwrap();
    let free = candidate[0].positions().to_vec();
    contact
        .resolve(
            &original,
            &mut candidate,
            &asset,
            &contact_pose(0.12),
            &contact_pose(0.12),
            1.0 / 120.0,
        )
        .unwrap();
    assert_eq!(candidate[0].positions(), free);
    let metrics = contact
        .resolve(
            &original,
            &mut candidate,
            &asset,
            &contact_pose(0.12),
            &contact_pose(0.09),
            1.0 / 120.0,
        )
        .unwrap();
    assert!(metrics[0].active_contacts > 0);
    assert!(metrics[0].maximum_penetration_m <= RESIDUAL);
    assert!(candidate[0].positions()[3].z < 0.09);
    assert!(candidate[0].metrics().minimum_volume_ratio > 0.8);
    assert_eq!(original[0].positions(), unchanged);
    assert_eq!(&candidate[0].positions()[..3], &unchanged[..3]);
}

#[test]
fn tissue_contact_enforces_clearance_at_partly_fixed_skin() {
    use crate::cc_render::soft_tissue::{PressureDiscretization, TimeIntegration, Viscosity};
    let (mut asset, contact) = contact_fixture(0.22316298434880016);
    let material = &mut asset.regions[0].material;
    material.pressure_discretization = PressureDiscretization::AverageNodalBoundary;
    material.pressure_stabilization_pa = 8000.0;
    material.element_pressure_quality_cutoff = 0.15;
    material.time_integration = TimeIntegration::Bdf2;
    material.viscosity = Some(Viscosity {
        shear_pa_s: 5.0,
        bulk_pa_s: 5000.0,
    });
    material.shear_pa = 500.0;
    material.bulk_pa = 500000.0;
    material.strain_stiffening = 8.0;
    material.iterations = 32;
    // A partially fixed embedding reduces normal mobility. Smaller timesteps
    // increase inertial resistance, so subdivision alone cannot fix the penalty.
    for dt in [1.0 / 240.0, 1.0 / 960.0] {
        let original = vec![asset.regions[0].solver(Frame::IDENTITY).unwrap()];
        let unchanged = original[0].positions().to_vec();
        let mut candidate = original.clone();
        candidate[0]
            .step(dt, Frame::IDENTITY, DVec3::NEG_Y * 9.81)
            .unwrap();
        let metrics = contact
            .resolve(
                &original,
                &mut candidate,
                &asset,
                &contact_pose(0.102),
                &contact_pose(0.099),
                dt,
            )
            .unwrap();
        assert!(metrics[0].active_contacts > 0 && metrics[0].normal_force_n > 0.0);
        assert!(metrics[0].maximum_penetration_m <= RESIDUAL, "{metrics:?}");
        let (point, _) = sample_position(
            &contact.samples[0],
            &asset,
            &contact_pose(0.099),
            &candidate[0],
        );
        assert!(point.z <= 0.099 - CLEARANCE + RESIDUAL);
        assert_eq!(original[0].positions(), unchanged);
        assert_eq!(&candidate[0].positions()[..3], &unchanged[..3]);
        assert!(candidate[0].metrics().minimum_volume_ratio > 0.9);
    }
}

#[test]
fn tissue_contact_rejects_kinematic_skin_overlap() {
    let (asset, contact) = contact_fixture(0.0);
    let original = vec![asset.regions[0].solver(Frame::IDENTITY).unwrap()];
    let unchanged = original[0].positions().to_vec();
    let mut candidate = original.clone();
    candidate[0]
        .step(1.0 / 240.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81)
        .unwrap();
    let error = contact
        .resolve(
            &original,
            &mut candidate,
            &asset,
            &contact_pose(0.102),
            &contact_pose(0.099),
            1.0 / 240.0,
        )
        .unwrap_err();
    assert!(error.is::<ContactUnresolved>());
    assert_eq!(original[0].positions(), unchanged);
}

#[test]
fn tissue_contact_pressure_cap_bounds_force_and_reports_permitted_overlap() {
    // A 30 mm intrusion at the uncapped penalty is resolved by force; with a
    // bounded pressure the reaction stays at the cap and overlap is reported.
    let policy = ContactPolicy { max_pressure_pa: 2_000.0, strict: false };
    let (asset, contact) = contact_fixture_with(1.0, policy);
    let original = vec![asset.regions[0].solver(Frame::IDENTITY).unwrap()];
    let unchanged = original[0].positions().to_vec();
    let mut candidate = original.clone();
    candidate[0].step(1.0 / 120.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81).unwrap();
    let metrics = contact
        .resolve(&original, &mut candidate, &asset, &contact_pose(0.12), &contact_pose(0.09), 1.0 / 120.0)
        .unwrap();
    let cap = policy.max_pressure_pa * contact.samples[0].area;
    assert!(metrics[0].force_limited_contacts > 0, "{metrics:?}");
    assert!(metrics[0].normal_force_n <= cap * (1.0 + 1e-12), "{metrics:?}");
    assert!(metrics[0].maximum_force_limited_penetration_m > RESIDUAL);
    assert_eq!(metrics[0].maximum_penetration_m, metrics[0].maximum_force_limited_penetration_m);
    assert_eq!(original[0].positions(), unchanged);
    assert_eq!(&candidate[0].positions()[..3], &unchanged[..3]);
    // Strict mode rejects the same permitted overlap instead of reporting it.
    let (_, strict) = contact_fixture_with(1.0, ContactPolicy { strict: true, ..policy });
    let mut candidate = original.clone();
    candidate[0].step(1.0 / 120.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81).unwrap();
    let error = strict
        .resolve(&original, &mut candidate, &asset, &contact_pose(0.12), &contact_pose(0.09), 1.0 / 120.0)
        .unwrap_err();
    assert!(error.is::<ContactUnresolved>());
}

#[test]
fn tissue_sweep_terminates_on_recorded_grazing_motion() {
    // Seed-739 replay with a 10 kPa cap, 7881.20 ms: the sample slides along a
    // hand face at almost exactly the clearance. A 1e-8 m tolerance exhausted
    // 256 conservative-advancement iterations 1.4e-8 m short of contact.
    let old = [
        DVec3::new(-0.06817058812314611, 1.2179012734564287, 0.153855322000856),
        DVec3::new(-0.06455394625663757, 1.217216968536377, 0.15519660711288452),
        DVec3::new(-0.06819744408130646, 1.2204720973968506, 0.15317469835281372),
        DVec3::new(-0.06883394718170166, 1.2175859212875366, 0.15448200702667236),
    ];
    let end = [
        DVec3::new(-0.06816648372460249, 1.2179038455551892, 0.15386175961990448),
        DVec3::new(-0.06453630328178406, 1.2172183990478516, 0.15520501136779785),
        DVec3::new(-0.06818009912967682, 1.220473289489746, 0.15318351984024048),
        DVec3::new(-0.06881639361381531, 1.2175869941711426, 0.15449070930480957),
    ];
    let hit = sweep(old, end, CLEARANCE).unwrap().expect("grazing sample is within tolerance");
    assert!(hit.distance <= CLEARANCE + SWEEP_TOLERANCE);
}

#[test]
fn tissue_contact_excuses_only_receding_inherited_overlap() {
    // The sample starts 0.35 mm from the face, inside the inner band. When
    // the face recedes to 0.45 mm the step is valid even without a pressure
    // cap; when it stays, the uncapped penalty must still clear it.
    let (asset, contact) = contact_fixture(1.0);
    let original = vec![asset.regions[0].solver(Frame::IDENTITY).unwrap()];
    let mut candidate = original.clone();
    candidate[0].step(1.0 / 240.0, Frame::IDENTITY, DVec3::NEG_Y * 9.81).unwrap();
    let metrics = contact
        .resolve(&original, &mut candidate, &asset, &contact_pose(0.10035), &contact_pose(0.10045), 1.0 / 240.0)
        .unwrap();
    assert!(metrics[0].maximum_penetration_m <= RESIDUAL, "{metrics:?}");
    assert_eq!(metrics[0].force_limited_contacts, 0);
}

#[test]
fn tissue_local_swept_tree_matches_exhaustive_queries_after_motion_and_rewinds() {
    use parry_ccd::bounding_volume::BoundingVolume;
    let triangles: Vec<_> = (0..73).map(|i| [i * 3, i * 3 + 1, i * 3 + 2]).collect();
    let pose = |step: usize| -> Vec<DVec3> {
        (0..73)
            .flat_map(|i| {
                let shift = DVec3::new(
                    ((i * 17 + step * 13) % 73) as f64 * 0.07,
                    ((i * 11 + step * 7) % 19) as f64 * 0.04,
                    (step as f64 * 0.3 + i as f64).sin() * 0.08,
                );
                [shift, shift + DVec3::X * 0.03, shift + DVec3::Y * 0.03]
            })
            .collect()
    };
    // Include a repeated interval, a large jump, and backwards samples. A cache
    // must remain conservative regardless of the physics caller's commit order.
    for (a, b) in [(0, 1), (1, 2), (1, 2), (2, 48), (48, 49), (1, 0), (0, 0)] {
        let old = pose(a);
        let end = pose(b);
        let exact: Vec<_> = triangles
            .iter()
            .map(|t| swept_bounds(t.iter().flat_map(|v| [old[*v], end[*v]]), CLEARANCE))
            .collect();
        let vertices: Vec<_> = old
            .iter()
            .zip(&end)
            .map(|(a, b)| swept_bounds([*a, *b], CLEARANCE))
            .collect();
        for i in [0, 36, 153, 216] {
            let points = [old[i], old[i + 1], end[i], end[i + 1]];
            let region = swept_bounds(points, CLEARANCE);
            let tree = LocalTree::new(&triangles, &vertices, &region);
            if a == b {
                assert!(tree.faces.len() < triangles.len());
            }
            let mut hits = vec![usize::MAX];
            for point in points {
                let query = swept_bounds([point], CLEARANCE);
                tree.query(&query, &mut hits);
                let expected: Vec<_> = exact
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| b.intersects(&query).then_some(i))
                    .collect();
                assert_eq!(hits, expected, "interval {a}..{b}");
            }
        }
    }
    let query = swept_bounds([DVec3::ZERO], CLEARANCE);
    let empty = LocalTree::new(&[], &[], &query);
    let mut hits = Vec::new();
    for _ in 0..2 {
        empty.query(&query, &mut hits);
        assert!(hits.is_empty());
    }
}
