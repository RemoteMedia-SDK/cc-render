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
    /// one mole / freckle at `Pattern::at`
    Spot,
}

impl Kind {
    pub const ALL: [Kind; 8] = [Kind::Lipstick, Kind::Blush, Kind::Eyeshadow, Kind::Contour, Kind::Highlight, Kind::Freckles, Kind::Stubble, Kind::Spot];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Lipstick => "Lipstick",
            Kind::Blush => "Blush",
            Kind::Eyeshadow => "Eyeshadow",
            Kind::Contour => "Contour",
            Kind::Highlight => "Highlight",
            Kind::Freckles => "Freckles",
            Kind::Stubble => "Stubble",
            Kind::Spot => "Mole / spot",
        }
    }

    /// A starting colour (sRGB 0..255) and strength: fitted through the shape to
    /// the CC5 preset it stands for (`gen_fit.py`; the fitted shapes carry its
    /// opacity, so mostly strength 1; contour lower: its shape spreads wider than
    /// CC5's, and 0.35 matches CC5's render best).
    pub fn default_look(self) -> ([u8; 3], f32) {
        match self {
            Kind::Lipstick => ([164, 41, 40], 0.95),
            Kind::Blush => ([189, 91, 82], 1.0),
            Kind::Eyeshadow => ([96, 38, 25], 0.73),
            Kind::Contour => ([135, 52, 20], 0.35),
            Kind::Highlight => ([220, 162, 140], 1.0),
            Kind::Freckles => ([117, 17, 5], 0.15),
            Kind::Stubble => ([132, 121, 108], 0.6),
            Kind::Spot => ([120, 76, 52], 0.9),
        }
    }

    /// Its starting pattern: stubble denser than freckles' default.
    pub fn default_pattern(self) -> Pattern {
        match self {
            Kind::Stubble => Pattern { density: 3.0, ..Pattern::default() },
            _ => Pattern::default(),
        }
    }

    /// How it blends, as the CC5 preset it is fitted to does.
    pub fn default_blend(self) -> crate::Blend {
        match self {
            Kind::Contour | Kind::Freckles => crate::Blend::Multiply,
            Kind::Highlight => crate::Blend::Screen,
            _ => crate::Blend::Normal,
        }
    }

    /// Roughness it lays down (0..1), if any: lipstick's satin finish.
    pub fn default_roughness(self) -> Option<f32> {
        (self == Kind::Lipstick).then_some(150.0 / 255.0)
    }
}

/// Freckle / stubble pattern controls: dot size and density (1 = default) and
/// how much dots vary in size and darkness (0 = all alike, 1 = very uneven); and
/// SkinGen's Range controls of the shape: expansion (grow / shrink), blur
/// (feather) and contrast, -1..1 (0 = as fitted; blur 0..1).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Pattern {
    pub size: f32,
    pub density: f32,
    pub variation: f32,
    /// [`Kind::Spot`]: where (UV, 0..1)
    #[serde(default = "centre")]
    pub at: [f32; 2],
    #[serde(default)]
    pub expansion: f32,
    #[serde(default)]
    pub blur: f32,
    #[serde(default)]
    pub contrast: f32,
}

fn centre() -> [f32; 2] {
    [0.5, 0.5]
}

impl Default for Pattern {
    fn default() -> Self {
        Self { size: 1.0, density: 1.0, variation: 0.5, at: centre(), expansion: 0.0, blur: 0.0, contrast: 0.0 }
    }
}

/// What a generator makes: the colour layer and, for lipstick, a roughness layer
/// (R = roughness, A = opacity) for its finish.
pub struct Generated {
    pub color: LayerImage,
    pub roughness: Option<LayerImage>,
}

/// The head's named regions, in [`Masks`]' channels: (name, which map, channel).
pub const REGIONS: [(&str, usize, usize); 10] = [
    ("cheek", 0, 0),
    ("forehead", 0, 1),
    ("upper_lip", 0, 2),
    ("chin", 0, 3),
    ("nose", 1, 0),
    ("mouth", 1, 1),
    ("upper_lid", 1, 2),
    ("inner_lid", 1, 3),
    ("neck", 2, 0),
    ("ear", 2, 1),
];

