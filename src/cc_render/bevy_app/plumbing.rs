//! Collision-aware water driven by scene valves; avatar geometry is read-only.
use super::{
    water_collision::WaterSurfaces,
    water_dynamics::WaterSimulation,
    water_skin::{observe_skin, SkinCache, WaterActivity},
};
use crate::cc_render::{
    scene_package::ScenePackage,
    scene_water::{WaterBridge, WaterFixture, WaterSetting, WaterSnapshot},
};
use bevy::{
    pbr::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
    transform::TransformSystem,
};

#[derive(Resource, Default)]
struct Plumbing(Vec<Outlet>);
struct Outlet {
    fixture: WaterFixture,
    setting: WaterSetting,
    total_liters: f64,
    simulation: WaterSimulation,
}
#[derive(Component)]
struct Drop {
    fixture: usize,
    index: usize,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Plumbing>()
        .init_resource::<WaterSurfaces>()
        .init_resource::<SkinCache>()
        .init_resource::<WaterActivity>()
        .add_systems(Startup, spawn)
        .add_systems(
            PostUpdate,
            (observe_skin, advance, display, observe)
                .chain()
                .after(TransformSystem::TransformPropagate)
                .after(bevy::render::mesh::inherit_weights)
                .after(super::tissue::TissueSurfaceUpdate),
        );
}

