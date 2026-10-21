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
pub mod generate;

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
    /// normal map texels the layer lays down (RGB, tangent space as CC5's maps)
    /// with their opacity (A), and where they sit, when it changes normals
    /// (scars, wrinkles, acne)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<(String, [u32; 4])>,
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

    /// A layer's normal map for `material`, decoded (RGB = normal, A = opacity).
    pub fn layer_normal(&self, layer: &Layer, material: &str) -> Result<Option<LayerImage>> {
        let Some(m) = layer.maps.get(material) else { return Ok(None) };
        let Some((f, rect)) = &m.normal else { return Ok(None) };
        self.decode(f, *rect).map(Some)
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

/// Where a layer goes, relative to where it was captured or generated: moved
/// (`offset`, as a fraction of the texture), scaled and rotated (degrees) about
/// its rect's centre, as SkinGen places a layer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Xf {
    pub offset: [f32; 2],
    pub scale: f32,
    pub rotation: f32,
}

impl Default for Xf {
    fn default() -> Self {
        Self { offset: [0.0, 0.0], scale: 1.0, rotation: 0.0 }
    }
}

impl Xf {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// One layer to lay over a texture.
pub struct Apply<'a> {
    pub image: &'a LayerImage,
    /// opacity scale (CC5's layer strength), 0..1 (above 1: stronger, clamped per texel)
    pub strength: f32,
    /// (new colour, the layer's [`Layer::color`]), sRGB 0..255: recolour the
    /// layer, keeping its pattern (each texel's brightness relative to the mean)
    pub tint: Option<([f32; 3], [f32; 3])>,
    /// 0..1: how much the skin's own shading (each texel's brightness) shows
    /// through the layer's colour (generated lipstick: the lips' texture)
    pub detail: f32,
    pub xf: Xf,
}

