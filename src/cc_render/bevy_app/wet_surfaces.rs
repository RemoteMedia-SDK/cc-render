//! Decorative water film on authored exterior paving; no pose or physics access.
use crate::cc_render::scene_package::ScenePackage;
use bevy::{
    asset::AssetPath,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderRef},
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct WetExtension {
    #[uniform(100)]
    sky: Vec4,
    /// Wetness, balcony shelter mask selector, baked sky strength, reserved.
    #[uniform(101)]
    coating: Vec4,
}

impl MaterialExtension for WetExtension {
    fn fragment_shader() -> ShaderRef { "scene://wet_surfaces.wgsl".into() }
}

type WetMaterial = ExtendedMaterial<StandardMaterial, WetExtension>;

pub(super) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<WetMaterial>::default())
        .add_systems(Update, (attach, update).chain().after(super::lightmaps::attach_lightmaps));
}

fn exterior_style(package: &ScenePackage, name: &str, path: &AssetPath<'_>) -> Option<f32> {
    if path.source().as_str() != Some("scene") || path.path().file_name() != package.geometry.file_name() {
        return None;
    }
    match name {
        "Condo baked balcony" | "Balcony slab 12 x 2.4 m" => Some(1.0),
        "Toronto streets -80.4" | "Toronto streets -90.15" => Some(0.0),
        _ => None,
    }
}

fn attach(
    mut commands: Commands,
    package: Res<ScenePackage>,
    weather: Res<super::weather_transition::AppliedWeather>,
    assets: Res<AssetServer>,
    originals: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<WetMaterial>>,
    added: Query<(Entity, &Name, &MeshMaterial3d<StandardMaterial>), Added<MeshMaterial3d<StandardMaterial>>>,
) {
    for (entity, name, handle) in &added {
        let Some(path) = assets.get_path(handle.0.id()) else { continue };
        let Some(style) = exterior_style(&package, name.as_str(), &path) else { continue };
        let Some(base) = originals.get(&handle.0) else { continue };
        // Clone per exterior mesh. Stone shared by indoor furniture stays dry.
        let strength = package.lightmaps.iter().find(|m| m.mesh == name.as_str())
            .map_or(0.0, |m| m.sky_strength);
        let handle = materials.add(ExtendedMaterial {
            base: base.clone(),
            extension: WetExtension {
                sky: weather.current.sky,
                coating: Vec4::new(weather.surface_wetness, style, strength, 0.0),
            },
        });
        commands.entity(entity).remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(handle));
        tracing::info!(target: "cc_render", "exterior wet surface: {} sheltered={} baked_sky={}", name, style, strength);
    }
}

fn update(weather: Res<super::weather_transition::AppliedWeather>, mut materials: ResMut<Assets<WetMaterial>>) {
    for (_, material) in materials.iter_mut() {
        material.extension.sky = weather.current.sky;
        material.extension.coating.x = weather.surface_wetness;
        let strength = material.extension.coating.z;
        if strength > 0.0 {
            material.base.lightmap_exposure = strength * weather.current.lighting.environment;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paving_in_the_selected_scene_can_receive_the_coating() {
        let package: ScenePackage = serde_json::from_str(include_str!("../../../scenes/toronto_condo/scene.json")).unwrap();
        let path = AssetPath::parse("scene://interior-city.glb#Material0");
        assert_eq!(exterior_style(&package, "Condo baked balcony", &path), Some(1.0));
        assert_eq!(exterior_style(&package, "Toronto streets -90.15", &path), Some(0.0));
        for name in ["Condo baked floor", "Bathroom floor", "Hair.0", "Open balcony threshold", "Balcony overhang"] {
            assert_eq!(exterior_style(&package, name, &path), None);
        }
        for path in ["beautiful_assistant.glb#Material0", "scene://vehicle.glb#Material0", "other://interior-city.glb#Material0"] {
            assert_eq!(exterior_style(&package, "Condo baked balcony", &AssetPath::parse(path)), None);
        }
    }
}
