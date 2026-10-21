//! Character Creator 5 skin customization for cc-render.
//!
//! CC5 customizes skin with SkinGen: layers (makeup, freckles, moles, stubble,
//! scars, tattoos, ...) composited over a skin base into the materials' textures,
//! and whole skins. Their presets are encrypted, so a [`SkinPack`] holds what CC5
//! makes of them on one character, captured from CC5 itself
//! (`remotemedia-sdk/scripts/avatars/cc5_editor/skin/cc5_skin_capture.py`, then
//! `cc5-skin build`):
//!
//! - a **layer** per preset and material: colour `C` and opacity `α` per texel
//!   (sRGB RGBA PNG, cropped to where it is visible), solved from CC5's textures
//!   of two skin bases with and without the layer: CC5's texel = base·(1−α) + C·α.
//!   [`compose`] lays layers over any base at a strength (α·s), optionally
//!   recoloured (C scaled towards a tint, keeping its pattern);
//! - a **skin**: CC5's whole texture set for a skin preset, and its shader settings.
//!
//! ```json
//! {"format": "cc5-skin-pack", "version": 1, "size": 2048,
//!  "layers": [{"id": "lip_rouge", "name": "Rouge", "group": "Lip Makeup", "color": [190, 40, 40],
//!              "maps": {"Std_Skin_Head": {"diffuse": "layers/lip_rouge.Std_Skin_Head.png", "rect": [x, y, w, h]}}}],
//!  "skins": [{"id": "full_default_male", "name": "Default Male",
//!             "maps": {"Std_Skin_Head": {"Diffuse": "skins/full_default_male/Std_Skin_Head_Diffuse.jpg", ...}},
//!             "vars": {"Std_Skin_Head": {...}}}]}
//! ```
use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod build;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LayerMap {
    /// RGBA PNG (sRGB colour, opacity), relative to the pack
    pub diffuse: String,
    /// where it sits in the material's texture: x, y, w, h (texels at `size`)
    pub rect: [u32; 4],
    /// roughness the layer lays down (R) with the same opacity (A), when it
    /// changes roughness (lipstick, sweat, ...); same rect
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub group: String,
    /// opacity-weighted mean colour (sRGB 0..255): what a tint replaces
    pub color: [f32; 3],
    pub maps: BTreeMap<String, LayerMap>,
    /// the CC5 preset it was captured from
    #[serde(default)]
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Skin {
    pub id: String,
    pub name: String,
    /// material -> CC5 channel / slot -> image relative to the pack
    pub maps: BTreeMap<String, BTreeMap<String, String>>,
    /// material -> CC5 shader variables (`extras.cc5.vars` form)
    #[serde(default)]
    pub vars: BTreeMap<String, Value>,
    #[serde(default)]
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SkinPack {
    pub format: String,
    pub version: u32,
    /// texture size the layer rects are in
    pub size: u32,
    pub layers: Vec<Layer>,
    pub skins: Vec<Skin>,
    #[serde(skip)]
    pub dir: PathBuf,
}

impl SkinPack {
    pub fn load(dir: &Path) -> Result<Self> {
        let p = dir.join("skin.json");
        let mut pack: SkinPack =
            serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?).with_context(|| format!("parsing {}", p.display()))?;
        anyhow::ensure!(pack.format == "cc5-skin-pack", "{}: not a cc5-skin-pack", p.display());
        pack.dir = dir.to_path_buf();
        Ok(pack)
    }

    pub fn save(&self) -> Result<()> {
        std::fs::write(self.dir.join("skin.json"), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn skin(&self, id: &str) -> Option<&Skin> {
        self.skins.iter().find(|s| s.id == id)
    }

    /// A layer's map for `material`, decoded.
    pub fn layer_image(&self, layer: &Layer, material: &str) -> Result<Option<LayerImage>> {
        let Some(m) = layer.maps.get(material) else { return Ok(None) };
        self.decode(&m.diffuse, m.rect).map(Some)
    }

    /// A layer's roughness map for `material`, decoded (R = roughness, A = opacity).
    pub fn layer_roughness(&self, layer: &Layer, material: &str) -> Result<Option<LayerImage>> {
        let Some(m) = layer.maps.get(material) else { return Ok(None) };
        let Some(r) = &m.roughness else { return Ok(None) };
        self.decode(r, m.rect).map(Some)
    }

    fn decode(&self, file: &str, rect: [u32; 4]) -> Result<LayerImage> {
        let img = image::open(self.dir.join(file)).with_context(|| file.to_string())?.to_rgba8();
        Ok(LayerImage { rect, size: self.size, w: img.width(), h: img.height(), rgba: img.into_raw() })
    }
}

/// A decoded layer map.
#[derive(Clone)]
pub struct LayerImage {
    pub rect: [u32; 4],
    /// texture size `rect` is in
    pub size: u32,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// One layer to lay over a texture.
pub struct Apply<'a> {
    pub image: &'a LayerImage,
    /// opacity scale (CC5's layer strength), 0..1 (above 1: stronger, clamped per texel)
    pub strength: f32,
    /// (new colour, the layer's [`Layer::color`]), sRGB 0..255: recolour the
    /// layer, keeping its pattern (each texel's brightness relative to the mean)
    pub tint: Option<([f32; 3], [f32; 3])>,
}

/// Lays layers' roughness (their maps' R, at their opacity x strength) over the
/// G channel of a glTF metallic-roughness texture (RGBA8, `w` x `h`), in place.
pub fn compose_roughness(mr: &mut [u8], w: u32, h: u32, layers: &[(&LayerImage, f32)]) {
    for (li, strength) in layers {
        if *strength <= 0.0 {
            continue;
        }
        let k = w as f32 / li.size as f32;
        let (rx, ry) = (li.rect[0] as f32 * k, li.rect[1] as f32 * k);
        let (rw, rh) = (li.rect[2] as f32 * k, li.rect[3] as f32 * k);
        let (x0, y0) = (rx.floor().max(0.0) as u32, ry.floor().max(0.0) as u32);
        let (x1, y1) = (((rx + rw).ceil() as u32).min(w), ((ry + rh).ceil() as u32).min(h));
        mr.par_chunks_mut(w as usize * 4).enumerate().filter(|(y, _)| (*y as u32) >= y0 && (*y as u32) < y1).for_each(|(y, row)| {
            let ly = ((y as f32 + 0.5 - ry) / rh * li.h as f32 - 0.5).clamp(0.0, li.h as f32 - 1.0);
            for x in x0..x1 {
                let lx = ((x as f32 + 0.5 - rx) / rw * li.w as f32 - 0.5).clamp(0.0, li.w as f32 - 1.0);
                let s = bilinear(li, lx, ly);
                let a = (s[3] / 255.0 * strength).clamp(0.0, 1.0);
                let g = &mut row[x as usize * 4 + 1];
                *g = (*g as f32 * (1.0 - a) + s[0] * a).round().clamp(0.0, 255.0) as u8;
            }
        });
    }
}

/// Lays `layers` over `base` (RGBA8 sRGB, `w` x `h`, any size: rects scale) in
/// order, in place. CC5 blends layers in sRGB.
pub fn compose(base: &mut [u8], w: u32, h: u32, layers: &[Apply]) {
    for a in layers {
        if a.strength <= 0.0 {
            continue;
        }
        let li = a.image;
        let k = w as f32 / li.size as f32;
        let (rx, ry) = (li.rect[0] as f32 * k, li.rect[1] as f32 * k);
        let (rw, rh) = (li.rect[2] as f32 * k, li.rect[3] as f32 * k);
        let (x0, y0) = (rx.floor().max(0.0) as u32, ry.floor().max(0.0) as u32);
        let (x1, y1) = (((rx + rw).ceil() as u32).min(w), ((ry + rh).ceil() as u32).min(h));
        let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let tint = a.tint.map(|(to, from)| (to, 1.0 / lum(from).max(1.0)));
        base.par_chunks_mut(w as usize * 4).enumerate().filter(|(y, _)| (*y as u32) >= y0 && (*y as u32) < y1).for_each(|(y, row)| {
            // texel centre -> layer image coordinates (bilinear)
            let ly = ((y as f32 + 0.5 - ry) / rh * li.h as f32 - 0.5).clamp(0.0, li.h as f32 - 1.0);
            for x in x0..x1 {
                let lx = ((x as f32 + 0.5 - rx) / rw * li.w as f32 - 0.5).clamp(0.0, li.w as f32 - 1.0);
                let s = bilinear(li, lx, ly);
                let alpha = (s[3] / 255.0 * a.strength).clamp(0.0, 1.0);
                if alpha <= 0.0 {
                    continue;
                }
                let p = &mut row[x as usize * 4..x as usize * 4 + 3];
                let rel = tint.map(|(_, k)| lum([s[0], s[1], s[2]]) * k);
                for c in 0..3 {
                    let col = match (tint, rel) {
                        (Some((to, _)), Some(r)) => (to[c] * r).min(255.0),
                        _ => s[c],
                    };
                    p[c] = (p[c] as f32 * (1.0 - alpha) + col * alpha).round().clamp(0.0, 255.0) as u8;
                }
            }
        });
    }
}

fn bilinear(li: &LayerImage, x: f32, y: f32) -> [f32; 4] {
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(li.w - 1), (y0 + 1).min(li.h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: u32, y: u32| {
        let i = (y * li.w + x) as usize * 4;
        [li.rgba[i] as f32, li.rgba[i + 1] as f32, li.rgba[i + 2] as f32, li.rgba[i + 3] as f32]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    // colour weighted by opacity, so transparent texels' colour does not bleed in
    let mut out = [0.0; 4];
    let wts = [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy];
    let mut aw = 0.0;
    for (p, wt) in [a, b, c, d].iter().zip(wts) {
        out[3] += p[3] * wt;
        aw += p[3] * wt;
        for k in 0..3 {
            out[k] += p[k] * p[3] * wt;
        }
    }
    for k in 0..3 {
        out[k] = if aw > 0.0 { out[k] / aw } else { 0.0 };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_over_with_strength_and_tint() {
        let li = LayerImage { rect: [0, 0, 2, 2], size: 2, w: 2, h: 2, rgba: [200, 0, 0, 255].repeat(4) };
        let mut base = [100u8, 100, 100, 255].repeat(4);
        compose(&mut base, 2, 2, &[Apply { image: &li, strength: 0.5, tint: None }]);
        assert_eq!(&base[0..3], &[150, 50, 50]);
        let mut base = [100u8, 100, 100, 255].repeat(4);
        compose(&mut base, 2, 2, &[Apply { image: &li, strength: 1.0, tint: Some(([0.0, 0.0, 200.0], [200.0, 0.0, 0.0])) }]);
        assert_eq!(&base[0..3], &[0, 0, 200]);
    }
}
