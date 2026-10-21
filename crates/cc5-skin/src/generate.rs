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
            Kind::Stubble => ([105, 92, 80], 0.7),
            Kind::Spot => ([120, 76, 52], 0.9),
        }
    }

    /// Its starting pattern: stubble denser than freckles' default.
    pub fn default_pattern(self) -> Pattern {
        match self {
            // CC5's beard_stubble: fine, dense, pale hairs (with the colour
            // [105, 92, 80], change error 0.88 -> 0.67, gen_sweep.py)
            Kind::Stubble => Pattern { density: 3.0, size: 0.7, ..Pattern::default() },
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
    // "g<u>,<v>,<sigma>": a Gaussian blob at (u, v) / 64 of the texture, sigma in
    // texels at 1024, mirrored across the face's centre line (u = 0.5)
    if let Some(g) = name.strip_prefix('g') {
        let p: Vec<f32> = g.split(',').map(|x| x.parse().unwrap_or_else(|_| panic!("feature: {name}"))).collect();
        let (u, v, st) = (p[0] / 64.0, p[1] / 64.0, p[2] / 1024.0);
        let k = -0.5 / (st * st);
        let mut out = vec![0.0f32; (w * h) as usize];
        for y in 0..h {
            let dy = (y as f32 + 0.5) / h as f32 - v;
            for x in 0..w {
                let px = (x as f32 + 0.5) / w as f32;
                let mut g = ((px - u).powi(2) + dy * dy) * k;
                g = g.exp();
                if p[0] != 32.0 {
                    g += (((px - (1.0 - u)).powi(2) + dy * dy) * k).exp();
                }
                out[(y * w + x) as usize] = g;
            }
        }
        return out;
    }
    let (ch, op) = name.split_once('.').expect("feature: <channel>.<op>");
    let get = |ch: &str| chs.iter().find(|c| c.0 == ch).unwrap_or_else(|| panic!("feature: no channel {ch}")).1;
    let s = w as f32 / 1024.0;
    let b = |a: &[f32], r: f32| blur(a, w, h, (r * s).round() as u32, 3);
    // "a*b.p<r>": the border of two regions, both blurred by r, 1 where each is 0.5
    if let Some((ca, cb)) = ch.split_once('*') {
        let r: f32 = op.strip_prefix('p').and_then(|r| r.parse().ok()).unwrap_or_else(|| panic!("feature: {name}"));
        return b(get(ca), r).iter().zip(b(get(cb), r)).map(|(x, y)| (4.0 * x * y).clamp(0.0, 1.0)).collect();
    }
    let m = get(ch);
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
/// the comment, soft IoU against it). Per kind the fit that renders closest to
/// CC5 (gen_validate.py change error): contour, highlight and stubble use the
/// mirrored UV blobs ("g<u>,<v>,<sigma>"), blush and freckles the region-border
/// features ("a*b.p<r>"), eyeshadow the region features alone.
/// blush_round, IoU 0.601
const SHAPE_BLUSH: &[(&str, f32)] = &[("inner_lid*ear.p24", 27547131013629.965), ("inner_lid*neck.p24", 1843519202873.1365), ("upper_lid*neck.p24", 13787982385.9988), ("cheek*ear.p24", 11.7386), ("cheek.dx24", 0.1947), ("cheek.dy-48", 0.0434), ("cheek.dy-24", 0.002)];
/// eyeshadow_smoky, IoU 0.773
const SHAPE_EYESHADOW: &[(&str, f32)] = &[("inner_lid.out", 6.0932), ("upper_lid.out", 5.4479), ("upper_lid.dx24", 5.2087), ("inner_lid.dx-24", 3.4349), ("upper_lid.dy-24", 2.0685), ("inner_lid.dy12s", 1.5726), ("upper_lid.dx-24", 1.4144), ("inner_lid.dy-24", 1.0936), ("upper_lid.dx-12s", 0.9085), ("inner_lid.dx-12s", 0.7209), ("inner_lid.in", 0.0753), ("forehead.out", 0.0692), ("forehead.dy48", 0.0027)];
/// contour_basic, IoU 0.876
const SHAPE_CONTOUR: &[(&str, f32)] = &[("inner_lid*ear.p24", 657890830992.3206), ("forehead*ear.p10", 43266030140.677), ("inner_lid*neck.p24", 43237318235.3565), ("upper_lid*neck.p24", 1364292677.0043), ("upper_lid*ear.p24", 21256457.6875), ("nose*neck.p24", 540604.4567), ("forehead*inner_lid.p10", 387.6922), ("forehead*upper_lip.p24", 376.1623), ("forehead*upper_lid.p10", 94.7765), ("nose*upper_lid.p10", 35.1827), ("forehead*ear.p24", 22.521), ("chin*mouth.p10", 13.8989), ("inner_lid.dy-24", 1.1349), ("nose*upper_lid.p24", 1.0591), ("cheek*forehead.p24", 0.8236), ("chin*neck.p10", 0.5563), ("nose*inner_lid.p24", 0.3129), ("upper_lip*nose.p10", 0.22), ("upper_lid.dx24", 0.1763), ("upper_lid.dx-24", 0.13), ("g42,30,24", 0.1259), ("g42,38,24", 0.115), ("g40,32,24", 0.1042), ("g46,26,24", 0.1038), ("g44,36,24", 0.1002), ("g44,22,24", 0.0993), ("g46,24,24", 0.0952), ("g40,40,24", 0.0947), ("g46,30,24", 0.094), ("g46,18,24", 0.0927), ("g44,28,24", 0.0889), ("cheek*nose.p10", 0.0876), ("chin.dy48", 0.0871), ("g44,40,24", 0.0856), ("g34,42,24", 0.082), ("nose.dx12s", 0.0819), ("g46,16,24", 0.0781), ("g42,42,24", 0.0764), ("mouth.dy-48", 0.0755), ("g32,44,24", 0.0746), ("g46,34,24", 0.0726), ("g44,20,24", 0.0716), ("g42,14,24", 0.0705), ("g44,14,24", 0.0693), ("g50,34,24", 0.0655), ("g42,16,24", 0.0623), ("g46,28,24", 0.0612), ("g36,42,24", 0.061), ("g40,14,24", 0.0592), ("g44,32,24", 0.0572), ("g48,32,24", 0.0562), ("g46,38,24", 0.0562), ("g38,40,24", 0.0552), ("g40,12,24", 0.0545), ("g48,36,24", 0.053), ("g38,12,24", 0.0515), ("g40,34,24", 0.0512), ("chin*mouth.p24", 0.0485), ("g50,32,24", 0.0484), ("g48,20,24", 0.0478), ("g42,18,24", 0.0474), ("g38,44,24", 0.0459), ("g38,42,24", 0.0441), ("g44,18,24", 0.0431), ("g42,32,24", 0.042), ("g36,40,24", 0.0403), ("g42,22,24", 0.04), ("g40,42,24", 0.0363), ("g48,38,24", 0.0355), ("g46,32,24", 0.0347), ("g34,40,24", 0.0328), ("g50,36,24", 0.031), ("g34,44,24", 0.031), ("g36,44,24", 0.0301), ("g46,40,24", 0.0295), ("chin.dy-48", 0.029), ("g48,34,24", 0.0262), ("nose.dx24", 0.0256), ("g44,16,24", 0.0254), ("g34,22,24", 0.0234), ("g46,36,24", 0.0224), ("g34,46,24", 0.0211), ("g42,28,24", 0.0195), ("g48,22,24", 0.0194), ("g44,34,24", 0.018), ("g52,34,24", 0.0172), ("ear.out", 0.0171), ("g40,44,24", 0.0163), ("g48,24,24", 0.0137), ("g52,32,24", 0.0133), ("g46,20,24", 0.0132), ("chin.dy-24", 0.013), ("g40,22,24", 0.0129), ("g38,34,24", 0.0106), ("g32,42,24", 0.0084), ("g42,12,24", 0.0044), ("g36,46,24", 0.004)];
/// highlight_basic, IoU 0.559
const SHAPE_HIGHLIGHT: &[(&str, f32)] = &[("forehead*mouth.p24", 655421.9837), ("forehead*upper_lid.p10", 0.5964), ("forehead*nose.p10", 0.2796), ("inner_lid.dy-48", 0.1145), ("g34,18,24", 0.0887), ("chin.dy-12s", 0.0813), ("nose.dx-24", 0.0649), ("g32,18,24", 0.0581), ("g38,26,24", 0.0459), ("g38,20,24", 0.0458), ("g32,24,24", 0.0362), ("g36,18,24", 0.032), ("g40,26,24", 0.025), ("upper_lid.dy-24", 0.018), ("g32,16,24", 0.0087), ("chin.dy-24", 0.0082), ("g38,28,24", 0.0079), ("g32,26,24", 0.0051), ("g40,20,24", 0.0045), ("chin.dx-24", 0.0027), ("forehead.out", 0.0023)];
/// freckle_01, IoU 0.721
const SHAPE_FRECKLES: &[(&str, f32)] = &[("forehead*mouth.p24", 3090685.8117), ("upper_lid*ear.p24", 1992205.6961), ("cheek*chin.p10", 678118.019), ("upper_lip*chin.p10", 207565.1497), ("forehead*upper_lip.p24", 14794.6705), ("mouth*ear.p24", 3485.3992), ("forehead*neck.p24", 78.7555), ("cheek*inner_lid.p10", 23.8259), ("nose*upper_lid.p10", 22.5836), ("inner_lid.dy48", 8.5088), ("cheek*chin.p24", 3.0278), ("cheek*nose.p24", 1.1413), ("chin.dx-24", 0.33), ("cheek*upper_lip.p24", 0.283), ("chin.dy-24", 0.1828), ("nose.dx-12s", 0.1827), ("mouth.dy48", 0.1606), ("nose.dy-48", 0.1565), ("chin.dx24", 0.1564), ("cheek.dx-24", 0.1147), ("nose.dy-24", 0.0583), ("cheek.dy-48", 0.0532), ("chin.dy12s", 0.0402), ("chin.b3", 0.0361), ("upper_lid.dy-48", 0.0332), ("upper_lip.dx-12s", 0.0318), ("nose.dy12s", 0.0275), ("upper_lip.dy-12s", 0.0213), ("chin.dx12s", 0.0123), ("mouth.dy-24", 0.006), ("chin.in", 0.0021)];
/// beard_stubble, IoU 0.925
const SHAPE_STUBBLE: &[(&str, f32)] = &[("inner_lid*ear.p24", 5479545913392.647), ("cheek*chin.p10", 3732604.0488), ("upper_lip*chin.p10", 1373823.9532), ("upper_lip*neck.p24", 78.9638), ("chin*ear.p10", 27.582), ("cheek*neck.p24", 8.4685), ("chin.b20", 1.0691), ("chin*mouth.p24", 0.6941), ("g36,38,24", 0.3557), ("g36,36,24", 0.35), ("g34,40,24", 0.3301), ("g40,38,24", 0.3281), ("g42,36,24", 0.2756), ("g40,34,24", 0.2745), ("g42,32,24", 0.2682), ("g40,32,24", 0.2561), ("g38,36,24", 0.2445), ("g36,42,24", 0.243), ("g44,34,24", 0.2408), ("g38,32,24", 0.2391), ("g38,34,24", 0.2383), ("g38,40,24", 0.2298), ("g38,38,24", 0.2277), ("g44,28,24", 0.2194), ("g44,36,24", 0.2128), ("g42,38,24", 0.2046), ("g34,42,24", 0.2033), ("g32,42,24", 0.1999), ("g44,38,24", 0.1926), ("g36,40,24", 0.1891), ("g44,30,24", 0.1879), ("g46,32,24", 0.1738), ("g34,44,24", 0.1707), ("g42,30,24", 0.1702), ("g42,40,24", 0.1669), ("g34,38,24", 0.1622), ("g46,30,24", 0.1619), ("nose*mouth.p24", 0.161), ("upper_lip.dx24", 0.1544), ("g40,42,24", 0.1497), ("g36,44,24", 0.1467), ("g38,42,24", 0.1437), ("g46,28,24", 0.143), ("g44,32,24", 0.137), ("g36,32,24", 0.1362), ("g32,44,24", 0.136), ("chin.dx24", 0.1311), ("g40,36,24", 0.1279), ("g40,40,24", 0.1227), ("g32,30,24", 0.1219), ("g38,44,24", 0.1196), ("g46,34,24", 0.1132), ("mouth.dy48", 0.1122), ("g32,46,24", 0.1085), ("upper_lip.dy12", 0.106), ("chin.dy48", 0.0996), ("mouth.dy-24", 0.0853), ("g46,36,24", 0.085), ("g34,46,24", 0.0838), ("g32,40,24", 0.0639), ("upper_lip.dx-24", 0.0603), ("upper_lip.dy-12s", 0.0582), ("g46,26,24", 0.0574), ("g44,40,24", 0.0562), ("upper_lip.b6", 0.0457), ("g36,46,24", 0.0408), ("chin.dy-24", 0.0391), ("cheek.dy48", 0.0378), ("g42,42,24", 0.0302), ("g42,34,24", 0.029), ("chin.b3", 0.0287), ("g44,26,24", 0.0262), ("chin.dy-48", 0.0252), ("chin.dy12s", 0.0247), ("chin.dy24", 0.0242), ("mouth.dy-48", 0.0222), ("g38,30,24", 0.0215), ("g46,24,24", 0.021), ("chin.dy12", 0.02), ("chin.dy-12s", 0.0199), ("g38,46,24", 0.0192), ("g40,44,24", 0.0186), ("g34,36,24", 0.015), ("g34,48,24", 0.0143), ("g32,50,24", 0.0117), ("chin.dx-24", 0.01), ("g40,30,24", 0.0087), ("cheek.out", 0.0087), ("g46,38,24", 0.0085), ("upper_lip*nose.p10", 0.007), ("upper_lip.dy-12", 0.0069), ("g34,50,24", 0.0069), ("mouth.out", 0.0062), ("g32,48,24", 0.0061), ("chin.in", 0.0042), ("g44,24,24", 0.0039), ("upper_lip.dx-12s", 0.0037), ("g40,46,24", 0.0037), ("g48,30,24", 0.0036), ("g36,50,24", 0.0036), ("g42,28,24", 0.0035), ("neck.out", 0.0021), ("g44,42,24", 0.0016)];

/// Hairs: short soft strokes of half-width `r` and length `len` (texels) at
/// density `d` (per texel), pointing down the texture (hair grows down the face)
/// within ±35°, varying in length and darkness by `var`.
fn strokes(w: u32, h: u32, r: f32, len: f32, d: f32, var: f32, seed: u32) -> Vec<f32> {
    let cell = (1.0 / d.max(1e-6)).sqrt().max(2.0 * r + 1.0);
    let cells = ((w as f32 / cell).ceil() as u32, (h as f32 / cell).ceil() as u32);
    let mut out = vec![0.0f32; (w * h) as usize];
    let v = var.clamp(0.0, 1.0);
    for cy in 0..cells.1 {
        for cx in 0..cells.0 {
            let (px, py) = ((cx as f32 + hash(cx, cy, seed)) * cell, (cy as f32 + hash(cy, cx, seed ^ 0x9e37)) * cell);
            let l = len * (1.0 - 0.6 * v + 1.2 * v * hash(cx ^ 0x55, cy, seed));
            let k = 1.0 - v + v * hash(cx, cy ^ 0x77, seed);
            let a = (hash(cx ^ 0x33, cy ^ 0x11, seed) - 0.5) * 1.22; // ±35°
            let (dx, dy) = (a.sin(), a.cos());
            let (qx, qy) = (px + dx * l, py + dy * l);
            let (x0, x1) = ((px.min(qx) - r - 1.0).max(0.0) as u32, ((px.max(qx) + r + 1.0) as u32).min(w - 1));
            let (y0, y1) = ((py.min(qy) - r - 1.0).max(0.0) as u32, ((py.max(qy) + r + 1.0) as u32).min(h - 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    // distance to the segment
                    let (ex, ey) = (x as f32 - px, y as f32 - py);
                    let t = ((ex * dx + ey * dy) / l.max(1e-3)).clamp(0.0, 1.0) * l;
                    let dd = ((ex - dx * t).powi(2) + (ey - dy * t).powi(2)).sqrt();
                    let val = ((r - dd) / 0.8 + 0.5).clamp(0.0, 1.0) * k;
                    let o = &mut out[(y * w + x) as usize];
                    *o = o.max(val);
                }
            }
        }
    }
    out
}

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

/// A layer of `rgb` with `layer`'s opacity (and place): e.g. a layer's glow.
pub fn colour_layer(layer: &LayerImage, rgb: [u8; 3]) -> LayerImage {
    let rgba = layer.rgba.chunks(4).flat_map(|p| [rgb[0], rgb[1], rgb[2], p[3]]).collect();
    LayerImage { rgba, blend: crate::Blend::Normal, parts: Vec::new(), ..layer.clone() }
}

/// A roughness layer for `color`'s opacity at `value` (0..1): R = roughness, A = opacity.
/// (Any value: metallic, occlusion as well.)
pub fn roughness_layer(color: &LayerImage, value: f32) -> LayerImage {
    let r = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgba = color.rgba.chunks(4).flat_map(|p| [r, r, r, p[3]]).collect();
    LayerImage { rgba, blend: crate::Blend::Normal, parts: Vec::new(), ..color.clone() }
}

/// Stubble's haze between the hairs (opacity at strength 1, CC5's ~0.1-0.2).
const STUBBLE_HAZE: f32 = 0.3;

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
    // stubble's pale haze under its hairs
    let mut haze: Option<Vec<f32>> = None;
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
            // as CC5's stubble: a pale haze over the beard area (shaved hair under
            // the skin) with the darker hairs on it (its layer: opacity ~0.1 between
            // hairs, ~0.45 on them)
            let region = range(fitted(&chs, w, h, SHAPE_STUBBLE), w, h, &pattern);
            let hairs = strokes(w, h, 0.45 * s * psize, 3.0 * s * psize, 0.03 * pdens / (s * s), pvar, seed);
            haze = Some(region.iter().map(|r| r * STUBBLE_HAZE).collect());
            region.iter().zip(&hairs).map(|(r, d)| r * d).collect()
        }
    };

    let alpha = if matches!(kind, Kind::Freckles | Kind::Stubble) { alpha } else { range(alpha, w, h, &pattern) };
    let layer = |a: &[f32], c: [u8; 3]| {
        let rgba: Vec<u8> = a.iter().flat_map(|a| [c[0], c[1], c[2], (a.clamp(0.0, 1.0) * 255.0).round() as u8]).collect();
        LayerImage { rect: [0, 0, w, h], size: w, w, h, rgba, blend: kind.default_blend(), parts: Vec::new() }
    };
    let color = match &haze {
        // the haze (the hair colour washed towards pale grey-beige), the hairs over it
        Some(hz) => {
            let pale = [0, 1, 2].map(|c| (rgb[c] as f32 * 0.35 + [176.0, 164.0, 150.0][c] * 0.65).round() as u8);
            LayerImage { parts: vec![layer(&alpha, rgb)], ..layer(hz, pale) }
        }
        None => layer(&alpha, rgb),
    };
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
