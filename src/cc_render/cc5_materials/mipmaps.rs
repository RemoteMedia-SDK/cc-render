//! Mipmaps for CC5 hair and skin textures.
//!
//! Bevy loads PNG / JPEG textures (glTF base colours, the CC5 maps) with a
//! single mip level. Hair cards squeeze a whole strand clump (hundreds of
//! texels across) into a few pixels on screen, so without mips the strands
//! alias into coarse dashes and noise. This adds the full chain to the hair
//! materials' textures once they are loaded: a 2×2 box filter in linear space,
//! colour weighted by alpha (transparent texels, often white in CC5's maps, do
//! not bleed into the strands) and alpha averaged (the dithered coverage of
//! `hair_types.wgsl` then keeps the strand density at every distance).

use std::collections::HashSet;

use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{FilterMode, TextureFormat};

use super::{Cc5Handle, Cc5HairMaterial, Cc5Material, Cc5SkinMaterial};

/// Mip chains for every loaded hair texture (base colour and CC5 maps), once.
pub(super) fn hair_mipmaps(
    mats: Query<&Cc5Material>,
    hair: Res<Assets<Cc5HairMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut done: Local<HashSet<AssetId<Image>>>,
) {
    for m in &mats {
        let Cc5Handle::Hair(h) = &m.cc5 else { continue };
        let Some(mat) = hair.get(h) else { continue };
        let ext = &mat.extension;
        let textures = [(mat.base.base_color_texture.as_ref(), true), (ext.root.as_ref(), false), (ext.id.as_ref(), false),
            (ext.flow.as_ref(), false), (ext.spec_mask.as_ref(), false)];
        for (t, alpha_weighted) in textures {
            let Some(t) = t else { continue };
            if done.contains(&t.id()) || images.get(t).is_none_or(|i| i.data.is_empty()) {
                continue; // done, or not loaded yet
            }
            done.insert(t.id());
            if let Some(img) = images.get_mut(t) {
                if generate_mips(img, alpha_weighted) {
                    debug!("hair mipmaps: {:?} ({} levels)", t.id(), img.texture_descriptor.mip_level_count);
                }
            }
        }
    }
}

/// Mip chains for every loaded skin texture, once: without them a 2K-8K skin
/// map seen a few hundred pixels across aliases into a blotchy, sparkly mottle
/// (CC5 samples its skins with mips).
pub(super) fn skin_mipmaps(
    mats: Query<&Cc5Material>,
    skin: Res<Assets<Cc5SkinMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut done: Local<HashSet<AssetId<Image>>>,
) {
    for m in &mats {
        let Cc5Handle::Skin(h) = &m.cc5 else { continue };
        let Some(mat) = skin.get(h) else { continue };
        let (b, e) = (&mat.base, &mat.extension);
        let textures = [
            b.base_color_texture.as_ref(),
            b.normal_map_texture.as_ref(),
            b.metallic_roughness_texture.as_ref(),
            b.occlusion_texture.as_ref(),
            e.micro_normal.as_ref(),
            e.micro_mask.as_ref(),
            e.sss.as_ref(),
            e.transmission.as_ref(),
            e.mask_a.as_ref(),
            e.mask_b.as_ref(),
            e.mask_c.as_ref(),
            e.mouth_ao.as_ref(),
            e.specular.as_ref(),
        ];
        for t in textures.into_iter().flatten() {
            if done.contains(&t.id()) || images.get(t).is_none_or(|i| i.data.is_empty()) {
                continue; // done, or not loaded yet
            }
            done.insert(t.id());
            if let Some(img) = images.get_mut(t) {
                generate_mips(img, false);
            }
        }
    }
}

