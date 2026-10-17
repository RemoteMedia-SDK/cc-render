//! Actual authored drawer shapes under diagnostic loads; not avatar handling.
use super::*;

fn package() -> ScenePackage {
    ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-drawers.json")).unwrap()
}

fn drawer_state(app: &App, mechanism: &SceneMechanism, entity: Entity) -> serde_json::Value {
    let prop = app.world().get::<Prop>(entity).unwrap();
    let part = mechanism.parts.iter().find(|p| p.id == prop.id).unwrap();
    let pose = app.world().get::<Transform>(entity).unwrap();
    let origin = pose.translation - pose.rotation * part.bounds().0;
    let rest = Vec3::from(mechanism.placement) + Vec3::from(part.position);
    let delta = origin - rest;
    assert!(delta.x.abs() < 0.003 && delta.y.abs() < 0.003, "rail transverse drift {delta:?}");
    assert!(pose.rotation.angle_between(Quat::IDENTITY) < 0.005, "rail rotation");
    assert!((-0.003..=0.403).contains(&delta.z), "rail travel {}", delta.z);
    serde_json::json!({"id":prop.id,"position":origin.to_array(),
        "rotation_xyzw":pose.rotation.to_array(),"travel_m":delta.z,
        "speed_m_s":app.world().get::<Velocity>(entity).unwrap().linvel.length()})
}

fn obstruction_state(app: &App, entity: Option<Entity>) -> serde_json::Value {
    let Some(entity) = entity else { return serde_json::Value::Null; };
    let pose = app.world().get::<Transform>(entity).unwrap();
    let shape = app.world().get::<Collider>(entity).unwrap().raw.as_cuboid().unwrap();
    serde_json::json!({"position":pose.translation.to_array(),
        "half_extents":[shape.half_extents.x,shape.half_extents.y,shape.half_extents.z]})
}

fn assert_phase(phase: usize, states: &serde_json::Value) {
    for state in states.as_array().unwrap() {
        let travel = state["travel_m"].as_f64().unwrap();
        match phase {
            0 | 1 | 5 | 6 => assert!(travel.abs() < 0.003, "not closed: {state}"),
            2 if state["id"] == "bedside_upper_drawer" => assert!((0.12..0.26).contains(&travel), "obstruction missed: {state}"),
            _ => assert!((travel-0.4).abs() < 0.003, "not open: {state}"),
        }
        if phase == 4 || phase == 6 {
            assert!(state["speed_m_s"].as_f64().unwrap() < 0.01, "not settled: {state}");
        }
    }
}

#[test]
fn bedside_drawers_have_open_cavities_and_cloth_uses_only_fixed_shell() {
    let package = package();
    let mechanism = package.mechanisms[1].clone();
    assert_eq!(mechanism.replaces_nodes.len(), 7);
    assert!(!package.static_solids().any(|s| s.id == "bedside_unit"));
    let (mut app, _) = fixture_for(package.clone(), 1);
    let world = app.world_mut();
    let cavity = Vec3::from(mechanism.placement) + Vec3::new(0., 0.48, 0.);
    for (collider, pose) in world.query_filtered::<(&Collider, &Transform), With<MechanismBody>>().iter(world) {
        assert!(!collider.raw.contains_point(&bevy_rapier3d::rapier::na::Isometry3::identity(),
            &bevy_rapier3d::rapier::na::Point3::from((cavity-pose.translation).to_array())), "filled drawer cavity");
    }
    let cloth = &package.cloths[0];
    assert!(!cloth.collision_solids.iter().any(|id| id == "bedside_unit"));
    assert_eq!(cloth.collision_fixed_parts, ["bedside_shell"]);
    assert_eq!(cloth.colliders(&package).unwrap().len(), cloth.collision_solids.len()+7);
    for invalid in ["bedside_upper_drawer", "missing", "fridge_main_door"] {
        let mut bad = package.clone();
        bad.cloths[0].collision_fixed_parts = vec![invalid.into()];
        assert!(crate::cc_render::scene_cloth::validate(&bad).is_err());
        assert!(bad.cloths[0].colliders(&bad).is_err());
    }
    let mut duplicate = package.clone();
    duplicate.cloths[0].collision_fixed_parts.push("bedside_shell".into());
    assert!(crate::cc_render::scene_cloth::validate(&duplicate).is_err());
}

#[test]
fn passive_drawers_resist_side_loads_obstruction_and_overtravel_then_release_and_close() {
    let (mut app, mechanism) = fixture_for(package(), 1);
    let drawers = door_entities(&mut app);
    assert_eq!(drawers.len(), 2);
    let mut trace = Vec::new();
    let mut obstruction = None;
    for phase in 0..7 {
        if phase == 2 {
            obstruction = Some(app.world_mut().spawn((RigidBody::Fixed, Collider::cuboid(0.35, 0.115, 0.025),
                Transform::from_xyz(5.52, 0.48, -3.60))).id());
        }
        if phase == 3 { app.world_mut().despawn(obstruction.take().unwrap()); }
        let force = match phase {
            1 => Vec3::new(10., 4., 0.),
            2 | 3 => Vec3::Z * 12.,
            5 => -Vec3::Z * 12.,
            _ => Vec3::ZERO,
        };
        for tick in 0..120 {
            for entity in &drawers { app.world_mut().entity_mut(*entity).insert(ExternalForce { force, ..default() }); }
            app.update();
            let states: Vec<_> = drawers.iter().map(|e| drawer_state(&app, &mechanism, *e)).collect();
            trace.push(serde_json::json!({"frame":phase*120+tick,"phase":phase,"parts":states,
                "obstruction":obstruction_state(&app, obstruction)}));
        }
        assert_current_snapshots(&app, &mechanism, &drawers);
        assert_phase(phase, &trace.last().unwrap()["parts"]);
    }
    if let Ok(path) = std::env::var("CC_DRAWER_TRACE") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&serde_json::to_vec(&serde_json::json!({
            "scope":"Diagnostic Rapier loads on authored drawers; no avatar motion or runtime actuator",
            "mechanism":"bedside_drawers","placement":mechanism.placement,"frames":trace})).unwrap()).unwrap();
    }
}
