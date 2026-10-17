use super::*;
use bevy_rapier3d::rapier::{
    na,
    parry::shape::{Ball, Capsule},
};

fn plane() -> DeformedSurface {
    DeformedSurface::new(
        vec![Vec3::ZERO, Vec3::X, Vec3::Z],
        vec![[0, 1, 2]],
        vec![Vec2::ZERO, Vec2::X, Vec2::Y],
        0.001,
    )
    .unwrap()
}

#[test]
fn physical_shell_contact_has_material_witness_but_proximity_does_not() {
    let surface = plane();
    let shape = Ball::new(0.02);
    for sign in [-1., 1.] {
        assert!(surface
            .contacts(&shape, &Isometry::translation(0.25, sign * 0.021, 0.25))
            .unwrap()
            .is_empty());
        let contacts = surface
            .contacts(&shape, &Isometry::translation(0.25, sign * 0.0204, 0.25))
            .unwrap();
        assert_eq!(contacts.len(), 1);
        let contact = &contacts[0];
        assert!(contact.separation() < 0.);
        assert!((contact.midsurface_position() - Vec3::new(0.25, 0., 0.25)).length() < 1e-6);
        assert!((contact.normal() - Vec3::Y * sign).length() < 1e-6);
        let material = surface.material_point_from_contact(contact).unwrap();
        let resolved = surface.resolve(&material).unwrap();
        assert!((resolved.uv - Vec2::splat(0.25)).length() < 1e-6);
        assert!((resolved.position - contact.midsurface_position()).length() < 1e-6);
    }
}

#[test]
fn collider_orientation_is_used_instead_of_a_hand_distance_threshold() {
    let surface = plane();
    let capsule = Capsule::new_y(0.2, 0.02);
    let vertical = Isometry::translation(0.25, 0.05, 0.25);
    let horizontal = Isometry::new(
        na::Vector3::new(0.25, 0.05, 0.25),
        na::Vector3::new(0., 0., std::f32::consts::FRAC_PI_2),
    );
    assert!(!surface.contacts(&capsule, &vertical).unwrap().is_empty());
    assert!(surface.contacts(&capsule, &horizontal).unwrap().is_empty());
}

#[test]
fn contact_revision_expires_but_material_identity_follows_deformation() {
    let mut surface = plane();
    let shape = Ball::new(0.02);
    let contact = surface
        .contacts(&shape, &Isometry::translation(0.25, 0.019, 0.25))
        .unwrap()
        .remove(0);
    let material = surface.material_point_from_contact(&contact).unwrap();
    let old = surface.resolve(&material).unwrap();
    surface
        .update_positions(vec![Vec3::ZERO, Vec3::new(1., 0.2, 0.), Vec3::Z])
        .unwrap();
    assert!(!surface.contact_matches_surface(&contact));
    assert!(surface.material_point_from_contact(&contact).is_none());
    let new = surface.resolve(&material).unwrap();
    assert!((new.position - Vec3::new(0.25, 0.05, 0.25)).length() < 1e-6);
    assert_eq!(old.uv, new.uv);
    assert_eq!(new.revision, 1);
    let other = plane();
    assert!(!other.contact_matches_surface(&contact));
    assert!(other.resolve(&material).is_none());
}

#[test]
fn unchanged_geometry_is_still_a_new_observation_and_queries_do_not_mutate_it() {
    let mut surface = plane();
    let before = surface.positions().to_vec();
    let shape = Ball::new(0.02);
    let contact = surface
        .contacts(&shape, &Isometry::translation(0.25, 0.019, 0.25))
        .unwrap()
        .remove(0);
    assert_eq!(surface.positions(), before);
    assert_eq!(surface.revision(), 0);
    // Surface freshness alone does not certify the other collider stayed put.
    assert!(surface
        .contacts(&shape, &Isometry::translation(0.25, 0.1, 0.25))
        .unwrap()
        .is_empty());
    assert!(surface.contact_matches_surface(&contact));
    surface.update_positions(before).unwrap();
    assert!(!surface.contact_matches_surface(&contact));
}

