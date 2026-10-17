use super::*;
use crate::cc_render::scene_cloth::ClothAsset;
use bevy_rapier3d::rapier::parry::{math::Isometry, shape::SharedShape};

fn layered_app() -> App {
    layered_app_at([0.002, 0.025])
}

fn layered_app_at(heights: [f32; 2]) -> App {
    let mut app = app();
    let mut package = app.world_mut().resource_mut::<ScenePackage>();
    let template = package.cloths[0].clone();
    package.cloths = heights
        .into_iter()
        .enumerate()
        .map(|(i, y)| {
            let mut cloth = template.clone();
            cloth.id = format!("layer-{i}");
            cloth.collision_solids.clear();
            cloth.collision_fixed_parts.clear();
            cloth.parameters.thickness_m = 0.002;
            cloth.loaded = Some(std::sync::Arc::new(ClothAsset {
                schema_version: 1,
                rest_positions: vec![[-0.5, y, -0.5], [0.5, y, -0.5], [0., y, 0.5]],
                initial_positions: vec![[-0.5, y, -0.5], [0.5, y, -0.5], [0., y, 0.5]],
                triangles: vec![[0, 2, 1]],
                material_uv: vec![[0., 0.], [1., 0.], [0.5, 1.]],
            }));
            cloth
        })
        .collect();
    app.update();
    let world = app.world_mut();
    for mut cloth in world.query::<&mut LiveCloth>().iter_mut(world) {
        cloth.colliders = vec![FixedClothCollider::new(
            SharedShape::cuboid(2., 0.1, 2.),
            Isometry::translation(0., -0.1, 0.),
            0.4,
        )
        .unwrap()];
    }
    pose_clock(&mut app, 1000);
    app.update();
    app
}

#[test]
fn live_layers_collide_and_publish_synchronized_meshes() {
    let mut app = layered_app();
    let mut previous = app.world().resource::<ClothObservations>().0.clone();
    for tick in 1..=120 {
        pose_clock(&mut app, 1000 + (tick * 1000 + 59) / 60);
        app.update();
        let observations = &app.world().resource::<ClothObservations>().0;
        assert_eq!(observations.len(), 2);
        for (snapshot, before) in observations.iter().zip(&previous) {
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            assert_eq!(snapshot.simulated_ticks, tick);
            super::assert_publication(before, snapshot);
            // Coupled layers rest together.
            assert_eq!(snapshot.status, observations[0].status);
        }
        assert!(observations[1].bounds_min[1] - observations[0].bounds_max[1] >= 0.001999);
        previous = observations.clone();
    }
    assert!(previous.iter().all(|s| s.status == "resting"));
    let before = app.world().resource::<ClothObservations>().0.clone();
    assert!((before[0].bounds_min[1] - 0.001).abs() < 1e-6);
    assert!((before[1].bounds_min[1] - 0.003).abs() < 1e-6);
    app.update();
    for (paused, before) in app
        .world()
        .resource::<ClothObservations>()
        .0
        .iter()
        .zip(before)
    {
        assert_eq!(paused.status, "paused");
        assert_eq!(paused.surface_revision, before.surface_revision);
        assert_eq!(
            paused.surface_position_sha256,
            before.surface_position_sha256
        );
    }
}

#[test]
fn one_missing_mesh_stops_all_layers_before_physics() {
    let mut app = layered_app();
    let handle = app
        .world_mut()
        .query::<&Mesh3d>()
        .iter(app.world())
        .next()
        .unwrap()
        .0
        .clone();
    app.world_mut()
        .resource_mut::<Assets<Mesh>>()
        .remove(handle.id());
    pose_clock(&mut app, 1017);
    app.update();
    for snapshot in &app.world().resource::<ClothObservations>().0 {
        assert_eq!(snapshot.status, "failed");
        assert_eq!(snapshot.simulated_ticks, 0);
        assert_eq!(snapshot.surface_revision, 0);
        assert_eq!(snapshot.displayed_pts_ms, Some(1000));
    }
}

#[test]
fn divergent_layer_clock_rejects_the_group_without_partial_clock_update() {
    let mut app = layered_app();
    let world = app.world_mut();
    world
        .query::<&mut LiveCloth>()
        .iter_mut(world)
        .next()
        .unwrap()
        .clock
        .displayed = Some(1001);
    pose_clock(&mut app, 1017);
    app.update();
    let observations = &app.world().resource::<ClothObservations>().0;
    for snapshot in observations {
        assert_eq!(
            snapshot.error.as_deref(),
            Some("cloth layer clocks diverged; reload required")
        );
        assert_eq!(snapshot.surface_revision, 0);
        assert_eq!(snapshot.simulated_ticks, 0);
    }
    assert!(observations
        .iter()
        .any(|s| s.displayed_pts_ms == Some(1000)));
    assert!(observations
        .iter()
        .any(|s| s.displayed_pts_ms == Some(1001)));
}

#[test]
fn removing_a_layer_requires_reload_instead_of_reusing_the_batch() {
    let mut app = layered_app();
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<LiveCloth>>()
        .iter(app.world())
        .next()
        .unwrap();
    app.world_mut().despawn(entity);
    pose_clock(&mut app, 1017);
    app.update();
    let snapshot = &app.world().resource::<ClothObservations>().0[0];
    assert_eq!(
        snapshot.error.as_deref(),
        Some("cloth layer set changed; reload required")
    );
    assert_eq!(snapshot.surface_revision, 0);
}

#[test]
fn failed_contact_step_keeps_every_live_mesh_and_revision_unchanged() {
    let mut app = layered_app_at([0.02, 0.02]);
    let before = app.world().resource::<ClothObservations>().0.clone();
    pose_clock(&mut app, 1017);
    app.update();
    for (after, before) in app
        .world()
        .resource::<ClothObservations>()
        .0
        .iter()
        .zip(before)
    {
        assert_eq!(after.status, "failed");
        assert_eq!(after.surface_revision, 0);
        assert_eq!(after.simulated_ticks, 0);
        assert_eq!(
            after.surface_position_sha256,
            before.surface_position_sha256
        );
        assert_eq!(after.mesh_position_sha256, before.mesh_position_sha256);
    }
}
