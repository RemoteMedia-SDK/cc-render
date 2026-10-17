use super::*;
use crate::cc_render::office::timeline::StreamIdentity;

fn package() -> ScenePackage {
    ScenePackage::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo/scene-cloth.json"),
    )
    .unwrap()
}

#[test]
fn cloth_clock_uses_content_time_not_arrival_time_and_rejects_discontinuity() {
    let mut clock = ClothClock::default();
    assert_eq!(clock.observe("session", 100000).unwrap(), 0);
    assert_eq!(clock.observe("session", 100016).unwrap(), 0);
    assert_eq!(clock.observe("session", 100017).unwrap(), 1);
    clock.ticks = 1;
    assert_eq!(clock.observe("session", 100017).unwrap(), 1);
    assert!(clock.observe("session", 100000).is_err());
    assert!(clock.observe("other", 100017).is_err());
    assert!(clock.observe("session", 103000).is_err());
    assert_eq!(clock.displayed, Some(100017));
    assert_eq!(clock.observe("session", 100100).unwrap(), 6);
}

fn app() -> App {
    let mut app = App::new();
    app.insert_resource(package())
        .insert_resource(Assets::<Mesh>::default())
        .insert_resource(Assets::<StandardMaterial>::default())
        .init_resource::<ClothReadiness>()
        .init_resource::<ClothObservations>()
        .init_resource::<OfficeController>()
        .add_systems(Startup, spawn)
        .add_systems(Update, advance);
    app.world_mut().spawn(RapierConfiguration::new(1.0));
    app
}

fn pose_clock(app: &mut App, pts: u64) {
    let mut office = app.world_mut().resource_mut::<OfficeController>();
    office.observed.native_stream = Some(StreamIdentity {
        session_id: "cloth-clock-test".into(),
        generation_revision: 1,
        skeleton_id: "smpl22".into(),
        root_mode: "world".into(),
    });
    office.observed.displayed_pts_ms = pts;
}

#[test]
fn cloth_ecs_mesh_follows_solver_and_stalls_with_committed_motion_clock() {
    let mut app = app();
    app.update();
    let initial = app.world().resource::<ClothObservations>().0[0].clone();
    assert_eq!(initial.status, "waiting_for_motion");
    pose_clock(&mut app, 100000);
    app.update();
    let mut tail_speed = 0.0f64;
    let mut snapshots = Vec::new();
    for frame in 1..=600 {
        pose_clock(&mut app, 100000 + frame * 1000 / 30);
        app.update();
        let snapshot = app.world().resource::<ClothObservations>().0[0].clone();
        assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
        assert_eq!(
            snapshot.surface_position_sha256,
            snapshot.mesh_position_sha256
        );
        if frame > 585 {
            tail_speed = tail_speed.max(snapshot.max_speed_m_s);
        }
        snapshots.push(snapshot);
    }
    let final_state = snapshots.last().unwrap();
    assert_eq!(final_state.simulated_ticks, 1200);
    assert_eq!(final_state.surface_revision, 1200);
    assert_ne!(
        final_state.surface_position_sha256,
        initial.surface_position_sha256
    );
    for _ in 0..10 {
        app.update();
    }
    let paused = app.world().resource::<ClothObservations>().0[0].clone();
    assert_eq!(paused.status, "paused");
    assert_eq!(paused.surface_revision, final_state.surface_revision);
    assert_eq!(
        paused.surface_position_sha256,
        final_state.surface_position_sha256
    );
    println!(
        "LIVE_CLOTH_ECS_REPORT {}",
        serde_json::json!({"scope":"Headless ECS system test, no avatar interaction", "tail_max_speed_m_s":tail_speed,"final":final_state})
    );
    if let Ok(path) = std::env::var("CC_CLOTH_ECS_TRACE") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.write_all(&serde_json::to_vec(&snapshots).unwrap())
            .unwrap();
    }
    assert!(tail_speed < 0.05, "not settled: {tail_speed}");
    assert!(
        snapshots.iter().all(|s| s.max_edge_stretch_ratio < 1.1),
        "excessive stretch"
    );
    assert!(
        snapshots
            .iter()
            .all(|s| s.max_particle_penetration_m < 0.001),
        "particle penetration"
    );
    let mut office = app.world_mut().resource_mut::<OfficeController>();
    office.observed.displayed_pts_ms += 100;
    office.motion_error = Some("synthetic test rejection".into());
    app.update();
    assert_eq!(
        app.world().resource::<ClothObservations>().0[0].surface_revision,
        1200
    );
}