impl Masks {
    /// A named region (see [`REGIONS`]), 0..1 per texel.
    pub fn region(&self, name: &str) -> Option<Vec<f32>> {
        let &(_, map, c) = REGIONS.iter().find(|r| r.0 == name)?;
        Some(channel([&self.cfulc, &self.nmuil, &self.en][map], c, self.w, self.h))
    }
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
fn speckle(w: u32, h: u32, r: f32, d: f32, var: f32, seed: u32) -> Vec<f32> {
    let cell = (1.0 / d.max(1e-6)).sqrt().max(2.0 * r + 1.0);
    let cells = ((w as f32 / cell).ceil() as u32, (h as f32 / cell).ceil() as u32);
    let mut out = vec![0.0f32; (w * h) as usize];
    for cy in 0..cells.1 {
        for cx in 0..cells.0 {
            let (px, py) = ((cx as f32 + hash(cx, cy, seed)) * cell, (cy as f32 + hash(cy, cx, seed ^ 0x9e37)) * cell);
            let v = var.clamp(0.0, 1.0);
            let rr = r * (1.0 - 0.8 * v + 1.6 * v * hash(cx ^ 0x55, cy, seed));
            let k = 1.0 - v + v * hash(cx, cy ^ 0x77, seed);
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

/// `a` moved down by `dy` texels (negative: up), edges zero.
fn shift_y(a: &[f32], w: u32, h: u32, dy: i32) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        let src = y as i64 - dy as i64;
        if src >= 0 && (src as usize) < h {
            out[y * w..(y + 1) * w].copy_from_slice(&a[src as usize * w..(src as usize + 1) * w]);
        }
    }
    out
}

/// Each half of `a` moved away from the face's centre line (u = 0.5) by `dx`
/// texels (negative: towards it), edges zero.
fn shift_out(a: &[f32], w: u32, h: u32, dx: i32) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let half = w / 2;
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let (src, ok) = if x < half {
                let s = x as i64 + dx as i64;
                (s, s >= 0 && (s as usize) < half)
            } else {
                let s = x as i64 - dx as i64;
                (s, (s as usize) >= half && (s as usize) < w && s >= 0)
            };
            if ok {
                out[y * w + x] = a[y * w + src as usize];
            }
        }
    }
    out
}

/// A feature of a region mask channel, `<channel>.<op>` (radii and moves in
/// texels at 1024): `b3` / `b6` / `b20` blurred; `out` the band around the region;
/// `in` its rim inside; `dy<n>` moved down n (up when negative), `dx<n>` moved
/// away from the face's centre line n (towards it when negative), blurred 10, or
/// with an `s` suffix blurred 4.
fn feature(chs: &[(&str, &[f32])], w: u32, h: u32, name: &str) -> Vec<f32> {
    let (ch, op) = name.split_once('.').expect("feature: <channel>.<op>");
    let m = chs.iter().find(|c| c.0 == ch).unwrap_or_else(|| panic!("feature: no channel {ch}")).1;
    let s = w as f32 / 1024.0;
    let b = |a: &[f32], r: f32| blur(a, w, h, (r * s).round() as u32, 3);
    match op {
        "b3" => b(m, 3.0),
        "b6" => b(m, 6.0),
        "b20" => b(m, 20.0),
        "out" => b(m, 20.0).iter().zip(m).map(|(b, m)| (b - m).clamp(0.0, 1.0)).collect(),
        "in" => m.iter().zip(&b(m, 20.0)).map(|(m, b)| (m - b).clamp(0.0, 1.0) * 2.0).collect(),
        _ => {
            let (op, r) = match op.strip_suffix('s') {
                Some(o) => (o, 4.0),
                None => (op, 10.0),
            };
            let n = |t: &str| (t.parse::<f32>().unwrap_or_else(|_| panic!("feature: {name}")) * s).round() as i32;
            if let Some(t) = op.strip_prefix("dy") {
                b(&shift_y(m, w, h, n(t)), r)
            } else if let Some(t) = op.strip_prefix("dx") {
                b(&shift_out(m, w, h, n(t)), r)
            } else {
                panic!("feature: unknown op {name}")
            }
        }
    }
}

