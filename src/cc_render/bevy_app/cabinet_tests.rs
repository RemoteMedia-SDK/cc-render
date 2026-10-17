//! Authored entry cabinet under diagnostic loads, separate from avatar manipulation.
use super::*;

fn package() -> ScenePackage {
    ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-cabinet.json")).unwrap()
}

pub(super) fn observed_door(app: &App, mechanism: &SceneMechanism, entity: Entity) -> serde_json::Value {
    let prop = app.world().get::<Prop>(entity).unwrap();
    let part = mechanism.parts.iter().find(|p| p.id == prop.id).unwrap();
    let pose = app.world().get::<Transform>(entity).unwrap();
    let hinge = app.world().get::<ImpulseJoint>(entity).unwrap();
    let parent = app.world().get::<Transform>(hinge.parent).unwrap();
    let error = parent.transform_point(hinge.data.as_ref().local_anchor1())
        .distance(pose.transform_point(hinge.data.as_ref().local_anchor2()));
    assert!(error < 0.005, "hinge anchor drift {error}");
    let (angle, pitch, roll) = pose.rotation.to_euler(EulerRot::YXZ);
    assert!(pitch.abs() < 0.005 && roll.abs() < 0.005, "hinge tilt");
    let limits = part.joint.as_ref().unwrap().limits().unwrap();
    assert!(angle >= limits[0]-0.02 && angle <= limits[1]+0.02, "hinge overtravel {angle}");
    serde_json::json!({"id":prop.id,"position":(pose.translation-pose.rotation*part.bounds().0).to_array(),
        "rotation_xyzw":pose.rotation.to_array(),"angle_degrees":angle.to_degrees(),
        "anchor_error_m":error,"angular_speed_rad_s":app.world().get::<Velocity>(entity).unwrap().angvel.length()})
}

fn assert_phase(phase: usize, states: &[serde_json::Value]) {
    for state in states {
        let angle = state["angle_degrees"].as_f64().unwrap().abs();
        match phase {
            0 | 4 | 5 => assert!(angle < 1., "door not closed: {state}"),
            1 if state["id"] == "entry_left_door" => assert!((10. ..70.).contains(&angle), "obstruction missed: {state}"),
            _ => assert!((angle-90.).abs() < 1., "door not open: {state}"),
        }
        if phase == 3 || phase == 5 {
            assert!(state["angular_speed_rad_s"].as_f64().unwrap() < 0.01, "not settled: {state}");
        }
    }
}

#[test]
fn entry_cabinet_has_empty_shelf_spaces_and_opposite_passive_hinges() {
    let scene = package();
    assert!(!scene.static_solids().any(|s| s.id == "entry_storage"));
    assert!(!scene.cloths[0].collision_solids.iter().any(|s| s == "entry_storage"));
    assert!(scene.cloths[0].collision_fixed_parts.iter().any(|s| s == "entry_shell"));
    scene.cloths[0].colliders(&scene).unwrap();
    let (mut app, mechanism) = fixture_for(scene, 2);
    assert_eq!(mechanism.replaces_nodes.len(), 6);
    assert_eq!(mechanism.parts.len(), 3);
    let world = app.world_mut();
    let cavities = [-0.339, 0.339].map(|x| Vec3::from(mechanism.placement)+Vec3::new(x, 0.85, 0.));
    for (collider, pose) in world.query_filtered::<(&Collider, &Transform), With<MechanismBody>>().iter(world) {
        for cavity in cavities {
            assert!(!collider.raw.contains_point(&bevy_rapier3d::rapier::na::Isometry3::identity(),
                &bevy_rapier3d::rapier::na::Point3::from((cavity-pose.translation).to_array())), "filled shelf cavity");
        }
    }
    let left = mechanism.parts[1].joint.as_ref().unwrap().limits().unwrap();
    let right = mechanism.parts[2].joint.as_ref().unwrap().limits().unwrap();
    assert_eq!(left, [-std::f32::consts::FRAC_PI_2, 0.]);
    assert_eq!(right, [0., std::f32::consts::FRAC_PI_2]);
}

#[test]
fn passive_entry_doors_obstruct_open_release_close_and_publish_bounds() {
    let (mut app, mechanism) = fixture_for(package(), 2);
    let doors = door_entities(&mut app);
    assert_eq!(doors.len(), 2);
    let mut trace = Vec::new();
    let mut blocker = None;
    for phase in 0..6 {
        if phase == 1 {
            blocker = Some(app.world_mut().spawn((RigidBody::Fixed, Collider::cuboid(0.08, 0.3, 0.06),
                Transform::from_xyz(-2.13, 1.13, -4.07))).id());
        }
        if phase == 2 { app.world_mut().despawn(blocker.take().unwrap()); }
        for tick in 0..120 {
            for entity in &doors {
                let sign = if app.world().get::<Prop>(*entity).unwrap().id == "entry_left_door" { -1. } else { 1. };
                let load = match phase { 1 | 2 => 3., 4 => -3., _ => 0. };
                app.world_mut().entity_mut(*entity).insert(ExternalForce { torque: Vec3::Y*sign*load, ..default() });
            }
            app.update();
            let states: Vec<_> = doors.iter().map(|e| observed_door(&app, &mechanism, *e)).collect();
            let obstruction = blocker.map(|e| {
                let pose = app.world().get::<Transform>(e).unwrap();
                let shape = app.world().get::<Collider>(e).unwrap().raw.as_cuboid().unwrap();
                serde_json::json!({"position":pose.translation.to_array(),
                    "half_extents":[shape.half_extents.x,shape.half_extents.y,shape.half_extents.z]})
            });
            if tick == 119 { assert_phase(phase, &states); }
            trace.push(serde_json::json!({"frame":phase*120+tick,"phase":phase,"parts":states,"obstruction":obstruction}));
        }
        assert_current_snapshots(&app, &mechanism, &doors);
    }
    if let Ok(path) = std::env::var("CC_CABINET_TRACE") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&serde_json::to_vec(&serde_json::json!({
            "scope":"Diagnostic torques on actual cabinet shapes; no avatar or runtime actuator",
            "mechanism":mechanism.id,"placement":mechanism.placement,"frames":trace})).unwrap()).unwrap();
    }
}
