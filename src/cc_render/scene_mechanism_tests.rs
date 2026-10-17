use super::*;
use crate::cc_render::scene_package::ScenePackage;

fn fixture() -> ScenePackage {
    serde_json::from_str(include_str!("../../scenes/toronto_condo/scene-articulated.json")).unwrap()
}

#[test]
fn articulated_package_replaces_navigation_box_and_preserves_authored_frames() {
    let p = ScenePackage::load(&Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-articulated.json")).unwrap();
    let objects = p.office_objects();
    assert!(!objects.iter().any(|o| o.id == "kitchen_fridge"));
    let m = &p.mechanisms[0];
    for part in &m.parts {
        let object = objects.iter().find(|o| o.id == part.id).unwrap();
        let center = part.bounds().0;
        for (shape, actual) in part.colliders.iter().zip(&object.parts) {
            let expected = Vec3::from(m.placement) + Vec3::from(part.position) + Vec3::from(shape.position);
            assert!(Vec3::from(object.anchor(actual.position)).distance(expected) < 1e-6);
            assert_eq!(actual.half_extents, shape.half_extents);
        }
        assert!(Vec3::from(object.position).distance(Vec3::from(m.placement) + Vec3::from(part.position) + center) < 1e-6);
        if let Some(handle) = &part.handle_region {
            assert!(object.grasp_transform().unwrap().translation.distance(
                Vec3::from(m.placement) + Vec3::from(part.position) + Vec3::from(handle.centre)) < 1e-6);
        }
    }
}

#[test]
fn mechanism_schema_rejects_motion_channels_and_invalid_joint_geometry() {
    let mut raw: serde_json::Value = serde_json::from_str(include_str!("../../scenes/toronto_condo/scene-articulated.json")).unwrap();
    for field in ["motor", "root_pos", "target_angle", "hand_trajectory"] {
        raw["mechanisms"][0]["parts"][1]["joint"][field] = serde_json::json!(0);
        assert!(serde_json::from_value::<ScenePackage>(raw.clone()).is_err(), "accepted {field}");
        raw["mechanisms"][0]["parts"][1]["joint"].as_object_mut().unwrap().remove(field);
    }
    for invalid in [0, 1, 2, 3, 4, 5] {
        let mut p = fixture();
        let m = &mut p.mechanisms[0];
        match invalid {
            0 => m.parts[1].joint.as_mut().unwrap().parent = m.parts[1].id.clone(),
            1 => m.parts[1].joint.as_mut().unwrap().axis = [0., 2., 0.],
            2 => m.parts[1].joint.as_mut().unwrap().child_anchor = [1., 0., 0.],
            3 => m.parts[1].mass_kg = Some(-1.),
            4 => m.parts[1].colliders[0].half_extents[0] = 0.,
            _ => m.parts[1].handle_region.as_mut().unwrap().centre = [99.; 3],
        }
        assert!(validate_mechanisms(&p.mechanisms, &p.solids).is_err(), "accepted invalid case {invalid}");
    }
}

#[test]
fn mechanisms_reject_duplicate_ids_replacements_and_asset_mutation() {
    let mut p = fixture();
    p.mechanisms.push(p.mechanisms[0].clone());
    assert!(validate_mechanisms(&p.mechanisms, &p.solids).is_err());
    p = fixture(); p.mechanisms[0].parts[1].id = p.solids[0].id.clone();
    assert!(validate_mechanisms(&p.mechanisms, &p.solids).is_err());
    p = fixture(); p.mechanisms[0].replaces_solid = "missing".into();
    assert!(validate_mechanisms(&p.mechanisms, &p.solids).is_err());
    let temp = tempfile::tempdir().unwrap();
    let mut part = fixture().mechanisms.remove(0).parts.remove(0);
    part.geometry = "asset.glb".into();
    std::fs::write(temp.path().join(&part.geometry), b"mutated geometry").unwrap();
    assert!(part.verify_asset(&temp.path().canonicalize().unwrap()).is_err());
    part.geometry = "../outside.glb".into();
    assert!(part.validate().is_err());
}

#[test]
fn slider_schema_requires_metre_limits_and_rejects_commands_and_tilted_axes() {
    let raw: serde_json::Value = serde_json::from_str(include_str!("../../scenes/toronto_condo/scene-drawers.json")).unwrap();
    for field in ["motor", "target_position", "speed", "hand_trajectory"] {
        let mut value = raw.clone();
        value["mechanisms"][1]["parts"][1]["joint"][field] = serde_json::json!(1);
        assert!(serde_json::from_value::<ScenePackage>(value).is_err(), "accepted {field}");
    }
    for case in 0..7 {
        let mut package: ScenePackage = serde_json::from_value(raw.clone()).unwrap();
        let joint = package.mechanisms[1].parts[1].joint.as_mut().unwrap();
        match case {
            0 => joint.limits_m = None,
            1 => joint.limits_rad = Some([0., 0.4]),
            2 => joint.limits_m = Some([0.1, 0.4]),
            3 => joint.limits_m = Some([0., 11.]),
            4 => joint.limits_m = Some([0., f32::NAN]),
            5 => joint.axis = [0., 1., 0.],
            _ => joint.axis = [0., 0., 2.],
        }
        assert!(validate_mechanisms(&package.mechanisms, &package.solids).is_err(), "accepted case {case}");
    }
    let mut hinge = fixture();
    hinge.mechanisms[0].parts[1].joint.as_mut().unwrap().limits_m = Some([0., 0.4]);
    assert!(validate_mechanisms(&hinge.mechanisms, &hinge.solids).is_err());
}
