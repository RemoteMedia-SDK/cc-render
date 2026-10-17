//! Observed passive sliding glazing under diagnostic loads, not avatar handling.
use super::*;

#[test]
fn balcony_slider_resists_side_loads_blocks_opens_releases_and_closes() {
    let package = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-balcony-slider.json")).unwrap();
    assert!(!package.static_solids().any(|s| s.id == "south_glass_pane_002"));
    let (mut app, mechanism) = fixture_for(package, 4);
    let sliders = door_entities(&mut app);
    assert_eq!(sliders.len(), 1);
    let door = sliders[0];
    let mut trace = Vec::new();
    let mut blocker = None;
    for phase in 0..7 {
        if phase == 2 {
            blocker = Some(app.world_mut().spawn((RigidBody::Fixed, Collider::cuboid(0.1, 0.2, 0.1),
                Transform::from_xyz(0.8, 1.1, 4.9))).id());
        }
        if phase == 3 { app.world_mut().despawn(blocker.take().unwrap()); }
        let force = match phase {
            1 => Vec3::new(0., 20., 30.),
            2 | 3 => Vec3::X * 150.,
            5 => -Vec3::X * 150.,
            _ => Vec3::ZERO,
        };
        for tick in 0..120 {
            app.world_mut().entity_mut(door).insert(ExternalForce { force, ..default() });
            app.update();
            let state = super::drawers::drawer_state(&app, &mechanism, door);
            if tick == 119 { assert_phase(phase, &state); }
            trace.push(serde_json::json!({"frame":phase*120+tick,"phase":phase,"parts":[state],
                "obstruction":super::drawers::obstruction_state(&app, blocker)}));
        }
        assert_current_snapshots(&app, &mechanism, &sliders);
    }
    if let Ok(path) = std::env::var("CC_BALCONY_SLIDER_TRACE") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&serde_json::to_vec(&serde_json::json!({
            "scope":"Diagnostic loads on balcony slider; no avatar or runtime actuator",
            "mechanism":mechanism.id,"placement":mechanism.placement,"frames":trace})).unwrap()).unwrap();
    }
}

fn assert_phase(phase: usize, state: &serde_json::Value) {
    let travel = state["travel_m"].as_f64().unwrap();
    match phase {
        0 | 1 | 5 | 6 => assert!(travel.abs() < 0.003, "not closed: {state}"),
        2 => assert!((0.6..0.8).contains(&travel), "obstruction missed: {state}"),
        _ => assert!((travel-2.).abs() < 0.003, "not open: {state}"),
    }
    if phase == 4 || phase == 6 {
        assert!(state["speed_m_s"].as_f64().unwrap() < 0.01, "not settled: {state}");
    }
}
