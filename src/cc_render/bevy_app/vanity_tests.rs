//! Passive vanity rail and loose-content checks; only diagnostic loads.
use super::*;

fn vanity_fixture() -> (App, SceneMechanism) {
    let scene = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-vanity-drawers.json")).unwrap();
    let index = scene.mechanisms.iter().position(|m| m.id == "vanity_drawers").unwrap();
    assert!(!scene.static_solids().any(|s| s.id == "vanity"));
    assert!(scene.cloths[0].collision_fixed_parts.contains(&"vanity_shell".into()));
    fixture_for(scene, index)
}

#[test]
fn vanity_drawers_resist_side_loads_block_open_release_and_close() {
    let (mut app, mechanism) = vanity_fixture();
    let drawers = door_entities(&mut app);
    assert_eq!(drawers.len(), 2);
    let mut trace = Vec::new();
    let mut obstruction = None;
    for phase in 0..7 {
        if phase == 2 {
            obstruction = Some(app.world_mut().spawn((RigidBody::Fixed, Collider::cuboid(0.025, 0.115, 0.55),
                Transform::from_xyz(-5., 0.695, -2.83))).id());
        }
        if phase == 3 { app.world_mut().despawn(obstruction.take().unwrap()); }
        let force = match phase {
            1 => Vec3::new(0., 4., 10.),
            2 | 3 => Vec3::X * 15.,
            5 => -Vec3::X * 15.,
            _ => Vec3::ZERO,
        };
        for tick in 0..120 {
            for entity in &drawers { app.world_mut().entity_mut(*entity).insert(ExternalForce { force, ..default() }); }
            app.update();
            let states: Vec<_> = drawers.iter().map(|e| super::drawers::drawer_state(&app, &mechanism, *e)).collect();
            trace.push(serde_json::json!({"frame":phase*120+tick, "phase":phase, "parts":states,
                "obstruction":super::drawers::obstruction_state(&app, obstruction)}));
        }
        for state in trace.last().unwrap()["parts"].as_array().unwrap() {
            let travel = state["travel_m"].as_f64().unwrap();
            match phase {
                0 | 1 | 5 | 6 => assert!(travel.abs() < 0.003, "not closed: {state}"),
                2 if state["id"] == "vanity_upper_drawer" => assert!((0.17..0.24).contains(&travel), "obstruction missed: {state}"),
                _ => assert!((travel-0.4).abs() < 0.003, "not open: {state}"),
            }
            if phase == 4 || phase == 6 { assert!(state["speed_m_s"].as_f64().unwrap() < 0.01); }
        }
        assert_current_snapshots(&app, &mechanism, &drawers);
    }
    if let Ok(path) = std::env::var("CC_VANITY_TRACE") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&serde_json::to_vec(&serde_json::json!({
            "scope":"Diagnostic loads on passive vanity rails", "mechanism":mechanism.id,
            "placement":mechanism.placement, "frames":trace})).unwrap()).unwrap();
    }
}

#[test]
fn vanity_lower_drawer_contains_and_carries_a_loose_cube() {
    let (mut app, mechanism) = vanity_fixture();
    let drawers = door_entities(&mut app);
    let lower = *drawers.iter().find(|e| app.world().get::<Prop>(**e).unwrap().id == "vanity_lower_drawer").unwrap();
    let cube = app.world_mut().spawn((RigidBody::Dynamic, Collider::cuboid(0.04, 0.04, 0.04),
        ColliderMassProperties::Mass(0.2), Velocity::zero(),
        Transform::from_xyz(-5.5, 0.40, -2.83))).id();
    for _ in 0..120 { app.update(); }
    let resting = app.world().get::<Transform>(cube).unwrap().translation;
    assert!((0.38..0.39).contains(&resting.y), "cube did not rest on tray: {resting:?}");
    for tick in 0..360 {
        let force = if tick < 240 { Vec3::X * 10. } else { Vec3::ZERO };
        app.world_mut().entity_mut(lower).insert(ExternalForce { force, ..default() });
        app.update();
        let p = app.world().get::<Transform>(cube).unwrap().translation;
        assert!(p.y > 0.37 && p.y < 0.46, "cube escaped tray: {p:?}");
    }
    let state = super::drawers::drawer_state(&app, &mechanism, lower);
    assert!((state["travel_m"].as_f64().unwrap()-0.4).abs() < 0.003);
    let end = app.world().get::<Transform>(cube).unwrap().translation;
    assert!(end.x-resting.x > 0.25, "loose cube was not carried: {end:?}");
    assert!((end.z+2.83).abs() < 0.45);
    for tick in 0..360 {
        let force = if tick < 240 { -Vec3::X * 10. } else { Vec3::ZERO };
        app.world_mut().entity_mut(lower).insert(ExternalForce { force, ..default() });
        app.update();
        let p = app.world().get::<Transform>(cube).unwrap().translation;
        assert!((0.37..0.46).contains(&p.y), "cube escaped closing tray: {p:?}");
    }
    let closed = super::drawers::drawer_state(&app, &mechanism, lower);
    assert!(closed["travel_m"].as_f64().unwrap().abs() < 0.003);
    let returned = app.world().get::<Transform>(cube).unwrap().translation;
    assert!((-5.794..-5.346).contains(&returned.x));
    assert!((returned.z+2.83).abs() < 0.45);
    eprintln!("VANITY_LOOSE_CONTENT rest={resting:?} open={end:?} returned={returned:?}");
}