/// Mip chains for alpha-cutout materials' textures (glTF `MASK`: the Saluki
/// operators' hair, beard, brow and lash cards, which keep Bevy's
/// StandardMaterial), once. Without them a fur texture drawn a few pixels
/// across samples different texels every frame, and the strands crawl like
/// sand whenever the face moves. The base colour's chain keeps the share of
/// texels that pass the cutoff ([`preserve_coverage`]), so the fur doesn't
/// thin out with distance.
pub fn cutout_mipmaps(
    materials: Res<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut done: Local<HashSet<AssetId<Image>>>,
) {
    for (_, m) in materials.iter() {
        let AlphaMode::Mask(cutoff) = m.alpha_mode else { continue };
        // The texture's alpha is multiplied by the base colour's before the test.
        let texture_cutoff = (cutoff / m.base_color.alpha().max(1e-3)).clamp(0.0, 1.0);
        let textures = [
            (m.base_color_texture.as_ref(), Some(texture_cutoff)),
            (m.normal_map_texture.as_ref(), None),
            (m.metallic_roughness_texture.as_ref(), None),
            (m.occlusion_texture.as_ref(), None),
        ];
        for (t, cut) in textures {
            let Some(t) = t else { continue };
            if done.contains(&t.id()) || images.get(t).is_none_or(|i| i.data.is_empty()) {
                continue; // done, or not loaded yet
            }
            done.insert(t.id());
            let Some(img) = images.get_mut(t) else { continue };
            if generate_mips(img, cut.is_some()) {
                if let Some(c) = cut {
                    preserve_coverage(img, c);
                }
                debug!("cutout mipmaps: {:?} ({} levels)", t.id(), img.texture_descriptor.mip_level_count);
            }
        }
    }
}

