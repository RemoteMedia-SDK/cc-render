//! Decorative environment simulation. Never queries or writes avatar transforms.
use crate::cc_render::scene_package::ScenePackage;
use bevy::{
    math::Affine2,
    pbr::{CascadeShadowConfigBuilder, ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderRef},
};
use bevy_rapier3d::prelude::*;

#[derive(Component)]
struct TrafficCar {
    lane: usize,
    fraction: f32,
}

pub(super) struct TrafficRoute {
    segments: Vec<(Vec3, Vec3, f32)>,
    pub(super) length: f32,
}

impl TrafficRoute {
    pub(super) fn new(lane: &crate::cc_render::scene_package::TrafficLane) -> Self {
        let points = if lane.path.is_empty() {
            vec![lane.start, lane.end]
        } else {
            lane.path.clone()
        };
        let segments: Vec<_> = points
            .windows(2)
            .map(|pair| {
                let start = Vec3::from(pair[0]);
                let direction = Vec3::from(pair[1]) - start;
                (start, direction.normalize(), direction.length())
            })
            .collect();
        let length = segments.iter().map(|s| s.2).sum();
        Self { segments, length }
    }

    pub(super) fn transform(&self, fraction: f32) -> Transform {
        let mut distance = fraction.rem_euclid(1.0) * self.length;
        for (start, direction, length) in &self.segments {
            if distance <= *length {
                return Transform::from_translation(*start + *direction * distance)
                    .with_rotation(Quat::from_rotation_y(direction.x.atan2(direction.z)));
            }
            distance -= length;
        }
        // Only reachable through accumulated floating-point roundoff at the endpoint.
        Transform::from_translation(self.segments[0].0)
    }
}

#[derive(Resource)]
struct TrafficRoutes(Vec<TrafficRoute>);

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct WaterExtension {
    #[uniform(100)]
    conditions: Vec4,
}

impl MaterialExtension for WaterExtension {
    fn fragment_shader() -> ShaderRef {
        "scene://water.wgsl".into()
    }
}

type LakeWater = ExtendedMaterial<StandardMaterial, WaterExtension>;

pub(crate) fn install(app: &mut App, package: ScenePackage) {
    super::mechanisms::install(app);
    super::cloth::install(app);
    super::weather::install(app);
    super::sky::install(app);
    super::city_windows::install(app);
    super::texture_filtering::install(app);
    super::practical_lights::install(app);
    super::plumbing::install(app);
    super::wet_surfaces::install(app);
    app.add_plugins(MaterialPlugin::<LakeWater>::default())
        .insert_resource(package)
        .add_systems(
            Startup,
            (spawn_solids, spawn_lighting, spawn_traffic, spawn_lake, super::street_lights::spawn),
        )
        .add_systems(
            Update,
            (
                move_traffic,
                super::texture_filtering::prepare_avatar_hair,
                super::vehicle_lights::prepare_vehicle_lamps,
                super::texture_filtering::allow_light_through_glazing,
                super::lightmaps::attach_lightmaps,
                update_water,
            ),
        );
}

pub(super) fn spawn_solids(mut commands: Commands, package: Res<ScenePackage>) {
    for solid in package.static_solids() {
        commands.spawn((
            Name::new(format!("collision:{}", solid.id)),
            RigidBody::Fixed,
            solid.collider(),
            Friction::coefficient(0.7),
            Transform::from_translation(solid.position.into())
                .with_rotation(Quat::from_rotation_y(solid.yaw_deg.to_radians())),
        ));
    }
}

