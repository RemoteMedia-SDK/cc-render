//! Filter textures used by the scene-package renderer at grazing angles.
use bevy::{
    image::ImageFilterMode,
    pbr::NotShadowCaster,
    prelude::*,
    render::render_resource::{TextureDimension, TextureFormat},
};

pub(super) fn install(app: &mut App) {
    // AssetEvents publishes Added in Last. Update sees it one frame later,
    // after render extraction may have prepared a material with an unfiltered
    // image view. Replacing that GPU image does not rebuild existing material
    // bind groups. Filter in the publishing frame, before the first extraction.
    app.add_systems(
        Last,
        prepare_scene_textures.after(bevy::asset::AssetEvents),
    );
}

/// Use multisample coverage for overlapping avatar hair cards. Create a new
/// material rather than mutating an already prepared glTF material in place.
pub(super) fn prepare_avatar_hair(
    assets: Res<AssetServer>,
    names: Res<super::assets::MaterialNames>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: Query<
        &mut MeshMaterial3d<StandardMaterial>,
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
) {
    for mut handle in &mut meshes {
        let Some(path) = assets.get_path(handle.0.id()) else {
            continue;
        };
        if path.source().as_str().is_some() {
            continue;
        }
        let Some(name) = path
            .label()
            .and_then(|label| label.strip_prefix("Material"))
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| names.0.get(index))
        else {
            continue;
        };
        if !name.contains("Hair") && !name.starts_with("Scalp_") {
            continue;
        }
        let Some(original) = materials.get(&handle.0) else {
            continue;
        };
        if original.alpha_mode != AlphaMode::Blend {
            continue;
        }
        let mut coverage = original.clone();
        coverage.alpha_mode = AlphaMode::AlphaToCoverage;
        handle.0 = materials.add(coverage);
        tracing::info!(target: "cc_render", "avatar hair multisample coverage: {}", name);
    }
}

/// Bevy's shadow prepass treats blended alpha above 0.05 as opaque. Our clear
/// architectural panes use alpha 0.13, so opt them out of opaque shadow maps.
/// The mullions, frames and walls continue to cast shadows normally.
pub(super) fn allow_light_through_glazing(
    mut commands: Commands,
    assets: Res<AssetServer>,
    materials: Res<Assets<StandardMaterial>>,
    meshes: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
) {
    for (entity, handle) in &meshes {
        let Some(path) = assets.get_path(handle.0.id()) else {
            continue;
        };
        if path.source().as_str() != Some("scene") {
            continue;
        }
        let Some(material) = materials.get(&handle.0) else {
            continue;
        };
        if material.alpha_mode == AlphaMode::Blend && material.base_color.alpha() <= 0.2 {
            commands.entity(entity).insert(NotShadowCaster);
            tracing::info!(target: "cc_render", "clear scene glazing transmits shadow light: {}", path);
        }
    }
}

pub(super) fn prepare_scene_textures(
    mut events: EventReader<AssetEvent<Image>>,
    assets: Res<AssetServer>,
    avatar: Res<super::camera::GlbAsset>,
    mut images: ResMut<Assets<Image>>,
) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        let Some(path) = assets.get_path(*id) else {
            continue;
        };
        let avatar_texture = path.source().as_str().is_none()
            && path.path().file_name() == std::path::Path::new(&avatar.filename).file_name()
            && path
                .label()
                .is_some_and(|label| label.starts_with("Texture"));
        if path.source().as_str() != Some("scene") && !avatar_texture {
            continue;
        }
        if let Some(image) = images.get_mut(*id) {
            add_mips(image);
            tracing::info!(target: "cc_render", "package texture: {} format={:?} mips={}",
                path, image.texture_descriptor.format, image.texture_descriptor.mip_level_count);
        }
    }
}

pub(super) fn add_mips(image: &mut Image) {
    let desc = &image.texture_descriptor;
    if desc.dimension != TextureDimension::D2
        || desc.size.depth_or_array_layers != 1
        || desc.mip_level_count != 1
        || !matches!(
            desc.format,
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
        )
    {
        return;
    }
    let Some(mut pixels) =
        image::RgbaImage::from_raw(desc.size.width, desc.size.height, image.data.clone())
    else {
        return;
    };
    while pixels.width() > 1 || pixels.height() > 1 {
        pixels = image::imageops::resize(
            &pixels,
            (pixels.width() / 2).max(1),
            (pixels.height() / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
        image.data.extend_from_slice(pixels.as_raw());
        image.texture_descriptor.mip_level_count += 1;
    }
    let sampler = image.sampler.get_or_init_descriptor();
    sampler.mag_filter = ImageFilterMode::Linear;
    sampler.min_filter = ImageFilterMode::Linear;
    sampler.mipmap_filter = ImageFilterMode::Linear;
    sampler.anisotropy_clamp = 16;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{asset::RenderAssetUsages, render::render_resource::Extent3d};

    #[test]
    fn newly_loaded_scene_texture_is_filtered_before_first_render_extraction() {
        use bevy::{
            asset::{io::AssetSource, AssetPlugin},
            render::texture::ImagePlugin,
        };
        let directory = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(8, 4, image::Rgba([127, 127, 127, 255]))
            .save(directory.path().join("facade.png"))
            .unwrap();
        let mut app = App::new();
        app.register_asset_source(
            "scene",
            AssetSource::build().with_reader(AssetSource::get_default_reader(
                directory.path().to_string_lossy().into_owned(),
            )),
        );
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), ImagePlugin::default()))
            .insert_resource(super::super::camera::GlbAsset {
                filename: "unused-avatar.glb".into(),
                width: 8,
                height: 4,
                scene_filename: None,
            });
        install(&mut app);
        app.finish();
        app.cleanup();
        let handle: Handle<Image> = app.world().resource::<AssetServer>().load("scene://facade.png");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.update();
            // Render extraction follows the main schedules. The first frame
            // exposing the texture must already contain its complete mip chain.
            if let Some(texture) = app.world().resource::<Assets<Image>>().get(&handle) {
                assert_eq!(texture.texture_descriptor.mip_level_count, 4);
                assert!(matches!(&texture.sampler,
                    bevy::image::ImageSampler::Descriptor(sampler) if sampler.anisotropy_clamp == 16));
                break;
            }
            assert!(std::time::Instant::now() < deadline, "image loading timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn filters_rectangular_texture_once_and_preserves_top_level() {
        let mut texture = Image::new(
            Extent3d {
                width: 8,
                height: 4,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![127; 8 * 4 * 4],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        add_mips(&mut texture);
        assert_eq!(texture.texture_descriptor.mip_level_count, 4);
        assert_eq!(texture.data.len(), (8 * 4 + 4 * 2 + 2 + 1) * 4);
        assert_eq!(&texture.data[..8 * 4 * 4], &[127; 8 * 4 * 4]);
        let bytes = texture.data.clone();
        add_mips(&mut texture);
        assert_eq!(texture.data, bytes);
    }
}
