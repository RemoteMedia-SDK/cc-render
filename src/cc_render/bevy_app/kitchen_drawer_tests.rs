//! Real exported kitchen/island geometry, passive diagnostic loads and loose contents.
use super::*;

fn kitchen_fixture() -> (App, Vec<SceneMechanism>) {
    let scene = ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scenes/toronto_condo/scene-kitchen-drawers.json")).unwrap();
    let indices: Vec<_> = (0..scene.mechanisms.len()).collect();
    fixture_for_indices(scene, &indices)
}

fn bank_entities(app: &mut App, mechanism: &SceneMechanism) -> Vec<Entity> {
    door_entities(app).into_iter().filter(|e| {
        let id = &app.world().get::<Prop>(*e).unwrap().id;
        mechanism.parts.iter().any(|p| &p.id == id)
    }).collect()
}

#[test]
fn retained_kitchen_geometry_and_vanity_accessories_keep_source_coordinates() {
    let (_, mechanisms) = kitchen_fixture();
    for (bank, part, shape, expected) in [
        ("kitchen_drawers", "kitchen_drawer_shell", "Kitchen detail base back", [-5.935, 0.52, 0.175]),
        ("kitchen_drawers", "kitchen_drawer_shell", "Kitchen detail cutout worktop", [-5.55, 0.93, 0.175]),
        ("island_drawers", "island_drawer_shell", "Kitchen detail island stone top", [-3.55, 0.94, 0.]),
        ("vanity_drawers", "vanity_wall_towel", "vanity_accessory_0", [-2.891, 1.17, -2.80]),
    ] {
        let m = mechanisms.iter().find(|m| m.id == bank).unwrap();
        let p = m.parts.iter().find(|p| p.id == part).unwrap();
        let b = p.colliders.iter().find(|b| b.id == shape).unwrap();
        let world = Vec3::from(m.placement)+Vec3::from(p.position)+Vec3::from(b.position);
        assert!((world-Vec3::from(expected)).length() < 0.00001, "misplaced retained geometry {shape}: {world:?}");
    }
}

fn assert_bank_phase(phase: usize, states: &[serde_json::Value], blocked: &str) {
    for state in states {
        let travel = state["travel_m"].as_f64().unwrap();
        match phase {
            0 | 1 | 5 | 6 => assert!(travel.abs() < 0.003, "not closed: {state}"),
            2 if state["id"] == blocked => assert!((0.18..0.24).contains(&travel), "obstruction missed: {state}"),
            _ => assert!((travel-0.4).abs() < 0.003, "not open: {state}"),
        }
        if phase == 4 || phase == 6 { assert!(state["speed_m_s"].as_f64().unwrap() < 0.01); }
    }
}

#[test]
fn kitchen_and_island_drawers_obstruct_open_release_and_close() {
    for bank in ["kitchen_drawers", "island_drawers"] {
        let (mut app, mechanisms) = kitchen_fixture();
        let m = mechanisms.iter().find(|m| m.id == bank).unwrap();
        let drawers = bank_entities(&mut app, m);
        assert_eq!(drawers.len(), if bank == "kitchen_drawers" { 9 } else { 6 });
        let blocked = m.parts.iter().find(|p| p.id.ends_with("_0_1")).unwrap();
        let axis = Vec3::from(blocked.joint.as_ref().unwrap().axis);
        let front = blocked.colliders.iter().find(|s| s.id.ends_with("_front")).unwrap();
        let center = Vec3::from(m.placement)+Vec3::from(blocked.position)+Vec3::from(front.position)+axis*0.248;
        let mut obstruction = None;
        let mut trace = Vec::new();
        for phase in 0..7 {
            if phase == 2 {
                obstruction = Some(app.world_mut().spawn((RigidBody::Fixed,
                    Collider::cuboid(0.025, front.half_extents[1]+0.01, front.half_extents[2]+0.01),
                    Transform::from_translation(center))).id());
            }
            if phase == 3 { app.world_mut().despawn(obstruction.take().unwrap()); }
            let force = match phase { 1 => Vec3::new(0., 4., 10.), 2 | 3 => axis*25., 5 => -axis*25., _ => Vec3::ZERO };
            for tick in 0..120 {
                for e in &drawers { app.world_mut().entity_mut(*e).insert(ExternalForce { force, ..default() }); }
                app.update();
                let states: Vec<_> = drawers.iter().map(|e| super::drawers::drawer_state(&app, m, *e)).collect();
                if tick == 119 { assert_bank_phase(phase, &states, &blocked.id); }
                trace.push(serde_json::json!({"frame":phase*120+tick, "phase":phase, "parts":states,
                    "obstruction":super::drawers::obstruction_state(&app, obstruction)}));
            }
            assert_current_snapshots(&app, m, &drawers);
        }
        if let Ok(dir) = std::env::var("CC_KITCHEN_TRACE_DIR") {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
                .open(std::path::Path::new(&dir).join(format!("{bank}.json"))).unwrap();
            file.write_all(&serde_json::to_vec(&serde_json::json!({"scope":"Diagnostic loads on passive rails",
                "mechanism":m.id, "placement":m.placement, "frames":trace})).unwrap()).unwrap();
        }
    }
}

struct Contents {
    drawer: Entity,
    body: Entity,
    bounds_center: Vec3,
    floor: Vec3,
    half: Vec3,
    axis: Vec3,
}

