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
//!   (sRGB RGBA PNG, cropped to where it is visible) and a [`Blend`] mode, solved
//!   from CC5's textures of two skin bases with and without the layer (the mode
//!   that explains both best: normal base·(1−α) + C·α for makeup colour,
//!   multiply for tattoos / contour / tans / beards / freckles, screen for
//!   highlights). [`compose`] lays layers over any base at a strength (α·s),
//!   optionally recoloured (C scaled towards a tint, keeping its pattern);
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

/// How a layer's colour `C` at opacity `α` combines with the texel under it `B`
/// (sRGB 0..1), as SkinGen's layer blend modes do.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Blend {
    /// B·(1−α) + C·α (makeup colour)
    #[default]
    Normal,
    /// B·(1 − α + α·C) (tattoos, contour, tans, beards, freckles)
    Multiply,
    /// 1 − (1−B)·(1 − α·C) (highlights)
    Screen,
}

impl Blend {
    pub fn is_normal(&self) -> bool {
        *self == Blend::Normal
    }

    /// The texel under the layer `b`, with colour `c` at opacity `a` laid over it.
    pub fn apply(self, b: f32, c: f32, a: f32) -> f32 {
        match self {
            Blend::Normal => b * (1.0 - a) + c * a,
            Blend::Multiply => b * (1.0 - a + a * c),
            Blend::Screen => 1.0 - (1.0 - b) * (1.0 - a * c),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LayerMap {
    /// RGBA PNG (sRGB colour, opacity), relative to the pack
    pub diffuse: String,
    /// where it sits in the material's texture: x, y, w, h (texels at `size`)
    pub rect: [u32; 4],
    /// how its colour combines with the skin under it
    #[serde(default, skip_serializing_if = "Blend::is_normal")]
    pub blend: Blend,
    /// parts in other blend modes, laid after it (same rect)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<LayerPart>,
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

/// Part of a layer map in another blend mode than the map's (RGBA PNG like it).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LayerPart {
    pub diffuse: String,
    pub blend: Blend,
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
        let mut li = self.decode(&m.diffuse, m.rect, m.blend)?;
        for p in &m.parts {
            li.parts.push(self.decode(&p.diffuse, m.rect, p.blend)?);
        }
        Ok(Some(li))
    }

    /// A layer's roughness map for `material`, decoded (R = roughness, A = opacity).
    pub fn layer_roughness(&self, layer: &Layer, material: &str) -> Result<Option<LayerImage>> {
        let Some(m) = layer.maps.get(material) else { return Ok(None) };
        let Some(r) = &m.roughness else { return Ok(None) };
        self.decode(r, m.rect, Blend::Normal).map(Some)
    }

    /// A layer's normal map for `material`, decoded (RGB = normal, A = opacity).
    pub fn layer_normal(&self, layer: &Layer, material: &str) -> Result<Option<LayerImage>> {
        let Some(m) = layer.maps.get(material) else { return Ok(None) };
        let Some((f, rect)) = &m.normal else { return Ok(None) };
        self.decode(f, *rect, Blend::Normal).map(Some)
    }

    fn decode(&self, file: &str, rect: [u32; 4], blend: Blend) -> Result<LayerImage> {
        let img = image::open(self.dir.join(file)).with_context(|| file.to_string())?.to_rgba8();
        Ok(LayerImage { rect, size: self.size, w: img.width(), h: img.height(), rgba: img.into_raw(), blend, parts: Vec::new() })
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
    /// how the colour combines with the texel under it (colour maps)
    pub blend: Blend,
    /// parts in other blend modes (same rect), laid after this one
    pub parts: Vec<LayerImage>,
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

/// SkinGen's "Adjust Color" of a layer's colour: hue turn (degrees), and
/// saturation, brightness and contrast (-1..1, 0 = as it is).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Adjust {
    #[serde(default)]
    pub hue: f32,
    #[serde(default)]
    pub saturation: f32,
    #[serde(default)]
    pub brightness: f32,
    #[serde(default)]
    pub contrast: f32,
}

impl Adjust {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// The adjusted colour (sRGB 0..255).
    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        if self.is_identity() {
            return c;
        }
        let mut v = c.map(|x| x / 255.0);
        if self.hue != 0.0 {
            // rotation about the grey axis
            let (sn, cs) = self.hue.to_radians().sin_cos();
            let k = (1.0 - cs) / 3.0;
            let q = sn / 3f32.sqrt();
            let m = [[cs + k, k - q, k + q], [k + q, cs + k, k - q], [k - q, k + q, cs + k]];
            v = [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]);
        }
        let l = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        let sat = 1.0 + self.saturation;
        let bright = 2f32.powf(self.brightness * 1.5);
        let con = 1.0 + self.contrast;
        [0, 1, 2].map(|i| (((l + (v[i] - l) * sat) * bright - 0.5) * con + 0.5).clamp(0.0, 1.0) * 255.0)
    }
}

/// A layer mask over a material's whole texture (0..1 per texel, any size):
/// multiplies the layer's opacity where the layer lands (SkinGen's "Layer Mask").
#[derive(Clone, Debug)]
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub v: Vec<f32>,
}

