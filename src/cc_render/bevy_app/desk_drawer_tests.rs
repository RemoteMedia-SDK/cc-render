//! Passive desk rail through the retained frame; diagnostic forces only.
use super::*;

#[test]
fn desk_drawer_clears_frame_resists_side_loads_obstructs_and_closes() {
    let (mut app, mechanisms) = super::kitchen_drawers::furnished_fixture();
    let mechanism = mechanisms.iter().find(|m| m.id == "desk_drawer").unwrap();
    let entities = super::kitchen_drawers::bank_entities(&mut app, mechanism);
    assert_eq!(entities.len(), 1);
    let entity = entities[0];
    let mut trace = Vec::new();
    let mut obstruction = None;
    // Seven independent physical conditions, two seconds each. No pose writes.
    let phases = [(Vec3::ZERO, 0.), (Vec3::new(0., 4., 10.), 0.),
        (-Vec3::X*15., 0.18), (-Vec3::X*15., 0.35), (Vec3::ZERO, 0.35),
        (Vec3::X*15., 0.), (Vec3::ZERO, 0.)];
    for (phase, (force, target)) in phases.into_iter().enumerate() {
        if phase == 2 {
            obstruction = Some(app.world_mut().spawn((RigidBody::Fixed,
                Collider::cuboid(0.025, 0.13, 0.22), Transform::from_xyz(1.0125, 0.59, -3.86))).id());
        }
        if phase == 3 { app.world_mut().despawn(obstruction.take().unwrap()); }
        for tick in 0..120 {
            app.world_mut().entity_mut(entity).insert(ExternalForce { force, ..default() });
            app.update();
            let state = super::drawers::drawer_state(&app, mechanism, entity);
            trace.push(serde_json::json!({"frame":phase*120+tick, "phase":phase, "parts":[state],
                "obstruction":super::drawers::obstruction_state(&app, obstruction)}));
        }
        let state = &trace.last().unwrap()["parts"][0];
        let travel = state["travel_m"].as_f64().unwrap();
        assert!((travel-target).abs() < 0.003, "phase={phase}: {state}");
        if force == Vec3::ZERO { assert!(state["speed_m_s"].as_f64().unwrap() < 0.01); }
        assert_current_snapshots(&app, mechanism, &entities);
    }
    if let Ok(path) = std::env::var("CC_DESK_TRACE") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&serde_json::to_vec(&serde_json::json!({
            "scope":"Diagnostic loads on passive desk rail", "mechanism":mechanism.id,
            "placement":mechanism.placement, "frames":trace})).unwrap()).unwrap();
    }
}
