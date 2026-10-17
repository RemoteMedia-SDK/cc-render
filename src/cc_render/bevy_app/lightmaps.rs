//! Static sky irradiance on authored surfaces; never baked avatar lighting.
use crate::cc_render::scene_package::ScenePackage;
use bevy::{image::ImageLoaderSettings, pbr::Lightmap, prelude::*};

pub(super) fn attach_lightmaps(
    mut commands: Commands,
    package: Res<ScenePackage>,
    weather: Res<super::weather_transition::AppliedWeather>,
    assets: Res<AssetServer>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    added: Query<
        (Entity, &Name, &Mesh3d, &MeshMaterial3d<StandardMaterial>),
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
) {
    for (entity, name, mesh, material) in &added {
        let Some(map) = package.lightmaps.iter().find(|m| m.mesh == name.as_str()) else {
            continue;
        };
        let Some(path) = assets.get_path(material.0.id()) else {
            continue;
        };
        if path.source().as_str() != Some("scene") {
            continue;
        }
        if !meshes
            .get(&mesh.0)
            .is_some_and(|m| m.contains_attribute(Mesh::ATTRIBUTE_UV_1))
        {
            tracing::error!(target: "cc_render", "lightmap mesh {} has no UV1; scene cannot become ready", name);
            continue;
        }
        let Some(material) = materials.get_mut(&material.0) else {
            continue;
        };
        material.lightmap_exposure = map.sky_strength * weather.current.lighting.environment;
        let image = assets.load_with_settings(
            format!("scene://{}", map.image),
            |settings: &mut ImageLoaderSettings| {
                settings.is_srgb = false;
            },
        );
        commands
            .entity(entity)
            .insert((Lightmap { image, ..default() }, super::weather_transition::BakedSkyStrength(map.sky_strength)));
        tracing::info!(target: "cc_render", "sky lightmap attached: {} gain={}", name, material.lightmap_exposure);
    }
}