impl Mask {
    /// At texture coordinates `u`, `v` (0..1), bilinear.
    pub fn at(&self, u: f32, v: f32) -> f32 {
        let x = (u * self.w as f32 - 0.5).clamp(0.0, self.w as f32 - 1.0);
        let y = (v * self.h as f32 - 0.5).clamp(0.0, self.h as f32 - 1.0);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let g = |x: u32, y: u32| self.v[(y * self.w + x) as usize];
        (g(x0, y0) * (1.0 - fx) + g(x1, y0) * fx) * (1.0 - fy) + (g(x0, y1) * (1.0 - fx) + g(x1, y1) * fx) * fy
    }

    /// 1 − the mask.
    pub fn inverted(mut self) -> Self {
        self.v.iter_mut().for_each(|x| *x = 1.0 - *x);
        self
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
    /// colour adjustment (after the tint)
    pub adjust: Adjust,
    /// opacity mask over the texture
    pub mask: Option<&'a Mask>,
}

impl<'a> Apply<'a> {
    /// `image` as it is: full strength, no tint, adjustment or mask.
    pub fn new(image: &'a LayerImage) -> Self {
        Self { image, strength: 1.0, tint: None, detail: 0.0, xf: Xf::default(), adjust: Adjust::default(), mask: None }
    }
}

/// Runs `f(texel, sample, mask)` for every texel of a `w` x `h` RGBA8 texture the
/// placed layer covers, in parallel; `sample` is the layer's bilinear RGBA there
/// and `mask` the mask's value at the texel (1 without one).
fn each_texel(buf: &mut [u8], w: u32, h: u32, li: &LayerImage, xf: Xf, mask: Option<&Mask>, f: impl Fn(&mut [u8], [f32; 4], f32) + Sync) {
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
            let m = mask.map_or(1.0, |m| m.at((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32));
            if m > 0.0 {
                f(&mut row[x as usize * 4..x as usize * 4 + 4], bilinear(li, lx, ly), m);
            }
        }
    });
}

/// Lays layers' roughness (their maps' R, at their opacity x strength) over the
/// G channel of a glTF metallic-roughness texture (RGBA8, `w` x `h`), in place.
pub fn compose_roughness(mr: &mut [u8], w: u32, h: u32, layers: &[(&LayerImage, f32, Xf, Option<&Mask>)]) {
    for (li, strength, xf, mask) in layers {
        if *strength <= 0.0 {
            continue;
        }
        each_texel(mr, w, h, li, *xf, *mask, |p, s, m| {
            let a = (s[3] / 255.0 * strength * m).clamp(0.0, 1.0);
            p[1] = (p[1] as f32 * (1.0 - a) + s[0] * a).round().clamp(0.0, 255.0) as u8;
        });
    }
}

