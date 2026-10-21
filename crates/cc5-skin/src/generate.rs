//! Makeup and skin details generated from the head's region masks (CC5's RLHead
//! maps: `Cheek Fore UpperLip Chin Mask`, `Nose Mouth UpperInnerLid Mask`,
//! `Ear Neck Mask`), any colour, no capture: lipstick, blush, eyeshadow,
//! contour, highlight, freckles, stubble. Each makes a [`LayerImage`] over the
//! masks' UV square, laid over the head's diffuse by [`crate::compose`] like a
//! captured layer.
use crate::LayerImage;
use serde::{Deserialize, Serialize};

/// The head's region masks (RGBA8, same size): channel order as CC5's maps.
pub struct Masks {
    pub w: u32,
    pub h: u32,
    /// R cheek, G forehead, B upper lip, A chin
    pub cfulc: Vec<u8>,
    /// R nose, G mouth (lips), B upper lid, A inner lid
    pub nmuil: Vec<u8>,
    /// R neck / jaw, G ear
    pub en: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Lipstick,
    Blush,
    Eyeshadow,
    Contour,
    Highlight,
    Freckles,
    Stubble,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Lipstick, Kind::Blush, Kind::Eyeshadow, Kind::Contour, Kind::Highlight, Kind::Freckles, Kind::Stubble];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Lipstick => "Lipstick",
            Kind::Blush => "Blush",
            Kind::Eyeshadow => "Eyeshadow",
            Kind::Contour => "Contour",
            Kind::Highlight => "Highlight",
            Kind::Freckles => "Freckles",
            Kind::Stubble => "Stubble",
        }
    }

    /// A starting colour (sRGB 0..255) and strength.
    pub fn default_look(self) -> ([u8; 3], f32) {
        match self {
            Kind::Lipstick => ([160, 48, 58], 0.7),
            Kind::Blush => ([214, 106, 112], 0.3),
            Kind::Eyeshadow => ([96, 62, 70], 0.55),
            Kind::Contour => ([110, 72, 56], 0.35),
            Kind::Highlight => ([255, 236, 220], 0.25),
            Kind::Freckles => ([126, 74, 48], 0.6),
            Kind::Stubble => ([42, 32, 28], 0.8),
        }
    }
}

/// What a generator makes: the colour layer and, for lipstick, a roughness layer
/// (R = roughness, A = opacity) for its finish.
pub struct Generated {
    pub color: LayerImage,
    pub roughness: Option<LayerImage>,
}

fn channel(m: &[u8], c: usize, w: u32, h: u32) -> Vec<f32> {
    (0..(w * h) as usize).map(|i| m[i * 4 + c] as f32 / 255.0).collect()
}

/// Separable box blur of radius `r`, `passes` times (a soft Gaussian-ish
/// feather); edges clamp.
fn blur(a: &[f32], w: u32, h: u32, r: u32, passes: u32) -> Vec<f32> {
    let (w, h, r) = (w as usize, h as usize, r as isize);
    // one 1-D pass over `n` samples, read at `at(k)`
    let pass = |n: usize, at: &dyn Fn(usize) -> f32| -> Vec<f32> {
        let mut pre = vec![0.0f32; n + 1];
        for k in 0..n {
            pre[k + 1] = pre[k] + at(k);
        }
        let first = at(0);
        let last = at(n - 1);
        (0..n as isize)
            .map(|k| {
                let (lo, hi) = (k - r, k + r);
                let inside = pre[hi.min(n as isize - 1) as usize + 1] - pre[lo.max(0) as usize];
                let before = (-lo).max(0) as f32 * first;
                let after = (hi - (n as isize - 1)).max(0) as f32 * last;
                (inside + before + after) / (2 * r + 1) as f32
            })
            .collect()
    };
    let mut cur = a.to_vec();
    for _ in 0..passes {
        let mut tmp = vec![0.0; w * h];
        for y in 0..h {
            let row = pass(w, &|x| cur[y * w + x]);
            tmp[y * w..(y + 1) * w].copy_from_slice(&row);
        }
        for x in 0..w {
            let col = pass(h, &|y| tmp[y * w + x]);
            for y in 0..h {
                cur[y * w + x] = col[y];
            }
        }
    }
    cur
}