/// A fitted shape: the weighted sum of its features, 0..1.
fn fitted(chs: &[(&str, &[f32])], w: u32, h: u32, shape: &[(&str, f32)]) -> Vec<f32> {
    let mut out = vec![0.0f32; (w * h) as usize];
    for (name, k) in shape {
        for (o, v) in out.iter_mut().zip(feature(chs, w, h, name)) {
            *o += k * v;
        }
    }
    out.iter().map(|v| v.clamp(0.0, 1.0)).collect()
}

/// Shapes fitted to CC5's SkinGen presets (`gen_shape_fit.py`: the CC5 layer's
/// opacity as a weighted sum of [`feature`]s of the region masks; the preset in
/// the comment, soft IoU against it).
/// blush_round, IoU 0.591
const SHAPE_BLUSH: &[(&str, f32)] = &[("cheek.dx24", 0.2046), ("cheek.dy-48", 0.0401)];
/// eyeshadow_smoky, IoU 0.773
const SHAPE_EYESHADOW: &[(&str, f32)] = &[("inner_lid.out", 6.0932), ("upper_lid.out", 5.4479), ("upper_lid.dx24", 5.2087), ("inner_lid.dx-24", 3.4349), ("upper_lid.dy-24", 2.0685), ("inner_lid.dy12s", 1.5726), ("upper_lid.dx-24", 1.4144), ("inner_lid.dy-24", 1.0936), ("upper_lid.dx-12s", 0.9085), ("inner_lid.dx-12s", 0.7209), ("inner_lid.in", 0.0753), ("forehead.out", 0.0692), ("forehead.dy48", 0.0027)];
/// contour_basic, IoU 0.475
const SHAPE_CONTOUR: &[(&str, f32)] = &[("inner_lid.dy-24", 0.9212), ("forehead.out", 0.406), ("upper_lid.dx-24", 0.3988), ("upper_lid.dx24", 0.312), ("cheek.out", 0.2517), ("ear.out", 0.1833), ("mouth.dy-48", 0.1241), ("neck.dy-48", 0.0963), ("forehead.in", 0.0953), ("nose.dx12s", 0.0866), ("chin.dy-48", 0.0856), ("cheek.dx24", 0.0849), ("upper_lid.dy-12s", 0.0783), ("chin.dy48", 0.0586), ("forehead.dx24", 0.0526), ("neck.in", 0.0489), ("ear.dx-24", 0.0356), ("neck.out", 0.033), ("cheek.dy48", 0.0321), ("nose.dx24", 0.0309), ("nose.out", 0.0174), ("neck.dx-24", 0.0073)];
/// highlight_basic, IoU 0.276
const SHAPE_HIGHLIGHT: &[(&str, f32)] = &[("upper_lid.dy-48", 0.301), ("inner_lid.dy48", 0.2626), ("upper_lid.dy48", 0.1033), ("chin.dy-12s", 0.0807), ("nose.dx-24", 0.0783), ("nose.dy-48", 0.0288), ("upper_lid.dy-24", 0.0263), ("forehead.dy48", 0.0225), ("cheek.dy-24", 0.0151), ("chin.dy-24", 0.0081), ("forehead.in", 0.0057), ("nose.dx-12s", 0.0051), ("cheek.dx24", 0.0028), ("cheek.dx-12s", 0.0024), ("forehead.dx-24", 0.0017)];
/// freckle_01, IoU 0.682
const SHAPE_FRECKLES: &[(&str, f32)] = &[("inner_lid.dy48", 10.5913), ("inner_lid.dy24", 2.0273), ("inner_lid.dy12", 1.1606), ("nose.dy-24", 0.3314), ("chin.dx-24", 0.3044), ("mouth.dy48", 0.2687), ("nose.dx24", 0.2651), ("cheek.dx-24", 0.2373), ("nose.dx-12s", 0.2324), ("chin.dx24", 0.2157), ("chin.dy-12s", 0.1055), ("upper_lip.dy-12", 0.0989), ("upper_lip.dy-12s", 0.0627), ("chin.dy-12", 0.0534), ("chin.b3", 0.0278), ("upper_lid.dy-48", 0.0263), ("chin.dy12s", 0.0255), ("inner_lid.dx-12s", 0.0153), ("inner_lid.dy-12s", 0.0136), ("cheek.dy-48", 0.0101), ("mouth.dy-12s", 0.0096), ("inner_lid.dy-24", 0.0087), ("forehead.dx-24", 0.002)];
/// beard_stubble, IoU 0.545
const SHAPE_STUBBLE: &[(&str, f32)] = &[("chin.dx-24", 2.6286), ("chin.dx24", 1.179), ("cheek.out", 0.987), ("cheek.dy48", 0.6643), ("chin.dy48", 0.6328), ("neck.out", 0.4581), ("mouth.dy48", 0.4293), ("chin.out", 0.4202), ("neck.dy-48", 0.3845), ("upper_lip.dy48", 0.2932), ("upper_lip.dx-24", 0.2465), ("mouth.dy-24", 0.0943), ("mouth.out", 0.081), ("upper_lip.dx-12s", 0.0632), ("mouth.dy-48", 0.0365), ("chin.dx-12s", 0.0304), ("ear.dy-48", 0.0017)];

