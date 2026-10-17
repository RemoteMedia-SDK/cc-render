//! Actual bedroom doorway geometry under diagnostic torques, not avatar motion.
use super::*;

#[test]
fn bedroom_door_obstructs_opens_releases_closes_and_publishes_current_geometry() {
    let package = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-doors.json")).unwrap();
    assert!(!package.static_solids().any(|s| s.id == "bedroom_door_jamb"));
    assert!(package.cloths[0].collision_fixed_parts.iter().any(|s| s == "bedroom_hinge_jamb"));
    let (mut app, mechanism) = fixture_for(package, 3);
    let doors = door_entities(&mut app);
    assert_eq!(doors.len(), 1);
    let door = doors[0];
    let mut trace = Vec::new();
    let mut blocker = None;
    for phase in 0..6 {
        if phase == 1 {
            blocker = Some(app.world_mut().spawn((RigidBody::Fixed, Collider::cuboid(0.08, 0.3, 0.06),
                Transform::from_xyz(3.05, 1.04, -1.7))).id());
        }
        if phase == 2 { app.world_mut().despawn(blocker.take().unwrap()); }
        let torque = match phase { 1 | 2 => 8., 4 => -8., _ => 0. };
        for tick in 0..180 {
            app.world_mut().entity_mut(door).insert(ExternalForce { torque:Vec3::Y*torque, ..default() });
            app.update();
            let state = super::cabinets::observed_door(&app, &mechanism, door);
            if tick == 179 { assert_room_phase(phase, &state); }
            let obstruction = blocker.map(|e| {
                let pose = app.world().get::<Transform>(e).unwrap();
                let shape = app.world().get::<Collider>(e).unwrap().raw.as_cuboid().unwrap();
                serde_json::json!({"position":pose.translation.to_array(),
                    "half_extents":[shape.half_extents.x,shape.half_extents.y,shape.half_extents.z]})
            });
            trace.push(serde_json::json!({"frame":phase*180+tick,"phase":phase,
                "parts":[state],"obstruction":obstruction}));
        }
        assert_current_snapshots(&app, &mechanism, &doors);
    }
    if let Ok(path) = std::env::var("CC_ROOM_DOOR_TRACE") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&serde_json::to_vec(&serde_json::json!({
            "scope":"Diagnostic torques on bedroom door; no avatar or runtime actuator",
            "mechanism":mechanism.id,"placement":mechanism.placement,"frames":trace})).unwrap()).unwrap();
    }
}

fn assert_room_phase(phase: usize, state: &serde_json::Value) {
    let angle = state["angle_degrees"].as_f64().unwrap();
    match phase {
        0 | 4 | 5 => assert!(angle.abs() < 1., "not closed: {state}"),
        1 => assert!((10. ..70.).contains(&angle), "obstruction missed: {state}"),
        _ => assert!((angle-90.).abs() < 1., "not open: {state}"),
    }
    if phase == 3 || phase == 5 {
        assert!(state["angular_speed_rad_s"].as_f64().unwrap() < 0.01, "not settled: {state}");
    }
}