fn hash(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

/// Speckle: round dots of radius `r` (texels) at density `d` (dots per texel), soft-edged.
fn speckle(w: u32, h: u32, r: f32, d: f32, seed: u32) -> Vec<f32> {
    let cell = (1.0 / d.max(1e-6)).sqrt().max(2.0 * r + 1.0);
    let cells = ((w as f32 / cell).ceil() as u32, (h as f32 / cell).ceil() as u32);
    let mut out = vec![0.0f32; (w * h) as usize];
    for cy in 0..cells.1 {
        for cx in 0..cells.0 {
            let (px, py) = ((cx as f32 + hash(cx, cy, seed)) * cell, (cy as f32 + hash(cy, cx, seed ^ 0x9e37)) * cell);
            let rr = r * (0.6 + 0.8 * hash(cx ^ 0x55, cy, seed));
            let k = 0.5 + 0.5 * hash(cx, cy ^ 0x77, seed);
            let (x0, x1) = ((px - rr - 1.0).max(0.0) as u32, ((px + rr + 1.0) as u32).min(w - 1));
            let (y0, y1) = ((py - rr - 1.0).max(0.0) as u32, ((py + rr + 1.0) as u32).min(h - 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let dd = ((x as f32 - px).powi(2) + (y as f32 - py).powi(2)).sqrt();
                    let v = ((rr - dd) / 0.8 + 0.5).clamp(0.0, 1.0) * k;
                    let o = &mut out[(y * w + x) as usize];
                    *o = o.max(v);
                }
            }
        }
    }
    out
}

/// Blobs of the mask channel `a` grown towards -y (up the face in CC5's head UVs:
/// eyes are above their lid marks) and feathered: eyeshadow around the lid marks.
fn eye_area(lid: &[f32], w: u32, h: u32) -> Vec<f32> {
    let s = w as f32 / 1024.0;
    // the lid marks are small: grow them into the lid / brow-bone area
    let grown = blur(lid, w, h, (14.0 * s) as u32, 2);
    let max = grown.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    let mut out = vec![0.0; (w * h) as usize];
    let shift = (10.0 * s) as usize;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let src = y + shift;
            if src < h as usize {
                out[y * w as usize + x] = (grown[src * w as usize + x] / max * 2.2).min(1.0);
            }
        }
    }
    blur(&out, w, h, (4.0 * s) as u32, 2)
}