/// Lays layers' normals over a normal map (RGBA8, `w` x `h`), in place: the
/// unpacked normals (turned with the layer's rotation) blend by the layers'
/// opacity x strength and renormalize.
pub fn compose_normal(nm: &mut [u8], w: u32, h: u32, layers: &[(&LayerImage, f32, Xf, Option<&Mask>)]) {
    for (li, strength, xf, mask) in layers {
        if *strength <= 0.0 {
            continue;
        }
        let (sn, cs) = xf.rotation.to_radians().sin_cos();
        each_texel(nm, w, h, li, *xf, *mask, |p, s, m| {
            let a = (s[3] / 255.0 * strength * m).clamp(0.0, 1.0);
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
/// order, in place, each by its [`Blend`] (colour tinted, then adjusted; opacity
/// masked). CC5 blends layers in sRGB.
pub fn compose(base: &mut [u8], w: u32, h: u32, layers: &[Apply]) {
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for a in layers {
        if a.strength <= 0.0 {
            continue;
        }
        let tint = a.tint.map(|(to, from)| (to, 1.0 / lum(from).max(1.0)));
        for li in std::iter::once(a.image).chain(&a.image.parts) {
            each_texel(base, w, h, li, a.xf, a.mask, |p, s, m| {
                let alpha = (s[3] / 255.0 * a.strength * m).clamp(0.0, 1.0);
                if alpha <= 0.0 {
                    return;
                }
                let rel = tint.map(|(_, k)| lum([s[0], s[1], s[2]]) * k);
                let col = [0, 1, 2].map(|c| match (tint, rel) {
                    (Some((to, _)), Some(r)) => (to[c] * r).min(255.0),
                    _ => s[c],
                });
                let col = a.adjust.apply(col);
                // the skin's shading through the colour (relative to a mid skin tone)
                let shade = 1.0 + a.detail * (lum([p[0] as f32, p[1] as f32, p[2] as f32]) / 115.0 - 1.0);
                for c in 0..3 {
                    let v = li.blend.apply(p[c] as f32 / 255.0, (col[c] * shade).min(255.0) / 255.0, alpha);
                    p[c] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            });
        }
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
        let li = LayerImage { rect: [0, 0, 2, 2], size: 2, w: 2, h: 2, rgba: [200, 0, 0, 255].repeat(4), blend: Blend::Normal, parts: Vec::new() };
        let mut base = [100u8, 100, 100, 255].repeat(4);
        compose(&mut base, 2, 2, &[Apply { strength: 0.5, ..Apply::new(&li) }]);
        assert_eq!(&base[0..3], &[150, 50, 50]);
        let mut base = [100u8, 100, 100, 255].repeat(4);
        compose(&mut base, 2, 2, &[Apply { tint: Some(([0.0, 0.0, 200.0], [200.0, 0.0, 0.0])), ..Apply::new(&li) }]);
        assert_eq!(&base[0..3], &[0, 0, 200]);
    }

    #[test]
    fn placement_moves_the_layer() {
        // a 2x2 red patch at the top-left of a 8x8 texture, moved by half the texture
        let li = LayerImage { rect: [0, 0, 2, 2], size: 8, w: 2, h: 2, rgba: [255, 0, 0, 255].repeat(4), blend: Blend::Normal, parts: Vec::new() };
        let mut base = [0u8, 0, 0, 255].repeat(64);
        let xf = Xf { offset: [0.5, 0.5], scale: 1.0, rotation: 0.0 };
        compose(&mut base, 8, 8, &[Apply { xf, ..Apply::new(&li) }]);
        let px = |x: usize, y: usize| base[(y * 8 + x) * 4];
        assert_eq!((px(0, 0), px(4, 4), px(5, 5)), (0, 255, 255));
    }

    #[test]
    fn mask_and_adjust() {
        // a red layer over grey, masked to the left half; the adjustment turns it blue-ish
        let li = LayerImage { rect: [0, 0, 2, 1], size: 2, w: 2, h: 1, rgba: [200, 0, 0, 255].repeat(2), blend: Blend::Normal, parts: Vec::new() };
        let mask = Mask { w: 2, h: 1, v: vec![1.0, 0.0] };
        let mut base = [100u8, 100, 100, 255].repeat(2);
        compose(&mut base, 2, 1, &[Apply { mask: Some(&mask), ..Apply::new(&li) }]);
        assert_eq!((base[0], base[4]), (200, 100), "masked half untouched");
        let turned = Adjust { hue: 240.0, ..Default::default() }.apply([200.0, 0.0, 0.0]);
        assert!(turned[2] > 150.0 && turned[0] < 50.0, "{turned:?}");
        assert_eq!(Adjust::default().apply([10.0, 20.0, 30.0]), [10.0, 20.0, 30.0]);
    }

    #[test]
    fn compose_normal_blends_and_renormalizes() {
        // flat normal (0, 0, 1) blended halfway towards (1, 0, 0)
        let li = LayerImage { rect: [0, 0, 1, 1], size: 1, w: 1, h: 1, rgba: vec![255, 128, 128, 255], blend: Blend::Normal, parts: Vec::new() };
        let mut nm = vec![128u8, 128, 255, 255];
        compose_normal(&mut nm, 1, 1, &[(&li, 0.5, Xf::default(), None)]);
        let n: Vec<f32> = nm[..3].iter().map(|&v| v as f32 / 255.0 * 2.0 - 1.0).collect();
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 0.02 && (n[0] - n[2]).abs() < 0.03, "{n:?}");
    }
}