fn spawn_lighting(mut commands: Commands, weather: Res<super::weather::CondoWeather>) {
    let lighting = weather.lighting();
    spawn_window_skylight(&mut commands, lighting.window_lumens);
    commands.insert_resource(bevy::pbr::DirectionalLightShadowMap { size: 4096 });
    commands.insert_resource(AmbientLight {
        color: Color::srgb(0.92, 0.94, 1.0),
        brightness: lighting.ambient,
    });
    commands.spawn((
        Name::new("Condo afternoon sun"),
        super::weather_transition::WeatherSun,
        DirectionalLight {
            illuminance: lighting.sun,
            color: Color::srgb(1.0, 0.86, 0.70),
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(-30.0, 10.0, 40.0).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            maximum_distance: 1600.0,
            first_cascade_far_bound: 12.0,
            ..default()
        }
        .build(),
    ));
    for (i, position) in [[-3.8, 2.82, 0.5], [3.0, 2.82, 1.8], [0.0, 2.82, -3.5]]
        .iter()
        .enumerate()
    {
        commands.spawn((
            Name::new(format!("Condo warm practical {i}")),
            SpotLight {
                intensity: 16000.0,
                range: 8.0,
                radius: 0.12,
                inner_angle: 0.55,
                outer_angle: 1.15,
                color: Color::srgb(1.0, 0.83, 0.64),
                shadows_enabled: true,
                ..default()
            },
            Transform::from_translation((*position).into()).looking_to(Vec3::NEG_Y, Vec3::Z),
        ));
    }
    commands.spawn((
        Name::new("Reading lamp warm light"),
        PointLight {
            intensity: 3000.0,
            range: 4.0,
            radius: 0.2,
            color: Color::srgb(1.0, 0.77, 0.48),
            ..default()
        },
        Transform::from_xyz(5.35, 1.30, 2.65),
    ));
    // The forward renderer has no diffuse room bounce. A local, unshadowed
    // fill approximates light returned by the ceiling without brightening
    // the entire outdoor city through a stronger global ambient term.
    commands.spawn((
        Name::new("Condo ceiling bounce approximation"),
        PointLight {
            intensity: 6500.0,
            range: 6.0,
            radius: 1.0,
            color: Color::srgb(1.0, 0.88, 0.72),
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(0.0, 2.4, -1.5),
    ));
}

fn spawn_window_skylight(commands: &mut Commands, lumens: f32) {
    // Approximate the broad sky illumination entering below the balcony
    // overhang. These local fills replace missing diffuse GI, not the sun.
    for x in [-4.0, 0.0, 4.0] {
        commands.spawn((
            Name::new("Window skylight approximation"),
            super::weather_transition::WindowSkylight,
            SpotLight {
                intensity: lumens,
                range: 13.0,
                radius: 0.1,
                inner_angle: 1.1,
                outer_angle: 1.3,
                color: Color::srgb(0.80, 0.88, 1.0),
                shadows_enabled: false,
                ..default()
            },
            Transform::from_xyz(x, 2.2, 4.8).looking_to(Vec3::new(0.0, -0.25, -1.0), Vec3::Y),
        ));
    }
}

fn spawn_traffic(mut commands: Commands, package: Res<ScenePackage>, assets: Res<AssetServer>, weather: Res<super::weather::CondoWeather>) {
    let vehicles: Vec<Handle<Scene>> = (0..package.vehicle_variants)
        .map(|variant| assets.load(format!("scene://vehicle.glb#Scene{variant}")))
        .collect();
    let routes: Vec<_> = package.traffic.iter().map(TrafficRoute::new).collect();
    for (index, lane) in package.traffic.iter().enumerate() {
        for car in 0..lane.count {
            let fraction = (car as f32 + 0.25) / lane.count as f32;
            let mut entity = commands.spawn((
                Name::new(format!("Traffic {index}:{car}")),
                SceneRoot(vehicles[(index * 31 + car * 17) % vehicles.len()].clone()),
                TrafficCar {
                    lane: index,
                    fraction,
                },
                routes[index].transform(fraction),
            ));
            entity.with_children(|parent| super::vehicle_lights::spawn_headlight(
                parent, *weather == super::weather::CondoWeather::Night));
        }
    }
    commands.insert_resource(TrafficRoutes(routes));
}

fn move_traffic(
    time: Res<Time>,
    package: Res<ScenePackage>,
    routes: Res<TrafficRoutes>,
    mut cars: Query<(&mut Transform, &mut TrafficCar)>,
) {
    for (mut transform, mut car) in &mut cars {
        let lane = &package.traffic[car.lane];
        let route = &routes.0[car.lane];
        car.fraction =
            (car.fraction + time.delta_secs() * lane.speed_mps / route.length).rem_euclid(1.0);
        *transform = route.transform(car.fraction);
    }
}

fn water_normal(path: &std::path::Path) -> Image {
    use bevy::{
        asset::RenderAssetUsages,
        image::ImageAddressMode,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
    let pixels = image::open(path)
        .expect("scene water normal texture")
        .into_rgba8();
    let mut image = Image::new(
        Extent3d {
            width: pixels.width(),
            height: pixels.height(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels.as_raw().clone(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    super::texture_filtering::add_mips(&mut image);
    let sampler = image.sampler.get_or_init_descriptor();
    sampler.address_mode_u = ImageAddressMode::Repeat;
    sampler.address_mode_v = ImageAddressMode::Repeat;
    image
}

fn spawn_lake(
    mut commands: Commands,
    package: Res<ScenePackage>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LakeWater>>,
    weather: Res<super::weather::CondoWeather>,
) {
    let normal = images.add(water_normal(
        &package.geometry.with_file_name("water_normal.png"),
    ));
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::srgb(0.06, 0.25, 0.31),
            metallic: 0.0,
            perceptual_roughness: 0.26,
            // Dielectric water: F0 ~= 0.02 in Bevy's 0.16 * reflectance^2 convention.
            reflectance: 0.3535534,
            normal_map_texture: Some(normal),
            uv_transform: Affine2::from_scale(Vec2::splat(64.0)),
            ..default()
        },
        extension: WaterExtension {
            conditions: weather.sky(),
        },
    });
    let mut plane = Plane3d::default()
        .mesh()
        .size(package.lake_size[0], package.lake_size[1])
        .build();
    plane
        .generate_tangents()
        .expect("water plane has normals and UVs");
    commands.spawn((
        Name::new("Lake Ontario animated water"),
        Mesh3d(meshes.add(plane)),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(package.lake_center.into()),
    ));
}

fn update_water(weather: Res<super::weather_transition::AppliedWeather>, mut materials: ResMut<Assets<LakeWater>>) {
    for (_, material) in materials.iter_mut() {
        material.extension.conditions = weather.current.sky;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_uses_distance_along_curves_and_wraps() {
        let lane = crate::cc_render::scene_package::TrafficLane {
            start: [0., 0., 0.],
            end: [3., 0., 4.],
            path: vec![[0., 0., 0.], [3., 0., 0.], [3., 0., 4.]],
            speed_mps: 1.,
            count: 1,
        };
        let route = TrafficRoute::new(&lane);
        assert_eq!(route.length, 7.);
        let halfway = route.transform(0.5);
        assert!(halfway
            .translation
            .abs_diff_eq(Vec3::new(3., 0., 0.5), 1e-5));
        assert!((halfway.rotation * Vec3::Z).abs_diff_eq(Vec3::Z, 1e-5));
        assert!(route
            .transform(1.5)
            .translation
            .abs_diff_eq(halfway.translation, 1e-5));
    }

    #[test]
    fn water_texture_contains_complete_mip_chain() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("normal.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([128, 128, 255, 255]))
            .save(&path)
            .unwrap();
        let texture = water_normal(&path);
        assert_eq!(texture.texture_descriptor.mip_level_count, 4);
        assert_eq!(texture.data.len(), (64 + 16 + 4 + 1) * 4);
    }
}
