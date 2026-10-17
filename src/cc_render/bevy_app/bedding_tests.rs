//! Authored condo bedding through the production live cloth system.
use super::*;
use parry_ccd::{
    math::{Isometry, Point},
    query::distance,
    shape::TriMesh,
};
use std::io::Write;

fn bedding_package() -> ScenePackage {
    ScenePackage::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo/scene-bedding.json"),
    )
    .unwrap()
}

#[test]
fn additive_bedding_is_valid_but_duplicate_replacements_are_rejected() {
    let mut package = bedding_package();
    assert_eq!(package.cloths.len(), 3);
    assert_eq!(
        package
            .cloths
            .iter()
            .filter(|c| c.replaces_nodes.is_empty())
            .count(),
        2
    );
    let replaced = package
        .cloths
        .iter()
        .find(|c| !c.replaces_nodes.is_empty())
        .unwrap()
        .replaces_nodes
        .clone();
    package.cloths[0].replaces_nodes = replaced;
    assert!(crate::cc_render::scene_cloth::validate(&package).is_err());
}

fn observe_layers(app: &mut App, package: &ScenePackage) -> serde_json::Value {
    let world = app.world_mut();
    let mut layers: Vec<_> = world.query::<&LiveCloth>().iter(world).collect();
    layers.sort_by(|a, b| a.id.cmp(&b.id));
    let meshes: Vec<_> = layers
        .iter()
        .map(|cloth| {
            let definition = package.cloths.iter().find(|c| c.id == cloth.id).unwrap();
            let positions = cloth
                .solver
                .surface()
                .positions()
                .iter()
                .map(|p| Point::new(p.x as f64, p.y as f64, p.z as f64))
                .collect();
            TriMesh::new(
                positions,
                definition.loaded.as_ref().unwrap().triangles.clone(),
            )
        })
        .collect();
    let mut gaps = Vec::new();
    for a in 0..layers.len() {
        for b in a + 1..layers.len() {
            let gap = distance(
                &Isometry::identity(),
                &meshes[a],
                &Isometry::identity(),
                &meshes[b],
            )
            .unwrap();
            gaps.push(serde_json::json!({"a":layers[a].id,"b":layers[b].id,"gap_m":gap}));
        }
    }
    serde_json::json!({"gaps":gaps,"layers":layers.iter().map(|c| serde_json::json!({
        "id":c.id,"positions":c.solver.surface().positions().iter().map(|p|p.to_array()).collect::<Vec<_>>()
    })).collect::<Vec<_>>()})
}

#[test]
fn authored_bedding_layers_drape_together_in_the_live_scene() {
    let package = bedding_package();
    let mut app = app();
    app.insert_resource(package.clone());
    let mut trace = std::env::var("CC_BEDDING_TRACE").ok().map(|path| {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .unwrap()
    });
    pose_clock(&mut app, 1000);
    app.update();
    let mut tail_speed = 0.0f64;
    let started = std::time::Instant::now();
    for tick in 0..=600 {
        if tick > 0 {
            pose_clock(&mut app, 1000 + (tick * 1000 + 59) / 60);
            app.update();
        }
        let snapshots = app.world().resource::<ClothObservations>().0.clone();
        assert_eq!(snapshots.len(), 3);
        let mut geometry = observe_layers(&mut app, &package);
        geometry["frame"] = tick.into();
        geometry["observations"] = serde_json::to_value(&snapshots).unwrap();
        if tick == 0 {
            geometry["assets"] = serde_json::json!(package
                .cloths
                .iter()
                .map(|c| {
                    serde_json::json!({"id":c.id,"sha256":c.sha256,"parameters":c.parameters})
                })
                .collect::<Vec<_>>());
        }
        if let Some(file) = trace.as_mut() {
            writeln!(file, "{}", geometry).unwrap();
            file.flush().unwrap();
        }
        for snapshot in &snapshots {
            assert!(
                snapshot.error.is_none(),
                "tick {tick}, {}: {:?}",
                snapshot.id,
                snapshot.error
            );
            assert_eq!(snapshot.simulated_ticks, tick);
            assert_eq!(snapshot.surface_revision, tick);
            assert_eq!(
                snapshot.surface_position_sha256,
                snapshot.mesh_position_sha256
            );
            assert!(
                snapshot.max_particle_penetration_m < 0.001,
                "tick {tick}, penetration {snapshot:?}"
            );
            assert!(
                snapshot.max_edge_stretch_ratio < 1.15,
                "tick {tick}, stretch {snapshot:?}"
            );
            if tick > 570 {
                tail_speed = tail_speed.max(snapshot.max_speed_m_s);
            }
        }
        for gap in geometry["gaps"].as_array().unwrap() {
            let thickness = |key| {
                package
                    .cloths
                    .iter()
                    .find(|c| c.id == gap[key].as_str().unwrap())
                    .unwrap()
                    .parameters
                    .thickness_m as f64
            };
            let target = (thickness("a") + thickness("b")) * 0.5;
            assert!(
                gap["gap_m"].as_f64().unwrap() >= target - 2e-6,
                "tick {tick}, layer gap {gap}"
            );
        }
        if tick % 60 == 0 || tick == 16 {
            println!(
                "BEDDING_TICK {tick} wall_seconds={:.3}: {}",
                started.elapsed().as_secs_f64(),
                serde_json::to_string(&snapshots).unwrap()
            );
        }
    }
    println!("BEDDING_TAIL_SPEED {tail_speed}");
    assert!(tail_speed < 0.05, "bedding did not settle: {tail_speed}");
}