/// Scale each mip level's alpha so the share of texels at or above `cutoff`
/// matches the full-size level's (averaged mips of sparse strands fall under
/// the cutoff and vanish). RGBA8 with a mip chain from [`generate_mips`].
pub fn preserve_coverage(img: &mut Image, cutoff: f32) {
    let (w0, h0) = (img.texture_descriptor.size.width as usize, img.texture_descriptor.size.height as usize);
    let levels = img.texture_descriptor.mip_level_count as usize;
    let cut = (cutoff * 255.0).ceil().clamp(1.0, 255.0) as usize;
    let mut offset = 0;
    let mut target = None;
    for k in 0..levels {
        let n = (w0 >> k).max(1) * (h0 >> k).max(1) * 4;
        let Some(level) = img.data.get_mut(offset..offset + n) else { return };
        offset += n;
        let mut hist = [0usize; 256];
        for p in level.chunks_exact(4) {
            hist[p[3] as usize] += 1;
        }
        let total = n / 4;
        let covered = hist[cut..].iter().sum::<usize>();
        let Some(share) = target else {
            if covered == 0 || covered == total {
                return; // nothing drawn, or everything: no change with distance
            }
            target = Some(covered as f64 / total as f64);
            continue;
        };
        // The alpha at or above which the full-size share of texels lies,
        // scaled up to the cutoff (never down: denser mips stay as they are).
        let want = ((share * total as f64).round() as usize).max(1);
        let (mut above, mut t) = (0, 255);
        while t > 0 && above + hist[t] < want {
            above += hist[t];
            t -= 1;
        }
        if t == 0 || t >= cut {
            continue;
        }
        let scale = cut as f32 / t as f32;
        for p in level.chunks_exact_mut(4) {
            p[3] = (p[3] as f32 * scale).round().min(255.0) as u8;
        }
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Append a full mip chain to a single-level RGBA8 image; false if it has one
/// already or another format.
pub fn generate_mips(img: &mut Image, alpha_weighted: bool) -> bool {
    let d = &img.texture_descriptor;
    let srgb = match d.format {
        TextureFormat::Rgba8UnormSrgb => true,
        TextureFormat::Rgba8Unorm => false,
        _ => return false,
    };
    if d.mip_level_count != 1 || d.size.depth_or_array_layers != 1 {
        return false;
    }
    let (mut w, mut h) = (d.size.width as usize, d.size.height as usize);
    if img.data.len() != w * h * 4 {
        return false;
    }
    let lut: Vec<f32> = (0..256).map(|i| if srgb { srgb_to_linear(i as f32 / 255.0) } else { i as f32 / 255.0 }).collect();
    let encode = |c: f32| -> u8 {
        let c = c.clamp(0.0, 1.0);
        ((if srgb { linear_to_srgb(c) } else { c }) * 255.0 + 0.5) as u8
    };
    let mut level: Vec<[f32; 4]> = img
        .data
        .chunks_exact(4)
        .map(|p| [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize], p[3] as f32 / 255.0])
        .collect();
    let mut data = std::mem::take(&mut img.data);
    let mut levels = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![[0.0f32; 4]; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let (mut wsum, mut a, mut cw, mut c) = (0.0f32, 0.0f32, [0.0f32; 3], [0.0f32; 3]);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = level[(2 * y + dy).min(h - 1) * w + (2 * x + dx).min(w - 1)];
                    let wt = if alpha_weighted { p[3] } else { 1.0 };
                    for k in 0..3 {
                        cw[k] += p[k] * wt;
                        c[k] += p[k] * 0.25;
                    }
                    wsum += wt;
                    a += p[3] * 0.25;
                }
                let rgb = if wsum > 1e-4 { cw.map(|v| v / wsum) } else { c };
                next[y * nw + x] = [rgb[0], rgb[1], rgb[2], a];
            }
        }
        data.extend(next.iter().flat_map(|p| [encode(p[0]), encode(p[1]), encode(p[2]), (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]));
        level = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    img.data = data;
    img.texture_descriptor.mip_level_count = levels;
    match &mut img.sampler {
        ImageSampler::Descriptor(s) => s.mipmap_filter = FilterMode::Linear.into(),
        s => {
            let mut desc = bevy::image::ImageSamplerDescriptor::linear();
            desc.address_mode_u = bevy::image::ImageAddressMode::Repeat;
            desc.address_mode_v = bevy::image::ImageAddressMode::Repeat;
            *s = ImageSampler::Descriptor(desc);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension};

    #[test]
    fn builds_chain_and_ignores_transparent_colour() {
        // 2x2: one opaque red texel, three transparent white ones
        let data = vec![255, 0, 0, 255, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0];
        let mut img = Image::new(
            Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        assert!(generate_mips(&mut img, true));
        assert_eq!(img.texture_descriptor.mip_level_count, 2);
        assert_eq!(&img.data[16..20], &[255, 0, 0, 64]); // red, a quarter covered
        assert!(!generate_mips(&mut img, true)); // only once
    }

    #[test]
    fn cutout_mips_keep_their_coverage() {
        // 4x4 sparse strands: 4 of 16 texels opaque, one per 2x2 block, so the
        // averaged mip is a quarter covered everywhere and a 0.5 cutoff would
        // drop it all.
        let mut data = vec![0u8; 4 * 4 * 4];
        for (x, y) in [(0, 0), (2, 0), (1, 3), (3, 2)] {
            data[(y * 4 + x) * 4..][..4].copy_from_slice(&[40, 30, 20, 255]);
        }
        let mut img = Image::new(
            Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        assert!(generate_mips(&mut img, true));
        let mip1: Vec<u8> = img.data[64..80].chunks_exact(4).map(|p| p[3]).collect();
        assert_eq!(mip1, [64; 4]);
        preserve_coverage(&mut img, 0.5);
        // A quarter of the texels drawn at full size; all four mip texels tie,
        // so all are lifted to the cutoff (the nearest share a 2x2 level has).
        let mip1: Vec<u8> = img.data[64..80].chunks_exact(4).map(|p| p[3]).collect();
        assert!(mip1.iter().all(|&a| a >= 128), "{mip1:?}");
        assert_eq!(&img.data[..64].chunks_exact(4).filter(|p| p[3] == 255).count(), &4, "level 0 untouched");
    }

    #[test]
    fn coverage_scaling_leaves_dense_levels_alone() {
        let mut img = Image::new(
            Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
            TextureDimension::D2,
            vec![9, 9, 9, 255, 9, 9, 9, 255, 9, 9, 9, 255, 9, 9, 9, 0],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        assert!(generate_mips(&mut img, true));
        let before = img.data.clone();
        preserve_coverage(&mut img, 0.5);
        assert_eq!(img.data, before, "the 3/4-covered mip is already over the cutoff");
    }
}
