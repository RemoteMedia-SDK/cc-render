//! Reproducible local weather presets. These never author avatar motion.
use bevy::{
    pbr::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
};

pub(crate) use crate::cc_render::scene_weather::WeatherPreset as CondoWeather;

#[derive(Clone, Copy)]
pub(super) struct WeatherLight {
    pub sun: f32,
    pub ambient: f32,
    pub exposure: f32,
    pub environment: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub fog_color: Color,
    pub window_lumens: f32,
}

impl CondoWeather {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::parse(&std::env::var("CC_CONDO_WEATHER").unwrap_or_else(|_| "clear".into()))
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        value.parse()
    }

    pub(super) fn sky(self) -> Vec4 {
        let (cloud, night, wind) = match self {
            Self::Clear => (0.32, 0., 1.),
            Self::Overcast => (0.95, 0., 1.1),
            Self::Rain => (1., 0., 2.3),
            Self::Fog => (1., 0., 0.25),
            Self::Snow => (0.92, 0., 0.6),
            Self::Night => (0.35, 1., 0.7),
        };
        Vec4::new(0., cloud, night, wind)
    }

    pub(super) fn lighting(self) -> WeatherLight {
        let (sun, ambient, exposure, environment, fog_start, fog_end, window_lumens) = match self {
            Self::Clear => (14000., 160., 9.6, 500., 180., 6500., 18000.),
            Self::Overcast => (1800., 750., 10.7, 400., 120., 4200., 10000.),
            Self::Rain => (650., 550., 10.3, 260., 60., 1800., 6000.),
            Self::Fog => (400., 620., 10.3, 220., 15., 650., 8000.),
            Self::Snow => (1500., 800., 10.7, 330., 70., 2000., 10000.),
            Self::Night => (0.5, 16., 7.5, 8., 120., 4500., 0.),
        };
        let fog_color = if self == Self::Night {
            Color::srgb(0.025, 0.035, 0.065)
        } else {
            Color::srgb(0.67, 0.76, 0.82)
        };
        WeatherLight {
            sun,
            ambient,
            exposure,
            environment,
            fog_start,
            fog_end,
            fog_color,
            window_lumens,
        }
    }
}

#[derive(Component)]
struct Precipitation {
    origin: Vec3,
    phase: f32,
    speed: f32,
    snow: bool,
}

#[derive(Resource, Default)]
struct PrecipitationMaterials(Vec<(Handle<StandardMaterial>, bool)>);

pub(super) fn install(app: &mut App) {
    super::weather_transition::install(app);
    app.insert_resource(
        CondoWeather::from_env().expect("weather was validated before renderer startup"),
    )
    .init_resource::<PrecipitationMaterials>()
    .add_systems(Startup, spawn_precipitation)
    .add_systems(Update, (fall_precipitation, fade_precipitation));
}

fn random(index: u32, salt: u32) -> f32 {
    let mut value = index.wrapping_mul(747796405).wrapping_add(salt);
    value = (value ^ (value >> 16)).wrapping_mul(2246822519);
    (value ^ (value >> 13)) as f32 / u32::MAX as f32
}

fn spawn_precipitation(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut registry: ResMut<PrecipitationMaterials>,
) {
    for snow in [false, true] {
        spawn_precipitation_kind(&mut commands, &mut meshes, &mut materials, &mut registry, snow);
    }
}