/// The generator `kind` at colour `rgb`; `seed` varies freckles and stubble.
pub fn generate(m: &Masks, kind: Kind, rgb: [u8; 3], seed: u32) -> Generated {
    let (w, h) = (m.w, m.h);
    let s = w as f32 / 1024.0;
    let cheek = channel(&m.cfulc, 0, w, h);
    let fore = channel(&m.cfulc, 1, w, h);
    let upper_lip = channel(&m.cfulc, 2, w, h);
    let chin = channel(&m.cfulc, 3, w, h);
    let nose = channel(&m.nmuil, 0, w, h);
    let mouth = channel(&m.nmuil, 1, w, h);
    let lid = channel(&m.nmuil, 2, w, h);
    let neck = channel(&m.en, 0, w, h);
    let alpha: Vec<f32> = match kind {
        Kind::Lipstick => blur(&mouth, w, h, (1.5 * s).max(1.0) as u32, 1),
        Kind::Blush => blur(&cheek, w, h, (20.0 * s) as u32, 3).iter().map(|v| v.min(1.0)).collect(),
        Kind::Eyeshadow => eye_area(&lid, w, h),
        Kind::Contour => {
            // under the cheekbones and along the jaw: the cheek mask's lower edge
            // and the neck mask's top band, soft
            let low_cheek: Vec<f32> = (0..(w * h) as usize).map(|i| {
                let y = i / w as usize;
                let below = cheek.get(i.saturating_sub(w as usize * (24.0 * s) as usize)).copied().unwrap_or(0.0);
                if y > (24.0 * s) as usize { (below - cheek[i]).max(0.0) } else { 0.0 }
            }).collect();
            let jaw: Vec<f32> = (0..(w * h) as usize).map(|i| {
                let above = if i >= (w as usize) * (30.0 * s) as usize { neck[i - w as usize * (30.0 * s) as usize] } else { 0.0 };
                (neck[i] - above).max(0.0)
            }).collect();
            blur(&low_cheek.iter().zip(&jaw).map(|(a, b)| (a + b * 0.8).min(1.0)).collect::<Vec<_>>(), w, h, (8.0 * s) as u32, 3)
        }
        Kind::Highlight => {
            let high_cheek: Vec<f32> = (0..(w * h) as usize).map(|i| {
                let above = cheek.get(i + w as usize * (24.0 * s) as usize).copied().unwrap_or(0.0);
                (above - cheek[i]).max(0.0)
            }).collect();
            // the nose bridge, the cheekbones' upper edge and the forehead's middle, faint
            let a: Vec<f32> = (0..(w * h) as usize).map(|i| (nose[i] * 0.45 + high_cheek[i] * 0.5 + fore[i] * 0.25).min(1.0)).collect();
            blur(&a, w, h, (14.0 * s) as u32, 3)
        }
        Kind::Freckles => {
            let region = blur(&(0..(w * h) as usize).map(|i| (cheek[i] + nose[i] + fore[i] * 0.3).min(1.0)).collect::<Vec<_>>(), w, h, (16.0 * s) as u32, 3);
            let dots = speckle(w, h, 1.1 * s, 0.006 / (s * s), seed);
            region.iter().zip(&dots).map(|(r, d)| (r * 1.4).min(1.0) * d).collect()
        }
        Kind::Stubble => {
            let jaw_band: Vec<f32> = (0..(w * h) as usize).map(|i| {
                let above = if i >= (w as usize) * (60.0 * s) as usize { neck[i - w as usize * (60.0 * s) as usize] } else { 0.0 };
                (neck[i] - above).max(0.0)
            }).collect();
            let region = blur(&(0..(w * h) as usize).map(|i| (upper_lip[i] + chin[i] + jaw_band[i]).min(1.0)).collect::<Vec<_>>(), w, h, (6.0 * s) as u32, 2);
            let dots = speckle(w, h, 0.55 * s, 0.09 / (s * s), seed);
            region.iter().zip(&dots).map(|(r, d)| (r * 1.3).min(1.0) * d).collect()
        }
    };
    let rgba: Vec<u8> = alpha.iter().flat_map(|a| [rgb[0], rgb[1], rgb[2], (a.clamp(0.0, 1.0) * 255.0).round() as u8]).collect();
    let color = LayerImage { rect: [0, 0, w, h], size: w, w, h, rgba };
    // lipstick: a satin finish (rougher than bare lips' wet sheen is CC5's matte look)
    let roughness = (kind == Kind::Lipstick).then(|| {
        let rgba = alpha.iter().flat_map(|a| [150u8, 150, 150, (a.clamp(0.0, 1.0) * 255.0).round() as u8]).collect();
        LayerImage { rect: [0, 0, w, h], size: w, w, h, rgba }
    });
    Generated { color, roughness }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lipstick_follows_the_mouth_mask() {
        let (w, h) = (64u32, 64u32);
        let mut nmuil = vec![0u8; (w * h * 4) as usize];
        for y in 30..34 {
            for x in 20..44 {
                nmuil[((y * w + x) * 4 + 1) as usize] = 255;
            }
        }
        let m = Masks { w, h, cfulc: vec![0; (w * h * 4) as usize], nmuil, en: vec![0; (w * h * 4) as usize] };
        let g = generate(&m, Kind::Lipstick, [200, 0, 0], 1);
        let a = |x: u32, y: u32| g.color.rgba[((y * w + x) * 4 + 3) as usize];
        assert!(a(32, 32) > 200 && a(5, 5) == 0);
        assert!(g.roughness.is_some());
        let f = generate(&m, Kind::Freckles, [100, 50, 30], 1);
        assert!(f.color.rgba.chunks(4).all(|p| p[3] == 0), "no freckles outside the cheek / nose / forehead regions");
    }

    #[test]
    fn blur_keeps_flat_fields_flat() {
        let a = vec![0.5f32; 32 * 32];
        let b = blur(&a, 32, 32, 3, 2);
        assert!(b.iter().all(|v| (v - 0.5).abs() < 1e-4));
    }
}
