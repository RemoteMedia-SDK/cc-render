//! Authored indoor point lights and transmitting lamp shades. No avatar state.
use crate::cc_render::scene_package::{SceneLightDiffuser, ScenePackage, ScenePointLight};
use bevy::{asset::AssetPath, pbr::NotShadowCaster, prelude::*};

pub(super) fn install(app: &mut App) {
    app.add_systems(Startup, spawn)
        .add_systems(Update, prepare_diffusers);
}

fn light_color(light: &ScenePointLight) -> Color {
    Color::srgb(light.color_srgb[0], light.color_srgb[1], light.color_srgb[2])
}

fn spawn(mut commands: Commands, package: Res<ScenePackage>) {
    for light in &package.point_lights {
        commands.spawn((
            Name::new(format!("Condo practical: {}", light.id)),
            PointLight {
                color: light_color(light),
                intensity: light.lumens,
                range: light.range_m,
                radius: light.radius_m,
                shadows_enabled: light.shadows,
                ..default()
            },
            Transform::from_translation(light.position.into()),
        ));
        tracing::info!(target: "cc_render", "authored point light: {} lumens={} position={:?}", light.id, light.lumens, light.position);
    }
}

fn diffuser_for_mesh<'a>(
    package: &'a ScenePackage,
    name: &str,
    path: &AssetPath<'_>,
) -> Option<(&'a ScenePointLight, &'a SceneLightDiffuser)> {
    if path.source().as_str() != Some("scene")
        || path.path().file_name() != package.geometry.file_name()
    {
        return None;
    }
    package.point_lights.iter().find_map(|light| {
        light.diffuser.as_ref().filter(|d| d.mesh == name).map(|d| (light, d))
    })
}

fn prepare_diffusers(
    mut commands: Commands,
    package: Res<ScenePackage>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut added: Query<(Entity, &Name, &mut MeshMaterial3d<StandardMaterial>), Added<MeshMaterial3d<StandardMaterial>>>,
) {
    for (entity, name, mut handle) in &mut added {
        let Some(path) = assets.get_path(handle.0.id()) else { continue };
        let Some((light, diffuser)) = diffuser_for_mesh(&package, name.as_str(), &path) else { continue };
        let Some(original) = materials.get(&handle.0) else { continue };
        // Fabrics are shared with pillows and upholstery: change only this instance.
        let mut glowing = original.clone();
        let color = light_color(light).to_linear();
        glowing.emissive = LinearRgba::rgb(color.red * diffuser.emissive_strength,
            color.green * diffuser.emissive_strength, color.blue * diffuser.emissive_strength);
        glowing.emissive_exposure_weight = 1.0;
        handle.0 = materials.add(glowing);
        // Approximate linen transmission; furniture and walls still cast shadows.
        commands.entity(entity).insert(NotShadowCaster);
        tracing::info!(target: "cc_render", "authored light diffuser: {} mesh={}", light.id, name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ScenePackage {
        let mut package: ScenePackage = serde_json::from_str(include_str!("../../../scenes/toronto_condo/scene.json")).unwrap();
        package.point_lights = vec![serde_json::from_value(serde_json::json!({
            "id":"bedside", "position":[5.52,0.94,-4.18], "color_srgb":[1,0.78,0.52],
            "lumens":600, "range_m":3.5, "radius_m":0.08, "shadows":true,
            "diffuser":{"mesh":"Lamp shade","emissive_strength":50}
        })).unwrap()];
        package
    }

    #[test]
    fn authored_lights_spawn_once_with_configured_photometry() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).insert_resource(fixture()).add_systems(Startup, spawn);
        app.update();
        app.update();
        let mut query = app.world_mut().query::<(&PointLight, &Transform)>();
        let lights: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(lights.len(), 1);
        let (light, transform) = lights[0];
        assert_eq!(light.intensity, 600.0);
        assert_eq!(light.range, 3.5);
        assert_eq!(light.radius, 0.08);
        assert!(light.shadows_enabled);
        assert_eq!(transform.translation, Vec3::new(5.52, 0.94, -4.18));
        assert_eq!(light.color, Color::srgb(1.0, 0.78, 0.52));
    }

    #[test]
    fn diffuser_binding_rejects_avatar_foreign_geometry_and_unnamed_meshes() {
        let package = fixture();
        let scene = AssetPath::parse("scene://interior-city.glb#Material0");
        assert!(diffuser_for_mesh(&package, "Lamp shade", &scene).is_some());
        assert!(diffuser_for_mesh(&package, "Pillow", &scene).is_none());
        for path in ["beautiful_assistant.smpl_aligned.glb#Material0",
            "scene://vehicle.glb#Material0", "other://interior-city.glb#Material0"] {
            assert!(diffuser_for_mesh(&package, "Lamp shade", &AssetPath::parse(path)).is_none());
        }
    }
}