fn spawn_precipitation_kind(
    commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>,
    registry: &mut PrecipitationMaterials, snow: bool,
) {
    let mesh = if snow {
        meshes.add(Sphere::new(0.035).mesh().ico(1).unwrap())
    } else {
        meshes.add(Cuboid::new(0.012, 0.42, 0.012))
    };
    let material = materials.add(StandardMaterial {
        base_color: if snow {
            Color::srgb(0.88, 0.93, 1.)
        } else {
            Color::srgba(0.65, 0.78, 0.88, 0.28)
        },
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    registry.0.push((material.clone(), snow));
    for index in 0..if snow { 1400 } else { 2200 } {
        let origin = Vec3::new(
            random(index, 731) * 64. - 32.,
            0.,
            random(index, 941) * 72. - 24.,
        );
        // The exclusion extends past the facade and covered balcony, including
        // drift margin: no precipitation can spawn or drift through the room.
        if origin.x.abs() < 8. && (-7.0..10.0).contains(&origin.z) {
            continue;
        }
        let phase = random(index, 1637) * 55.;
        let speed = if snow {
            0.7 + random(index, 429) * 0.6
        } else {
            12. + random(index, 429) * 4.
        };
        commands.spawn((
            Name::new(if snow {
                "Exterior snow"
            } else {
                "Exterior rain"
            }),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Visibility::Hidden,
            Transform::from_xyz(origin.x, 30. - phase, origin.z)
                .with_rotation(Quat::from_rotation_z(0.10)),
            Precipitation {
                origin,
                phase,
                speed,
                snow,
            },
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }
}

fn fall_precipitation(time: Res<Time>, weather: Res<super::weather_transition::AppliedWeather>,
    mut particles: Query<(&Precipitation, &mut Transform, &mut Visibility)>) {
    let elapsed = time.elapsed_secs();
    for (particle, mut transform, mut visibility) in &mut particles {
        let density = if particle.snow { weather.current.snow } else { weather.current.rain };
        *visibility = if density > 0.0 { Visibility::Inherited } else { Visibility::Hidden };
        if density == 0.0 { continue; }
        transform.translation.y = 30. - (particle.phase + elapsed * particle.speed).rem_euclid(55.);
        let drift = if particle.snow { 1.1 } else { 0.15 };
        transform.translation.x =
            particle.origin.x + (weather.current.sky.x * 0.5 + particle.phase).sin() * drift;
    }
}

fn fade_precipitation(weather: Res<super::weather_transition::AppliedWeather>,
    registry: Res<PrecipitationMaterials>, mut materials: ResMut<Assets<StandardMaterial>>) {
    for (handle, snow) in &registry.0 {
        if let Some(material) = materials.get_mut(handle) {
            material.base_color = if *snow {
                Color::srgba(0.88, 0.93, 1.0, weather.current.snow)
            } else {
                Color::srgba(0.65, 0.78, 0.88, 0.28 * weather.current.rain)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precipitation_pools_are_bounded_hidden_and_outside_the_condo() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>().init_resource::<Assets<StandardMaterial>>()
            .init_resource::<PrecipitationMaterials>().add_systems(Startup, spawn_precipitation);
        app.update();
        let world = app.world_mut();
        let mut count = [0, 0];
        for (particle, transform, visibility) in world.query::<(&Precipitation, &Transform, &Visibility)>().iter(world) {
            count[usize::from(particle.snow)] += 1;
            assert_eq!(*visibility, Visibility::Hidden);
            assert!(particle.speed.is_finite() && particle.speed > 0.0);
            for drift in [-1.1, 1.1] {
                assert!((transform.translation.x + drift).abs() > 6.0 || !(-5.0..7.4).contains(&transform.translation.z));
            }
        }
        assert!(count[0] > 1800 && count[0] <= 2200);
        assert!(count[1] > 1100 && count[1] <= 1400);
        assert_eq!(world.resource::<PrecipitationMaterials>().0.len(), 2);
    }

    #[test]
    fn presets_are_explicit_and_precipitation_stays_outside() {
        for name in ["clear", "overcast", "rain", "fog", "snow", "night"] {
            assert!(CondoWeather::parse(name).is_ok());
        }
        assert!(CondoWeather::parse("typo").is_err());
        for index in 0..2200 {
            let x = random(index, 731) * 64. - 32.;
            let z = random(index, 941) * 72. - 24.;
            if x.abs() < 8. && (-7.0..10.0).contains(&z) {
                continue;
            }
            for drift in [-1.1, 1.1] {
                assert!((x + drift).abs() > 6. || !(-5.0..7.4).contains(&z));
            }
        }
    }
}