#[test]
fn folded_layers_keep_distinct_material_contacts() {
    let surface = DeformedSurface::new(
        vec![
            Vec3::ZERO,
            Vec3::X,
            Vec3::Z,
            Vec3::Y * 0.02,
            Vec3::X + Vec3::Y * 0.02,
            Vec3::Z + Vec3::Y * 0.02,
        ],
        vec![[0, 1, 2], [3, 4, 5]],
        vec![
            Vec2::ZERO,
            Vec2::X,
            Vec2::Y,
            Vec2::new(2., 0.),
            Vec2::new(3., 0.),
            Vec2::new(2., 1.),
        ],
        0.001,
    )
    .unwrap();
    let contacts = surface
        .contacts(&Ball::new(0.011), &Isometry::translation(0.25, 0.01, 0.25))
        .unwrap();
    assert_eq!(
        contacts
            .iter()
            .map(SurfaceContact::triangle)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    let uvs = contacts
        .iter()
        .map(|c| {
            surface
                .resolve(&surface.material_point_from_contact(c).unwrap())
                .unwrap()
                .uv
        })
        .collect::<Vec<_>>();
    assert!((uvs[1] - uvs[0] - Vec2::new(2., 0.)).length() < 1e-5);
}

#[test]
fn invalid_geometry_is_rejected_without_advancing_or_partially_updating() {
    let mut surface = plane();
    let before = surface.positions().to_vec();
    for positions in [
        vec![Vec3::ZERO; 3],
        vec![Vec3::NAN, Vec3::X, Vec3::Z],
        vec![Vec3::ZERO; 2],
        vec![Vec3::ZERO, Vec3::X * f32::MAX, Vec3::Z * f32::MAX],
    ] {
        assert!(surface.update_positions(positions).is_err());
        assert_eq!(surface.positions(), before);
        assert_eq!(surface.revision(), 0);
    }
    assert!(
        DeformedSurface::new(before.clone(), vec![[0, 1, 3]], vec![Vec2::ZERO; 3], 0.001).is_err()
    );
    assert!(
        DeformedSurface::new(before.clone(), vec![[0, 1, 2]], vec![Vec2::NAN; 3], 0.001).is_err()
    );
    assert!(DeformedSurface::new(before, vec![[0, 1, 2]], vec![Vec2::ZERO; 3], -0.001).is_err());
    assert!(surface
        .contacts(&Ball::new(0.02), &Isometry::translation(f32::NAN, 0., 0.))
        .is_err());
    assert!(surface
        .contacts(&Ball::new(-0.0001), &Isometry::identity())
        .is_err());
    let invalid_rotation = Isometry::from_parts(
        na::Translation3::identity(),
        na::UnitQuaternion::new_unchecked(na::Quaternion::new(2., 0., 0., 0.)),
    );
    assert!(surface
        .contacts(&Ball::new(0.02), &invalid_rotation)
        .is_err());
}

#[test]
fn observed_condo_sheet_supports_contact_and_material_round_trip() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/interaction-authoring/bedding");
    let asset: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("top-sheet-surface.json")).unwrap())
            .unwrap();
    let positions: Vec<[f32; 3]> =
        serde_json::from_slice(&std::fs::read(root.join("final-positions.json")).unwrap()).unwrap();
    let indices: Vec<[u32; 3]> = serde_json::from_value(asset["triangles"].clone()).unwrap();
    let uv: Vec<[f32; 2]> = serde_json::from_value(asset["material_uv"].clone()).unwrap();
    let selected = indices
        .iter()
        .position(|t| {
            t.iter().all(|i| {
                let p = uv[*i as usize];
                (0.45..=0.55).contains(&p[0]) && (0.45..=0.55).contains(&p[1])
            })
        })
        .unwrap();
    let vertices = indices[selected].map(|i| Vec3::from_array(positions[i as usize]));
    let centre = vertices.into_iter().sum::<Vec3>() / 3.;
    let normal = (vertices[1] - vertices[0])
        .cross(vertices[2] - vertices[0])
        .normalize();
    let surface = DeformedSurface::new(
        positions.into_iter().map(Vec3::from_array).collect(),
        indices,
        uv.into_iter().map(Vec2::from_array).collect(),
        0.001,
    )
    .unwrap();
    let hand = centre + normal * 0.01175;
    let contacts = surface
        .contacts(
            &Ball::new(0.012),
            &Isometry::translation(hand.x, hand.y, hand.z),
        )
        .unwrap();
    let contact = contacts.iter().find(|c| c.triangle() == selected).unwrap();
    let point = surface.material_point_from_contact(contact).unwrap();
    assert!((surface.resolve(&point).unwrap().position - centre).length() < 2e-5);
    let clear = centre + Vec3::Y * 0.2;
    assert!(surface
        .contacts(
            &Ball::new(0.012),
            &Isometry::translation(clear.x, clear.y, clear.z)
        )
        .unwrap()
        .is_empty());
    println!(
        "CLOTH_CONTACT_REPORT {}",
        serde_json::json!({
            "scope":"Read-only geometric contacts, not grasp admission or a runtime cloth solver",
            "coordinates":"runtime metres: X east, Y up, Z south",
            "surface_revision":surface.revision(),"physical_thickness_m":0.001,
            "sphere_radius_m":0.012,"touching_sphere_centre":hand.to_array(),
            "clear_sphere_centre":clear.to_array(),"clear_contacts":0,
            "selected_triangle":point.triangle(),"material_barycentric":point.barycentric(),
            "resolved_uv":surface.resolve(&point).unwrap().uv.to_array(),
            "contacts":contacts.iter().map(|c| serde_json::json!({
                "triangle":c.triangle(),"barycentric":c.barycentric(),"revision":c.revision(),
                "hand_position":c.hand_position().to_array(),"midsurface_position":c.midsurface_position().to_array(),
                "normal":c.normal().to_array(),"separation_m":c.separation(),
            })).collect::<Vec<_>>()
        })
    );
}