/// SkinGen's Range controls on a shape (0..1): blur feathers it (radius up to 24
/// texels at 1024), expansion grows (> 0) or shrinks (< 0) it (a power curve),
/// contrast hardens (> 0) or softens (< 0) its edge.
fn range(a: Vec<f32>, w: u32, h: u32, p: &Pattern) -> Vec<f32> {
    let s = w as f32 / 1024.0;
    let a = if p.blur > 0.0 { blur(&a, w, h, (p.blur.min(1.0) * 24.0 * s).round().max(1.0) as u32, 2) } else { a };
    if p.expansion == 0.0 && p.contrast == 0.0 {
        return a;
    }
    let g = 2f32.powf(-2.0 * p.expansion.clamp(-1.0, 1.0));
    let k = if p.contrast >= 0.0 { 1.0 + 4.0 * p.contrast } else { 1.0 + p.contrast.max(-0.9) };
    a.into_iter()
        .map(|v| {
            let v = v.clamp(0.0, 1.0).powf(g);
            if p.contrast == 0.0 {
                v
            } else {
                ((v - 0.5) * k + 0.5).clamp(0.0, 1.0)
            }
        })
        .collect()
}

/// A roughness layer for `color`'s opacity at `value` (0..1): R = roughness, A = opacity.
pub fn roughness_layer(color: &LayerImage, value: f32) -> LayerImage {
    let r = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgba = color.rgba.chunks(4).flat_map(|p| [r, r, r, p[3]]).collect();
    LayerImage { rgba, blend: crate::Blend::Normal, parts: Vec::new(), ..color.clone() }
}