/// Runs `f(texel, sample, cos/sin of the rotation)` for every texel of a
/// `w` x `h` RGBA8 texture the placed layer covers, in parallel; `sample` is the
/// layer's bilinear RGBA there.
fn each_texel(buf: &mut [u8], w: u32, h: u32, li: &LayerImage, xf: Xf, f: impl Fn(&mut [u8], [f32; 4]) + Sync) {
    let k = w as f32 / li.size as f32;
    let (rx, ry) = (li.rect[0] as f32 * k, li.rect[1] as f32 * k);
    let (rw, rh) = (li.rect[2] as f32 * k, li.rect[3] as f32 * k);
    let (cx, cy) = (rx + rw * 0.5, ry + rh * 0.5);
    let (ox, oy) = (xf.offset[0] * w as f32, xf.offset[1] * h as f32);
    let sc = xf.scale.max(1e-3);
    let (sn, cs) = xf.rotation.to_radians().sin_cos();
    // destination bounds: the rect's corners, placed
    let corners = [(rx, ry), (rx + rw, ry), (rx, ry + rh), (rx + rw, ry + rh)].map(|(x, y)| {
        let (dx, dy) = ((x - cx) * sc, (y - cy) * sc);
        (cx + ox + dx * cs - dy * sn, cy + oy + dx * sn + dy * cs)
    });
    let x0 = corners.iter().map(|c| c.0).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
    let x1 = (corners.iter().map(|c| c.0).fold(f32::MIN, f32::max).ceil().max(0.0) as u32).min(w);
    let y0 = corners.iter().map(|c| c.1).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
    let y1 = (corners.iter().map(|c| c.1).fold(f32::MIN, f32::max).ceil().max(0.0) as u32).min(h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    buf.par_chunks_mut(w as usize * 4).enumerate().filter(|(y, _)| (*y as u32) >= y0 && (*y as u32) < y1).for_each(|(y, row)| {
        for x in x0..x1 {
            // texel centre -> the layer's own frame (inverse placement) -> its image
            let (px, py) = (x as f32 + 0.5 - cx - ox, y as f32 + 0.5 - cy - oy);
            let (ux, uy) = ((px * cs + py * sn) / sc + cx, (-px * sn + py * cs) / sc + cy);
            if ux < rx || uy < ry || ux >= rx + rw || uy >= ry + rh {
                continue;
            }
            let lx = ((ux - rx) / rw * li.w as f32 - 0.5).clamp(0.0, li.w as f32 - 1.0);
            let ly = ((uy - ry) / rh * li.h as f32 - 0.5).clamp(0.0, li.h as f32 - 1.0);
            f(&mut row[x as usize * 4..x as usize * 4 + 4], bilinear(li, lx, ly));
        }
    });
}

/// Lays layers' roughness (their maps' R, at their opacity x strength) over the
/// G channel of a glTF metallic-roughness texture (RGBA8, `w` x `h`), in place.
pub fn compose_roughness(mr: &mut [u8], w: u32, h: u32, layers: &[(&LayerImage, f32, Xf)]) {
    for (li, strength, xf) in layers {
        if *strength <= 0.0 {
            continue;
        }
        each_texel(mr, w, h, li, *xf, |p, s| {
            let a = (s[3] / 255.0 * strength).clamp(0.0, 1.0);
            p[1] = (p[1] as f32 * (1.0 - a) + s[0] * a).round().clamp(0.0, 255.0) as u8;
        });
    }
}

/// Lays layers' normals over a normal map (RGBA8, `w` x `h`), in place: the
/// unpacked normals (turned with the layer's rotation) blend by the layers'
/// opacity x strength and renormalize.
pub fn compose_normal(nm: &mut [u8], w: u32, h: u32, layers: &[(&LayerImage, f32, Xf)]) {
    for (li, strength, xf) in layers {
        if *strength <= 0.0 {
            continue;
        }
        let (sn, cs) = xf.rotation.to_radians().sin_cos();
        each_texel(nm, w, h, li, *xf, |p, s| {
            let a = (s[3] / 255.0 * strength).clamp(0.0, 1.0);
            if a <= 0.0 {
                return;
            }
            let l = [0, 1, 2].map(|c| s[c] / 255.0 * 2.0 - 1.0);
            let l = [l[0] * cs - l[1] * sn, l[0] * sn + l[1] * cs, l[2]];
            let n: [f32; 3] = [0, 1, 2].map(|c| (p[c] as f32 / 255.0 * 2.0 - 1.0) * (1.0 - a) + l[c] * a);
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
            for c in 0..3 {
                p[c] = ((n[c] / len * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        });
    }
}

/// Lays `layers` over `base` (RGBA8 sRGB, `w` x `h`, any size: rects scale) in
/// order, in place. CC5 blends layers in sRGB.
pub fn compose(base: &mut [u8], w: u32, h: u32, layers: &[Apply]) {
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for a in layers {
        if a.strength <= 0.0 {
            continue;
        }
        let tint = a.tint.map(|(to, from)| (to, 1.0 / lum(from).max(1.0)));
        each_texel(base, w, h, a.image, a.xf, |p, s| {
            let alpha = (s[3] / 255.0 * a.strength).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                return;
            }
            let rel = tint.map(|(_, k)| lum([s[0], s[1], s[2]]) * k);
            // the skin's shading through the colour (relative to a mid skin tone)
            let shade = 1.0 + a.detail * (lum([p[0] as f32, p[1] as f32, p[2] as f32]) / 115.0 - 1.0);
            for c in 0..3 {
                let col = match (tint, rel) {
                    (Some((to, _)), Some(r)) => (to[c] * r).min(255.0),
                    _ => s[c],
                } * shade;
                p[c] = (p[c] as f32 * (1.0 - alpha) + col.min(255.0) * alpha).round().clamp(0.0, 255.0) as u8;
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
        compose(&mut base, 2, 2, &[Apply { image: &li, strength: 0.5, tint: None, detail: 0.0, xf: Xf::default() }]);
        assert_eq!(&base[0..3], &[150, 50, 50]);
        let mut base = [100u8, 100, 100, 255].repeat(4);
        compose(&mut base, 2, 2, &[Apply { image: &li, strength: 1.0, tint: Some(([0.0, 0.0, 200.0], [200.0, 0.0, 0.0])), detail: 0.0, xf: Xf::default() }]);
        assert_eq!(&base[0..3], &[0, 0, 200]);
    }

    #[test]
    fn placement_moves_the_layer() {
        // a 2x2 red patch at the top-left of a 8x8 texture, moved by half the texture
        let li = LayerImage { rect: [0, 0, 2, 2], size: 8, w: 2, h: 2, rgba: [255, 0, 0, 255].repeat(4) };
        let mut base = [0u8, 0, 0, 255].repeat(64);
        let xf = Xf { offset: [0.5, 0.5], scale: 1.0, rotation: 0.0 };
        compose(&mut base, 8, 8, &[Apply { image: &li, strength: 1.0, tint: None, detail: 0.0, xf }]);
        let px = |x: usize, y: usize| base[(y * 8 + x) * 4];
        assert_eq!((px(0, 0), px(4, 4), px(5, 5)), (0, 255, 255));
    }

    #[test]
    fn compose_normal_blends_and_renormalizes() {
        // flat normal (0, 0, 1) blended halfway towards (1, 0, 0)
        let li = LayerImage { rect: [0, 0, 1, 1], size: 1, w: 1, h: 1, rgba: vec![255, 128, 128, 255] };
        let mut nm = vec![128u8, 128, 255, 255];
        compose_normal(&mut nm, 1, 1, &[(&li, 0.5, Xf::default())]);
        let n: Vec<f32> = nm[..3].iter().map(|&v| v as f32 / 255.0 * 2.0 - 1.0).collect();
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 0.02 && (n[0] - n[2]).abs() < 0.03, "{n:?}");
    }
}
