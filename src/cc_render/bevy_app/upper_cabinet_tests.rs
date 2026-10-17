//! Exported upper cabinets under diagnostic loads, with loose shelf contents.
use super::*;
use super::kitchen_drawers as loaded;

fn assert_leaves(app: &App, mechanism: &SceneMechanism, doors: &[Entity], target: f64) {
    for entity in doors {
        let state = super::cabinets::observed_door(app, mechanism, *entity);
        assert!((state["angle_degrees"].as_f64().unwrap()-target).abs() < 1., "{state}");
    }
    assert_current_snapshots(app, mechanism, doors);
}

fn apply_torque(app: &mut App, doors: &[Entity], load: f32) {
    for e in doors {
        app.world_mut().entity_mut(*e).insert(ExternalForce { torque:Vec3::Y*load, ..default() });
    }
}

fn assert_recorded_phase(phase: usize, states: &[serde_json::Value]) {
    for state in states {
        let angle = state["angle_degrees"].as_f64().unwrap();
        if phase == 1 && state["id"] == "upper_cabinet_door_0" {
            assert!((10. ..70.).contains(&angle), "obstruction missed: {state}");
        } else {
            let expected = if phase == 0 || phase >= 4 { 0. } else { 90. };
            assert!((angle-expected).abs() < 1., "phase {phase}: {state}");
        }
        if phase == 3 || phase == 5 {
            assert!(state["angular_speed_rad_s"].as_f64().unwrap() < 0.01);
        }
    }
}

#[test]
fn upper_cabinet_hinges_obstruct_open_release_and_close() {
    let (mut app, mechanisms) = loaded::furnished_fixture();
    let m = mechanisms.iter().find(|m| m.id == "upper_cabinets").unwrap();
    let doors = loaded::bank_entities(&mut app, m);
    assert_eq!(doors.len(), 3);
    let mut trace = Vec::new();
    let mut blocker = None;
    for phase in 0..6 {
        if phase == 1 {
            blocker = Some(app.world_mut().spawn((RigidBody::Fixed, Collider::cuboid(0.06, 0.2, 0.06),
                Transform::from_xyz(-5.20, 2.12, 0.35))).id());
        }
        if phase == 2 { app.world_mut().despawn(blocker.take().unwrap()); }
        let load = match phase { 1 | 2 => 1.5, 4 => -1.5, _ => 0. };
        for tick in 0..120 {
            apply_torque(&mut app, &doors, load);
            app.update();
            let states: Vec<_> = doors.iter().map(|e| super::cabinets::observed_door(&app, m, *e)).collect();
            trace.push(serde_json::json!({"frame":phase*120+tick, "phase":phase, "parts":states,
                "obstruction":super::drawers::obstruction_state(&app, blocker)}));
        }
        assert_recorded_phase(phase, trace.last().unwrap()["parts"].as_array().unwrap());
        assert_current_snapshots(&app, m, &doors);
    }
    if let Ok(path) = std::env::var("CC_UPPER_TRACE") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&serde_json::to_vec(&serde_json::json!({"scope":"Diagnostic torques on passive upper hinges",
            "mechanism":m.id, "placement":m.placement, "frames":trace})).unwrap()).unwrap();
    }
}

struct ShelfContent {
    entity: Entity,
    shelf: Vec3,
    half: Vec3,
}

fn load_shelves(app: &mut App, mechanism: &SceneMechanism) -> Vec<ShelfContent> {
    let shell = &mechanism.parts[0];
    let mut contents = Vec::new();
    for shelf in shell.colliders.iter().filter(|s| s.id.ends_with("_shelf_0") || s.id.ends_with("_shelf_1")) {
        let center = Vec3::from(mechanism.placement)+Vec3::from(shelf.position);
        for index in 0..2 {
            let half = Vec3::new(0.035, 0.04, 0.035);
            let shape = if index == 0 { Collider::cuboid(half.x, half.y, half.z) }
                        else { Collider::cylinder(half.y, half.x) };
            let at = center+Vec3::new(-0.04, 0.06, (index as f32-0.5)*0.20);
            let entity = app.world_mut().spawn((RigidBody::Dynamic, shape,
                ColliderMassProperties::Mass(if index == 0 { 0.5 } else { 1. }),
                Velocity::zero(), Transform::from_translation(at))).id();
            contents.push(ShelfContent { entity, shelf:center, half });
        }
    }
    contents
}

fn assert_shelf_contents(app: &App, contents: &[ShelfContent]) {
    for c in contents {
        let pose = app.world().get::<Transform>(c.entity).unwrap();
        // All corners of the box enclosing each shape must stay on its shelf.
        for corner in 0..8 {
            let sign = Vec3::new(if corner&1==0 { -1. } else { 1. },
                if corner&2==0 { -1. } else { 1. }, if corner&4==0 { -1. } else { 1. });
            let p = pose.transform_point(sign*c.half)-c.shelf;
            assert!(p.x.abs() < 0.19 && p.z.abs() < 0.25 && (0.008.. 0.32).contains(&p.y),
                "shelf contents penetrated or escaped: {p:?}");
        }
    }
}

fn mixed_phase(app: &mut App, doors: &[Entity], shelves: &[ShelfContent], drawers: &[loaded::Contents], phase: usize) {
    let torque = match phase { 0 => 1.5, 2 => -1.5, _ => 0. };
    for _ in 0..120 {
        loaded::apply_loaded_force(app, drawers, phase);
        apply_torque(app, doors, torque);
        app.update();
        assert_shelf_contents(app, shelves);
        loaded::assert_contents(app, drawers, false);
    }
}

#[test]
fn loaded_shelves_and_drawers_survive_ten_simultaneous_hinge_and_rail_cycles() {
    let (mut app, mechanisms) = loaded::furnished_fixture();
    let m = mechanisms.iter().find(|m| m.id == "upper_cabinets").unwrap();
    let doors = loaded::bank_entities(&mut app, m);
    let shelf_contents = load_shelves(&mut app, m);
    let drawer_contents = loaded::load_drawers(&mut app, &mechanisms);
    assert_eq!(shelf_contents.len(), 12);
    assert_eq!(drawer_contents.len(), 48);
    for _ in 0..120 { app.update(); }
    assert_shelf_contents(&app, &shelf_contents);
    // Small physical jostle, not a pose/velocity correction or object follower.
    for c in &shelf_contents {
        app.world_mut().entity_mut(c.entity).insert(ExternalImpulse { impulse:Vec3::Z*0.008, ..default() });
    }
    for cycle in 0..10 {
        for phase in 0..4 {
            mixed_phase(&mut app, &doors, &shelf_contents, &drawer_contents, phase);
            assert_leaves(&app, m, &doors, if phase < 2 { 90. } else { 0. });
            loaded::assert_loaded_drawers(&mut app, &mechanisms, phase, cycle);
        }
    }
    for c in &shelf_contents {
        assert!(app.world().get::<Velocity>(c.entity).unwrap().linvel.length() < 0.01);
    }
    eprintln!("SHELF_DURABILITY_PASS hinges=3 drawers=16 shelf_contents=12 drawer_contents=48 cycles=10 physics_steps=4920");
}