fn load_drawers(app: &mut App, mechanisms: &[SceneMechanism]) -> Vec<Contents> {
    let mut contents = Vec::new();
    for m in mechanisms.iter().filter(|m| m.id == "kitchen_drawers" || m.id == "island_drawers") {
        for e in bank_entities(app, m) {
            let id = &app.world().get::<Prop>(e).unwrap().id;
            let part = m.parts.iter().find(|p| &p.id == id).unwrap();
            let floor = part.colliders.iter().find(|s| s.id.ends_with("_bin_0_floor")).unwrap();
            for (index, mass) in [0.2, 0.6, 1.2].into_iter().enumerate() {
                let shape = match index { 0 => Collider::ball(0.025), 1 => Collider::cylinder(0.025, 0.025),
                    _ => Collider::cuboid(0.035, 0.025, 0.025) };
                let at = Vec3::from(m.placement)+Vec3::from(part.position)+Vec3::from(floor.position)
                    +Vec3::new((index as f32-1.)*0.12, floor.half_extents[1]+0.03, 0.);
                let body = app.world_mut().spawn((RigidBody::Dynamic, shape, ColliderMassProperties::Mass(mass),
                    Velocity::zero(), Transform::from_translation(at))).id();
                contents.push(Contents { drawer:e, body, bounds_center:part.bounds().0,
                    floor:Vec3::from(floor.position), half:Vec3::from(floor.half_extents),
                    axis:Vec3::from(part.joint.as_ref().unwrap().axis) });
            }
        }
    }
    contents
}

fn assert_contents(app: &App, contents: &[Contents], settled: bool) {
    for c in contents {
        let drawer = app.world().get::<Transform>(c.drawer).unwrap();
        let body = app.world().get::<Transform>(c.body).unwrap();
        let local = drawer.rotation.inverse()*(body.translation-drawer.translation)+c.bounds_center-c.floor;
        assert!(local.x.abs() < c.half.x && local.z.abs() < c.half.z,
            "content center escaped its bin: {local:?}");
        assert!(local.y > c.half.y+0.018 && local.y < 0.12, "content lost tray support: {local:?}");
        if settled {
            let speed = app.world().get::<Velocity>(c.body).unwrap().linvel.length();
            assert!(speed < 0.05, "contents did not settle: speed={speed} drawer={} position={local:?}",
                app.world().get::<Prop>(c.drawer).unwrap().id);
        }
    }
}

fn wait_contents_settled(app: &mut App, contents: &[Contents]) -> usize {
    // Round loose contents can still roll after the rail stops. Measure a
    // bounded quiet interval, without changing their velocity or damping.
    let mut stable = 0;
    for tick in 0..480 {
        app.update();
        assert_contents(app, contents, false);
        let slow = contents.iter().all(|c|
            app.world().get::<Velocity>(c.body).unwrap().linvel.length() < 0.05);
        stable = if slow { stable+1 } else { 0 };
        if stable == 30 { return tick+1; }
    }
    assert_contents(app, contents, true);
    panic!("contents never sustained 30 quiet frames within the 10-second total settling window");
}

fn drive_loaded_phase(app: &mut App, contents: &[Contents], phase: usize) {
    for _ in 0..120 {
        for c in contents.iter().step_by(3) {
            let force = match phase { 0 => c.axis*25., 2 => -c.axis*25., _ => Vec3::ZERO };
            app.world_mut().entity_mut(c.drawer).insert(ExternalForce { force, ..default() });
        }
        app.update();
        assert_contents(app, contents, false);
    }
}

fn assert_loaded_drawers(app: &mut App, mechanisms: &[SceneMechanism], phase: usize, cycle: usize) {
    for m in mechanisms.iter().filter(|m| m.id == "kitchen_drawers" || m.id == "island_drawers") {
        let drawers = bank_entities(app, m);
        for e in &drawers {
            let state = super::drawers::drawer_state(app, m, *e);
            let expected = if phase < 2 { 0.4 } else { 0. };
            assert!((state["travel_m"].as_f64().unwrap()-expected).abs() < 0.003,
                "cycle {cycle} phase {phase}: {state}");
        }
        assert_current_snapshots(app, m, &drawers);
    }
}

#[test]
fn kitchen_and_island_loaded_drawers_survive_ten_simultaneous_cycles() {
    let (mut app, mechanisms) = kitchen_fixture();
    let contents = load_drawers(&mut app, &mechanisms);
    assert_eq!(contents.len(), 45);
    for _ in 0..120 { app.update(); }
    assert_contents(&app, &contents, true);
    let mut extra_steps = 0;
    let mut longest_settle = 0;
    for cycle in 0..10 {
        for phase in 0..4 {
            drive_loaded_phase(&mut app, &contents, phase);
            if phase == 1 || phase == 3 {
                eprintln!("KITCHEN_SETTLE_CHECK cycle={cycle} phase={phase}");
                let steps = wait_contents_settled(&mut app, &contents);
                extra_steps += steps;
                longest_settle = longest_settle.max(120+steps);
                assert_contents(&app, &contents, true);
            }
            assert_loaded_drawers(&mut app, &mechanisms, phase, cycle);
        }
    }
    eprintln!("KITCHEN_DURABILITY_PASS drawers=15 contents=45 load_per_drawer_kg=2 cycles=10 physics_steps={} longest_settle_seconds={}",
        4920+extra_steps, longest_settle as f32/60.);
}
