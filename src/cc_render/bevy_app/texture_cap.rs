//! Cap texture size (`CC_AVATAR_MAX_TEXTURE`, pixels per side; unset = no
//! cap). A streamed close-up is a few hundred pixels across, but CC5
//! characters carry 2-4K maps (Beautiful ~1.5 GB of RGBA8, Valen ~3 GB),
//! which crowd a game sharing the GPU. Each RGBA8 texture larger than the
//! cap is halved (2x2 box, colour weighted by alpha so cut-out hair cards
//! keep their colour at the edges) until it fits, as it loads and before
//! its first upload; the CC5 materials' mip generation then works on the
//! smaller image.
use bevy::prelude::*;
use bevy::render::render_resource::{TextureDimension, TextureFormat, TextureUsages};

#[derive(Resource)]
pub(crate) struct MaxTexture(pub u32);

impl MaxTexture {
    pub(crate) fn from_env() -> Option<Self> {
        let max: u32 = std::env::var("CC_AVATAR_MAX_TEXTURE").ok()?.trim().parse().ok()?;
        (max >= 16).then_some(Self(max))
    }
}

/// Runs every frame: images are inserted by the asset server earlier in the
/// frame and uploaded at its end, so a texture is shrunk before the GPU
/// ever holds the full size. Shrunk ones no longer match, so each is
/// touched once.
pub(crate) fn cap_texture_size(max: Res<MaxTexture>, mut images: ResMut<Assets<Image>>) {
    let max = max.0;
    let too_big: Vec<AssetId<Image>> = images
        .iter()
        .filter(|(_, img)| {
            let d = &img.texture_descriptor;
            (d.size.width > max || d.size.height > max)
                && matches!(d.format, TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm)
                && d.dimension == TextureDimension::D2
                && d.mip_level_count == 1
                && d.size.depth_or_array_layers == 1
                && !d.usage.contains(TextureUsages::RENDER_ATTACHMENT)
                && img.data.len() == (d.size.width * d.size.height * 4) as usize
        })
        .map(|(id, _)| id)
        .collect();
    for id in too_big {
        let Some(img) = images.get_mut(id) else { continue };
        let (w0, h0) = (img.texture_descriptor.size.width, img.texture_descriptor.size.height);
        let (mut w, mut h) = (w0, h0);
        let mut data = std::mem::take(&mut img.data);
        while w > max || h > max {
            (data, w, h) = halve(&data, w, h);
        }
        img.data = data;
        img.texture_descriptor.size.width = w;
        img.texture_descriptor.size.height = h;
        debug!(target: "cc_render", "texture cap: {w0}x{h0} -> {w}x{h}");
    }
}

/// One 2x2 box step (odd edges repeat their last texel). Colour is
/// averaged weighted by alpha, alpha plainly.
fn halve(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let (w, h) = (w as usize, h as usize);
    let mut out = Vec::with_capacity(nw as usize * nh as usize * 4);
    for y in 0..nh as usize {
        for x in 0..nw as usize {
            let (mut rgb, mut a_sum, mut plain) = ([0u32; 3], 0u32, [0u32; 3]);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((2 * x + dx).min(w - 1), (2 * y + dy).min(h - 1));
                let p = &src[(sy * w + sx) * 4..][..4];
                let a = p[3] as u32;
                for c in 0..3 {
                    rgb[c] += p[c] as u32 * a;
                    plain[c] += p[c] as u32;
                }
                a_sum += a;
            }
            for c in 0..3 {
                out.push(if a_sum > 0 { ((rgb[c] + a_sum / 2) / a_sum) as u8 } else { ((plain[c] + 2) / 4) as u8 });
            }
            out.push(((a_sum + 2) / 4) as u8);
        }
    }
    (out, nw, nh)
}

#[cfg(test)]
mod tests {
    use super::halve;

    #[test]
    fn halves_and_weights_colour_by_alpha() {
        // 2x2: one opaque red texel, three transparent black ones.
        let src = [255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let (out, w, h) = halve(&src, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![255, 0, 0, 64]);
    }

    #[test]
    fn odd_sizes_repeat_the_edge() {
        let src = [10u8; 3 * 1 * 4];
        let (out, w, h) = halve(&src, 3, 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![10, 10, 10, 10]);
    }
}
