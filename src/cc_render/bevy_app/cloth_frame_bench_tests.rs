//! Release cost of the live cloth systems per rendered frame: catch-up
//! batch ticks, mesh upload, normals, bounds, snapshots and hand-contact
//! observation. Run with `--ignored --nocapture`:
//! CC_CLOTH_ECS_SCENE (default scene-vanity-drawers.json), CC_CLOTH_ECS_FRAMES
//! (600 at 30 fps), CC_CLOTH_ECS_LOAD (a bench state to resume from).
use super::*;
use bevy::math::DVec3;

#[test]
#[ignore = "benchmark: run in release with --nocapture"]
fn bench_live_cloth_frames() {
    let scene =
        std::env::var("CC_CLOTH_ECS_SCENE").unwrap_or_else(|_| "scene-vanity-drawers.json".into());
    let frames: u64 = std::env::var("CC_CLOTH_ECS_FRAMES")
        .unwrap_or_else(|_| "600".into())
        .parse()
        .unwrap();
    let package = ScenePackage::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo")
            .join(&scene),
    )
    .unwrap();
    let mut app = App::new();
    app.insert_resource(package)
        .insert_resource(Assets::<Mesh>::default())
        .insert_resource(Assets::<StandardMaterial>::default())
        .init_resource::<ClothReadiness>()
        .init_resource::<ClothObservations>()
        .init_resource::<simulation::ClothSimulation>()
        .init_resource::<OfficeController>()
        .add_systems(Startup, spawn)
        .add_systems(Update, (advance, super::contact::observe).chain());
    app.world_mut().spawn(RapierConfiguration::new(1.0));
    app.update();
    if let Ok(path) = std::env::var("CC_CLOTH_ECS_LOAD") {
        let state: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let world = app.world_mut();
        let mut layers: Vec<_> = world.query::<&mut LiveCloth>().iter_mut(world).collect();
        layers.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(layers.len(), state.len());
        for (layer, saved) in layers.iter_mut().zip(state) {
            let read = |key: &str| -> Vec<DVec3> {
                serde_json::from_value::<Vec<[f64; 3]>>(saved[key].clone())
                    .unwrap()
                    .into_iter()
                    .map(DVec3::from_array)
                    .collect()
            };
            layer
                .solver
                .restore_state(read("positions"), read("velocities"));
        }
    }
    pose_clock(&mut app, 100000);
    app.update();
    let mut window = Vec::new();
    for frame in 1..=frames {
        pose_clock(&mut app, 100000 + frame * 1000 / 30);
        let started = std::time::Instant::now();
        app.update();
        window.push(started.elapsed().as_secs_f64() * 1e3);
        if frame % 30 == 0 || frame == frames {
            let observations = &app.world().resource::<ClothObservations>().0;
            assert!(observations.iter().all(|o| o.error.is_none()));
            let mean = window.iter().sum::<f64>() / window.len() as f64;
            let max = window.iter().copied().fold(0., f64::max);
            println!(
                "CLOTH_FRAME {}",
                serde_json::json!({
                    "scene": scene, "frame": frame, "seconds": frame as f64 / 30.,
                    "mean_ms_per_frame": mean, "max_ms_per_frame": max,
                    "status": observations.iter().map(|o| o.status.clone()).collect::<Vec<_>>(),
                    "max_speed": observations.iter().map(|o| o.max_speed_m_s).fold(0., f64::max),
                })
            );
            window.clear();
        }
    }
}