fn spawn(
    mut commands: Commands,
    package: Res<ScenePackage>,
    mut plumbing: ResMut<Plumbing>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Sphere::new(1.0).mesh().uv(8, 6));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.72, 0.86, 0.91, 0.65),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.12,
        reflectance: 0.25,
        ..default()
    });
    for (fixture, definition) in package.water_fixtures.iter().enumerate() {
        let simulation = WaterSimulation::new(definition.kind);
        for index in 0..simulation.particles.len() {
            commands.spawn((
                Name::new(format!("Water:{}:{index}", definition.id)),
                Drop { fixture, index },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }
        plumbing.0.push(Outlet {
            fixture: definition.clone(),
            setting: WaterSetting::default(),
            total_liters: 0.,
            simulation,
        });
    }
}

fn advance(
    time: Res<Time>,
    bridge: Res<WaterBridge>,
    surfaces: Res<WaterSurfaces>,
    mut plumbing: ResMut<Plumbing>,
    mut activity: ResMut<WaterActivity>,
) {
    let settings = bridge.requests.borrow();
    for outlet in &mut plumbing.0 {
        outlet.setting = settings[&outlet.fixture.id];
        if surfaces.error.is_none() {
            outlet.total_liters +=
                f64::from(outlet.setting.flow * outlet.fixture.max_liters_per_minute)
                    * time.delta_secs_f64()
                    / 60.;
        }
        let Outlet {
            fixture,
            setting,
            simulation,
            ..
        } = outlet;
        simulation.advance(fixture, setting.flow, time.delta_secs_f64(), &surfaces);
    }
    activity.0 = plumbing
        .0
        .iter()
        .any(|o| o.simulation.particles.iter().any(|p| p.active));
}

fn display(
    plumbing: Res<Plumbing>,
    mut drops: Query<(&Drop, &mut Transform, &mut GlobalTransform, &mut Visibility)>,
) {
    for (drop, mut transform, mut global, mut visibility) in &mut drops {
        let parcel = &plumbing.0[drop.fixture].simulation.particles[drop.index];
        *visibility = if parcel.active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !parcel.active {
            continue;
        }
        let speed = parcel.velocity.length() as f32;
        let length = if parcel.runoff() {
            0.003
        } else {
            (speed * 0.006).clamp(0.002, 0.04)
        };
        *transform = Transform::from_translation(parcel.position.as_vec3())
            .with_scale(Vec3::new(0.0015, length, 0.0015));
        if speed > 1e-6 {
            transform.rotation =
                Quat::from_rotation_arc(Vec3::Y, parcel.velocity.as_vec3().normalize());
        }
        // Root water entities only: skin was observed after the ordinary global
        // transform pass. Publish water globals now without a second body pass.
        *global = GlobalTransform::from(*transform);
    }
}

fn observe(plumbing: Res<Plumbing>, bridge: Res<WaterBridge>, surfaces: Res<WaterSurfaces>) {
    bridge.snapshots.send_replace(
        plumbing
            .0
            .iter()
            .map(|o| WaterSnapshot {
                id: o.fixture.id.clone(),
                revision: o.setting.revision,
                flow: o.setting.flow,
                liters_per_minute: o.setting.flow * o.fixture.max_liters_per_minute,
                total_liters: o.total_liters,
                visible_particles: o.simulation.particles.iter().filter(|p| p.active).count(),
                runoff_particles: o.simulation.particles.iter().filter(|p| p.runoff()).count(),
                body_impacts: o.simulation.body_impacts,
                drain_impacts: o.simulation.drain_impacts,
                dropped_emissions: o.simulation.dropped_emissions,
                unresolved_collisions: o.simulation.unresolved_collisions,
                collision_meshes: surfaces.meshes,
                collision_triangles: surfaces.triangles.len(),
                collision_revision: surfaces.revision,
                collision_error: surfaces.error.clone(),
                collision_update_ms: surfaces.update_ms,
                simulation_ms: o.simulation.elapsed_ms,
                degenerate_triangles: surfaces.degenerate_triangles,
            })
            .collect(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::scene_water::WaterControl;
    #[test]
    fn water_ecs_turns_on_adjusts_and_drains_all_outlets_without_pool_growth() {
        let package: ScenePackage =
            serde_json::from_str(include_str!("../../../scenes/toronto_condo/scene.json")).unwrap();
        let (control, bridge) = WaterControl::new(&package.water_fixtures);
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .insert_resource(package)
            .insert_resource(bridge)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<bevy::render::mesh::skinning::SkinnedMeshInverseBindposes>>();
        install(&mut app);
        app.update();
        let count = app.world_mut().query::<&Drop>().iter(app.world()).count();
        assert_eq!(count, 3072);
        assert!(control
            .state()
            .iter()
            .all(|s| s.flow == 0.0 && s.visible_particles == 0));
        for _ in 0..3 {
            for id in ["shower", "bathroom_faucet", "kitchen_faucet"] {
                control.set(id, 1.0).unwrap();
            }
            for _ in 0..90 {
                step(&mut app);
            }
            assert!(control
                .state()
                .iter()
                .all(|s| s.flow == 1.0 && s.visible_particles > 0 && s.total_liters > 0.0));
            control.set("bathroom_faucet", 0.25).unwrap();
            for _ in 0..60 {
                step(&mut app);
            }
            let states = control.state();
            assert_eq!(
                states
                    .iter()
                    .find(|s| s.id == "bathroom_faucet")
                    .unwrap()
                    .liters_per_minute,
                1.5
            );
            assert_eq!(states.iter().find(|s| s.id == "shower").unwrap().flow, 1.0);
            for id in ["shower", "bathroom_faucet", "kitchen_faucet"] {
                control.set(id, 0.0).unwrap();
            }
            let liters: Vec<_> = states.iter().map(|s| s.total_liters).collect();
            for _ in 0..90 {
                step(&mut app);
            }
            for (index, state) in control.state().iter().enumerate() {
                assert_eq!(state.visible_particles, 0);
                assert_eq!(state.total_liters, liters[index]);
            }
        }
        assert_eq!(
            app.world_mut().query::<&Drop>().iter(app.world()).count(),
            count
        );
    }
    fn step(app: &mut App) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(1.0 / 60.0));
        app.update();
        let mut query = app.world_mut().query::<(&Drop, &Transform)>();
        let plumbing = app.world().resource::<Plumbing>();
        for (drop, transform) in query
            .iter(app.world())
            .filter(|(drop, _)| plumbing.0[drop.fixture].simulation.particles[drop.index].active)
        {
            let fixture = &plumbing.0[drop.fixture].fixture;
            assert!(transform.translation.is_finite());
            assert!(transform.translation.y >= fixture.surface_y);
            assert!(transform.translation.y <= fixture.outlet[1] + 0.001);
        }
    }
}
