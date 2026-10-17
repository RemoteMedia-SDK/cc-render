//! Scenic road fixtures derived from existing traffic centrelines, not surveyed poles.
use super::{environment::TrafficRoute, weather::CondoWeather};
use crate::cc_render::scene_package::ScenePackage;
use bevy::prelude::*;

struct FixtureAssets {
    pole: Handle<Mesh>,
    arm: Handle<Mesh>,
    lens: Handle<Mesh>,
    metal: Handle<StandardMaterial>,
    diffuser: Handle<StandardMaterial>,
}

impl FixtureAssets {
    fn new(
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        night: bool,
    ) -> Self {
        Self {
            pole: meshes.add(Cylinder::new(0.075, 8.4).mesh().resolution(8)),
            arm: meshes.add(Cuboid::new(1.4, 0.10, 0.12)),
            lens: meshes.add(Cuboid::new(0.55, 0.06, 0.25)),
            metal: materials.add(StandardMaterial {
                base_color: Color::srgb(0.24, 0.26, 0.27),
                metallic: 0.65,
                perceptual_roughness: 0.48,
                ..default()
            }),
            diffuser: materials.add(StandardMaterial {
                base_color: Color::srgb(0.75, 0.74, 0.65),
                emissive: if night {
                    LinearRgba::rgb(1000.0, 780.0, 470.0)
                } else {
                    LinearRgba::BLACK
                },
                emissive_exposure_weight: 1.0,
                ..default()
            }),
        }
    }

    fn children(&self, parent: &mut ChildBuilder, night: bool) {
        for (name, mesh, material, position) in [
            (
                "Streetlight pole",
                &self.pole,
                &self.metal,
                Vec3::new(5.6, 4.2, 0.0),
            ),
            (
                "Streetlight arm",
                &self.arm,
                &self.metal,
                Vec3::new(4.95, 8.35, 0.0),
            ),
            (
                "Streetlight diffuser",
                &self.lens,
                &self.diffuser,
                Vec3::new(4.35, 8.28, 0.0),
            ),
        ] {
            parent.spawn((
                Name::new(name),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(position),
            ));
        }
        parent.spawn((
                Name::new("Streetlight road pool"),
                super::weather_transition::NightSpot(160_000.0),
                if night { Visibility::Inherited } else { Visibility::Hidden },
                SpotLight {
                    intensity: if night { 160_000.0 } else { 0.0 },
                    // The elevated deck is about 10 m above the lower ground.
                    // End these unshadowed beams before that second surface.
                    range: 16.5,
                    radius: 0.15,
                    inner_angle: 0.65,
                    outer_angle: 0.95,
                    color: Color::srgb(1.0, 0.84, 0.63),
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(4.35, 8.23, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
            ));
    }
}

pub(super) fn spawn(
    mut commands: Commands,
    package: Res<ScenePackage>,
    weather: Res<CondoWeather>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut emission: ResMut<super::weather_transition::NightEmission>,
) {
    let night = *weather == CondoWeather::Night;
    let assets = FixtureAssets::new(&mut meshes, &mut materials, night);
    emission.0.push((assets.diffuser.clone(), LinearRgba::rgb(1000.0, 780.0, 470.0)));
    let mut count = 0;
    for lane in &package.traffic {
        let route = TrafficRoute::new(lane);
        for index in 0..(route.length / 40.0) as usize {
            let transform = route.transform((index as f32 * 40.0 + 20.0) / route.length);
            // Concentrate authored detail on the waterfront visible from this condo.
            if transform.translation.xz().length() > 1400.0 {
                continue;
            }
            commands
                .spawn((
                    Name::new("Scenic roadside fixture"),
                    transform,
                    Visibility::default(),
                ))
                .with_children(|parent| assets.children(parent, night));
            count += 1;
        }
    }
    tracing::info!(target: "cc_render", "scenic street fixtures: {count}, night emission: {night}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn street_fixtures_preserve_daylight_and_bound_night_spill() {
        for night in [false, true] {
            let mut app = App::new();
            let package: ScenePackage =
                serde_json::from_str(include_str!("../../../scenes/toronto_condo/scene.json"))
                    .unwrap();
            app.insert_resource(package)
                .insert_resource(if night {
                    CondoWeather::Night
                } else {
                    CondoWeather::Clear
                })
                .init_resource::<Assets<Mesh>>()
                .init_resource::<Assets<StandardMaterial>>()
                .init_resource::<super::super::weather_transition::NightEmission>()
                .add_systems(Startup, spawn);
            app.update();
            let world = app.world_mut();
            let fixtures = world.query::<&Mesh3d>().iter(world).count() / 3;
            assert!(fixtures > 0 && fixtures < 250);
            let lights = world.query::<&SpotLight>().iter(world).filter(|light| light.intensity > 0.0).count();
            assert_eq!(lights, if night { fixtures } else { 0 });
            assert_eq!(world.query::<&SpotLight>().iter(world).count(), fixtures);
            for (light, transform, parent) in world
                .query::<(&SpotLight, &Transform, &Parent)>()
                .iter(world)
            {
                // The highest ray on the cone must still point below the horizon.
                let y = transform.forward().y;
                let highest_ray =
                    y * light.outer_angle.cos() + (1.0 - y * y).sqrt() * light.outer_angle.sin();
                assert!(
                    highest_ray < 0.0,
                    "streetlight cone emits above the horizon"
                );
                let road = world.get::<Transform>(parent.get()).unwrap();
                if road.translation.y > -85.0 {
                    // Lower land is authored at -90.5 m in build.py. An elevated
                    // fixture's entire sphere must stop above that second surface.
                    let bottom = road.translation.y + transform.translation.y - light.range;
                    assert!(bottom > -90.5, "elevated streetlight reaches lower ground");
                }
            }
            let emissive = world
                .resource::<Assets<StandardMaterial>>()
                .iter()
                .filter(|(_, material)| material.emissive.red > 0.0)
                .count();
            assert_eq!(emissive, usize::from(night));
        }
    }
}