/// The generator `kind` at colour `rgb`; `seed` varies freckles and stubble.
pub fn generate(m: &Masks, kind: Kind, rgb: [u8; 3], seed: u32, pattern: Pattern) -> Generated {
    let (psize, pdens, pvar) = (pattern.size.max(0.05), pattern.density.max(0.01), pattern.variation);
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
    let inner_lid = channel(&m.nmuil, 3, w, h);
    let ear = channel(&m.en, 1, w, h);
    let chs: [(&str, &[f32]); 10] = [
        ("cheek", &cheek),
        ("forehead", &fore),
        ("upper_lip", &upper_lip),
        ("chin", &chin),
        ("nose", &nose),
        ("mouth", &mouth),
        ("upper_lid", &lid),
        ("inner_lid", &inner_lid),
        ("neck", &neck),
        ("ear", &ear),
    ];
    let alpha: Vec<f32> = match kind {
        Kind::Lipstick => blur(&mouth, w, h, (1.5 * s).max(1.0) as u32, 1),
        Kind::Blush => fitted(&chs, w, h, SHAPE_BLUSH),
        Kind::Eyeshadow => fitted(&chs, w, h, SHAPE_EYESHADOW),
        Kind::Contour => fitted(&chs, w, h, SHAPE_CONTOUR),
        Kind::Highlight => fitted(&chs, w, h, SHAPE_HIGHLIGHT),
        Kind::Freckles => {
            let region = range(fitted(&chs, w, h, SHAPE_FRECKLES), w, h, &pattern);
            let dots = speckle(w, h, 1.1 * s * psize, 0.006 * pdens / (s * s), pvar, seed);
            region.iter().zip(&dots).map(|(r, d)| r * d).collect()
        }
        Kind::Spot => {
            // a soft round spot, radius 1.2 texels at 1024 (a small mole) x size
            let (cx, cy) = (pattern.at[0] * w as f32, pattern.at[1] * h as f32);
            let r = 1.2 * s * psize;
            (0..(w * h) as usize)
                .map(|i| {
                    let (x, y) = ((i % w as usize) as f32 + 0.5, (i / w as usize) as f32 + 0.5);
                    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                    ((r - d) / (0.6 * s.max(1.0)) + 0.5).clamp(0.0, 1.0)
                })
                .collect()
        }
        Kind::Stubble => {
            let region = range(fitted(&chs, w, h, SHAPE_STUBBLE), w, h, &pattern);
            let dots = speckle(w, h, 0.55 * s * psize, 0.09 * pdens / (s * s), pvar, seed);
            region.iter().zip(&dots).map(|(r, d)| r * d).collect()
        }
    };

    let alpha = if matches!(kind, Kind::Freckles | Kind::Stubble) { alpha } else { range(alpha, w, h, &pattern) };
    let rgba: Vec<u8> = alpha.iter().flat_map(|a| [rgb[0], rgb[1], rgb[2], (a.clamp(0.0, 1.0) * 255.0).round() as u8]).collect();
    let color = LayerImage { rect: [0, 0, w, h], size: w, w, h, rgba, blend: kind.default_blend(), parts: Vec::new() };
    // lipstick: a satin finish (rougher than bare lips' wet sheen is CC5's matte look)
    let roughness = kind.default_roughness().map(|r| roughness_layer(&color, r));
    Generated { color, roughness }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifts_move_away_from_the_centre_line_and_down() {
        // one texel on each side of an 8 x 2 map's centre line
        let mut a = vec![0.0f32; 16];
        (a[3], a[4]) = (1.0, 1.0);
        let o = shift_out(&a, 8, 2, 2);
        assert_eq!((o[1], o[6], o[3], o[4]), (1.0, 1.0, 0.0, 0.0));
        let d = shift_y(&a, 8, 2, 1);
        assert_eq!((d[8 + 3], d[3]), (1.0, 0.0));
    }

    #[test]
    fn range_grows_shrinks_and_hardens() {
        let a: Vec<f32> = (0..16).map(|i| i as f32 / 15.0).collect();
        let p = |e: f32, c: f32| Pattern { expansion: e, contrast: c, ..Pattern::default() };
        let sum = |v: &[f32]| v.iter().sum::<f32>();
        assert_eq!(range(a.clone(), 16, 1, &Pattern::default()), a);
        assert!(sum(&range(a.clone(), 16, 1, &p(0.5, 0.0))) > sum(&a), "expansion grows");
        assert!(sum(&range(a.clone(), 16, 1, &p(-0.5, 0.0))) < sum(&a), "and shrinks");
        let hard = range(a.clone(), 16, 1, &p(0.0, 1.0));
        assert!(hard[2] < a[2] && hard[13] > a[13], "contrast hardens the edge");
    }

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
        let g = generate(&m, Kind::Lipstick, [200, 0, 0], 1, Pattern::default());
        let a = |x: u32, y: u32| g.color.rgba[((y * w + x) * 4 + 3) as usize];
        assert!(a(32, 32) > 200 && a(5, 5) == 0);
        assert!(g.roughness.is_some());
        let f = generate(&m, Kind::Freckles, [100, 50, 30], 1, Pattern::default());
        let fa = |x: u32, y: u32| f.color.rgba[((y * w + x) * 4 + 3) as usize];
        assert!((0..8).all(|y| (0..8).all(|x| fa(x, y) == 0)), "no freckles far from the regions");
    }

    #[test]
    fn blur_keeps_flat_fields_flat() {
        let a = vec![0.5f32; 32 * 32];
        let b = blur(&a, 32, 32, 3, 2);
        assert!(b.iter().all(|v| (v - 0.5).abs() < 1e-4));
    }
}