#[test]
fn cloth_package_replaces_tall_bed_collision_and_hashes_its_surface() {
    let package = package();
    assert!(!package.solids.iter().any(|s| s.id == "bed_base"));
    let mattress = package
        .solids
        .iter()
        .find(|s| s.id == "bed_mattress")
        .unwrap();
    assert!((mattress.position[1] + mattress.half_extents[1] - 0.62).abs() < 1e-5);
    assert!((mattress.border_radius_m - 0.1064).abs() < 1e-5);
    assert_eq!(
        package.cloths[0]
            .loaded
            .as_ref()
            .unwrap()
            .rest_positions
            .len(),
        1221
    );
    let mut cloth = package.cloths[0].clone();
    cloth.sha256 = "0".repeat(64);
    assert!(cloth.load(package.geometry.parent().unwrap()).is_err());
    let mut duplicate = package.clone();
    duplicate.cloths[0].collision_solids.push("bed_base".into());
    assert!(crate::cc_render::scene_cloth::validate(&duplicate).is_err());
    let json = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo/scene-cloth.json"),
    )
    .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["cloths"][0]["hand_target"] = serde_json::json!([0, 0, 0]);
    assert!(serde_json::from_value::<ScenePackage>(value).is_err());
}

#[test]
fn authored_cloth_starts_clear_of_the_complete_fixed_scene() {
    let mut app = app();
    app.update();
    let observation = &app.world().resource::<ClothObservations>().0[0];
    println!(
        "INITIAL_CLOTH_COLLISION {}",
        serde_json::to_string(observation).unwrap()
    );
    assert!(
        observation.max_particle_penetration_m < 0.001,
        "initial cloth overlaps fixed scene by {} m",
        observation.max_particle_penetration_m
    );
}

#[test]
fn removed_render_mesh_is_reported_as_failure() {
    let mut app = app();
    app.update();
    let handle = app
        .world_mut()
        .query::<&Mesh3d>()
        .single(app.world())
        .0
        .clone();
    app.world_mut()
        .resource_mut::<Assets<Mesh>>()
        .remove(handle.id());
    app.update();
    let observation = &app.world().resource::<ClothObservations>().0[0];
    assert_eq!(observation.status, "failed");
    assert_eq!(
        observation.error.as_deref(),
        Some("cloth render mesh missing")
    );
}

#[test]
fn static_bedding_replacement_is_scoped_to_the_environment() {
    let mut app = App::new();
    app.insert_resource(package())
        .init_resource::<ClothReadiness>()
        .add_systems(Update, hide_static_bedding);
    let root = app
        .world_mut()
        .spawn(super::super::camera::EnvironmentSceneRoot)
        .id();
    let replaced = app
        .world_mut()
        .spawn((
            Name::new("Bedroom detail folded duvet"),
            Visibility::Inherited,
        ))
        .set_parent(root)
        .id();
    let unrelated = app
        .world_mut()
        .spawn((
            Name::new("Bedroom detail folded duvet"),
            Visibility::Inherited,
        ))
        .id();
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(replaced).unwrap(),
        Visibility::Hidden
    );
    assert_eq!(
        *app.world().get::<Visibility>(unrelated).unwrap(),
        Visibility::Inherited
    );
}
