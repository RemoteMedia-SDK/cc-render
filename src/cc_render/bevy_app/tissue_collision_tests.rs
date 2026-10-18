use super::*;
use crate::cc_render::soft_tissue::{Attachment, Material, RegionAsset};

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
    assert!((hit.distance - CLEARANCE).abs() < 2e-8);
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

#[test]
fn tissue_moving_skinned_triangle_compresses_embedded_tissue_with_bounded_overlap() {
    let initial = vec![
        [0.0, 0.0, 0.0],
        [0.1, 0.0, 0.0],
        [0.0, 0.1, 0.0],
        [0.0, 0.0, 0.1],
    ];
    let asset = TissueAsset {
        schema_version: 1,
        source_sha256: vec![],
        surfaces: vec![],
        contact_surfaces: vec![],
        contact_inactive_morph_targets: vec![],
        regions: vec![RegionAsset {
            name: "test".into(),
            anchor_bone: "test".into(),
            rest_positions: initial.clone(),
            initial_positions: initial,
            tetrahedra: vec![[0, 1, 2, 3]],
            attachments: vec![
                Attachment::Fixed,
                Attachment::Fixed,
                Attachment::Fixed,
                Attachment::Free,
            ],
            material: Material {
                time_integration: Default::default(),
                viscosity: None,
                density_kg_m3: 950.0,
                shear_pa: 1500.0,
                strain_stiffening: 0.0,
                bulk_pa: 30000.0,
                strain_damping_per_second: 0.0,
                iterations: 8,
            },
        }],
    };
    let contact = BodyContacts {
        triangles: vec![[1, 2, 3]],
        samples: vec![SkinSample {
            vertex: 0,
            area: 1e-4,
            binding: VertexBinding {
                vertex: 0,
                region: 0,
                tetrahedron: 0,
                barycentric: [0.0, 0.0, 0.0, 1.0],
                influence: 1.0,
            },
        }],
    };
    let pose = |z| ContactPose {
        frames: vec![Frame::IDENTITY],
        skin: [vec![DVec3::Z * 0.1], plane(z).to_vec()].concat(),
    };
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
            &pose(0.12),
            &pose(0.12),
            1.0 / 120.0,
        )
        .unwrap();
    assert_eq!(candidate[0].positions(), free);
    let metrics = contact
        .resolve(
            &original,
            &mut candidate,
            &asset,
            &pose(0.12),
            &pose(0.09),
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
