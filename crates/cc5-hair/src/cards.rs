//! CC5 hair cards -> strands (hair_build.py `cards_to_strands`, `_patch_strands`).
//!
//! Per card (connected component of the hair material's triangles): strands at
//! texture columns that hold hair, from the root end (the card end nearer the
//! head) to where the texture's strands end, coloured from the texture, gathered
//! into clumps, spread a little off the card plane. UVs are glTF's (v down) with
//! images row 0 at the top: the same texels Blender's UVs address.

use glam::{Vec2, Vec3};
use rayon::prelude::*;

use crate::head::{Body, Surface};
use crate::settings::Convert;
use crate::{srgb_to_linear, uniform_polyline, Rng, Strand};

/// An image sampled as 0..1 floats (colour as stored, i.e. sRGB-encoded;
/// alpha straight).
pub struct Tex {
    pub w: usize,
    pub h: usize,
    pub img: std::sync::Arc<image::RgbaImage>,
}

impl Tex {
    pub fn new(img: std::sync::Arc<image::RgbaImage>) -> Self {
        Self { w: img.width() as usize, h: img.height() as usize, img }
    }

    /// Nearest texel at `uv` (wrapping).
    pub fn sample(&self, uv: Vec2) -> [f32; 4] {
        let x = ((uv.x.rem_euclid(1.0) * self.w as f32) as usize).min(self.w - 1);
        let y = ((uv.y.rem_euclid(1.0) * self.h as f32) as usize).min(self.h - 1);
        self.img.get_pixel(x as u32, y as u32).0.map(|c| c as f32 / 255.0)
    }
}

/// One card: its triangles' corners (positions and UVs, in the card's
/// (across, along) UV frame).
struct Card {
    cp: Vec<Vec3>,
    cu: Vec<Vec2>,
    swap: bool,
    area: f32,
}

impl Card {
    /// The 3D point at `uv` on the card, if a triangle covers it.
    fn point(&self, uv: Vec2) -> Option<Vec3> {
        for t in 0..self.cp.len() / 3 {
            let (a, b, c) = (self.cu[3 * t], self.cu[3 * t + 1], self.cu[3 * t + 2]);
            let (v0, v1, d) = (b - a, c - a, uv - a);
            let mut den = v0.x * v1.y - v1.x * v0.y;
            if den.abs() < 1e-12 {
                den = 1e-12;
            }
            let s = (d.x * v1.y - v1.x * d.y) / den;
            let tt = (v0.x * d.y - d.x * v0.y) / den;
            if s >= -1e-4 && tt >= -1e-4 && s + tt <= 1.0 + 1e-4 {
                let [p0, p1, p2] = [self.cp[3 * t], self.cp[3 * t + 1], self.cp[3 * t + 2]];
                return Some(p0 * (1.0 - s - tt) + p1 * s + p2 * tt);
            }
        }
        None
    }

    /// Texture UV of a point in the card's frame.
    fn tex_uv(&self, q: Vec2) -> Vec2 {
        if self.swap {
            Vec2::new(q.y, q.x)
        } else {
            q
        }
    }

    fn bounds(&self) -> (Vec2, Vec2) {
        self.cu.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), u| (lo.min(*u), hi.max(*u)))
    }
}

/// Split the hair material's triangles into cards (components over shared
/// vertices).
fn cards(pos: &[Vec3], uv: &[Vec2], tris: &[[u32; 3]]) -> Vec<Card> {
    let mut parent: Vec<u32> = (0..pos.len() as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    for t in tris {
        for (u, v) in [(t[0], t[1]), (t[1], t[2])] {
            let (ru, rv) = (find(&mut parent, u), find(&mut parent, v));
            if ru != rv {
                parent[ru as usize] = rv;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<u32, Vec<usize>> = Default::default();
    for (i, t) in tris.iter().enumerate() {
        let r = find(&mut parent, t[0]);
        groups.entry(r).or_default().push(i);
    }
    groups
        .into_values()
        .map(|ti| {
            let cp: Vec<Vec3> = ti.iter().flat_map(|&i| tris[i].map(|v| pos[v as usize])).collect();
            let mut cu: Vec<Vec2> = ti.iter().flat_map(|&i| tris[i].map(|v| uv[v as usize])).collect();
            let area: f32 = cp.chunks_exact(3).map(|c| (c[1] - c[0]).cross(c[2] - c[0]).length() / 2.0).sum();
            let (lo, hi) = cu.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), u| (lo.min(*u), hi.max(*u)));
            // strands run along the card's longer UV axis (most CC5 strips are
            // vertical; some short-hair cards lie sideways)
            let swap = (hi.x - lo.x) > 1.2 * (hi.y - lo.y);
            if swap {
                cu.iter_mut().for_each(|u| *u = Vec2::new(u.y, u.x));
            }
            Card { cp, cu, swap, area }
        })
        .collect()
}

/// Strands for one source layer (the hair material's triangles). Not pushed
/// off the head yet (see `shape::finish`).
pub fn cards_to_strands(
    pos: &[Vec3],
    uv: &[Vec2],
    tris: &[[u32; 3]],
    img: Option<&Tex>,
    flow: Option<&Tex>,
    conv: &Convert,
    body: &Body,
    seed: u64,
) -> Vec<Strand> {
    let cards = cards(pos, uv, tris);
    cards
        .par_iter()
        .enumerate()
        .flat_map_iter(|(ci, card)| {
            let mut rng = Rng::new(seed ^ (ci as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            card_strands(card, img, flow, conv, body, &mut rng)
        })
        .collect()
}

fn card_strands(card: &Card, img: Option<&Tex>, flow: Option<&Tex>, conv: &Convert, body: &Body, rng: &mut Rng) -> Vec<Strand> {
    let mut out = Vec::new();
    let (lo, hi) = card.bounds();
    let (umin, umax, vmin, vmax) = (lo.x, hi.x, lo.y, hi.y);
    if umax - umin < 1e-4 || vmax - vmin < 1e-4 {
        return out;
    }
    let np = conv.points.max(3);
    if ((vmax - vmin) as f64) < conv.patch_aspect * (umax - umin) as f64 {
        return patch_strands(card, img, flow, conv, rng, np);
    }
    // root end: the v end whose points are nearer the head
    let um = (umin + umax) / 2.0;
    let e0 = card.point(Vec2::new(um, vmin + 0.02 * (vmax - vmin)));
    let e1 = card.point(Vec2::new(um, vmax - 0.02 * (vmax - vmin)));
    let (d0, d1) = match (e0, e1) {
        (Some(a), Some(b)) => (body.nearest(a).dist, body.nearest(b).dist),
        _ => (0.0, 0.0),
    };
    let (v_root, v_tip) = if d0 <= d1 { (vmin, vmax) } else { (vmax, vmin) };
    let n_try = conv.min_tries.max((card.area as f64 * 1e4 * conv.strands_per_cm2 * 1.5) as usize);
    let vs: Vec<f32> = (0..32).map(|i| v_root + (v_tip - v_root) * i as f32 / 31.0).collect();
    let jit = conv.thickness;
    // clumps across the card: one tone and one offset off the card each
    let mut clumps: Vec<(f32, f64, f64)> = Vec::new();
    let mut spread = 0.0f64;
    if conv.clumps_per_cm > 0.0 {
        let vm = (vmin + vmax) / 2.0;
        let width_m = match (card.point(Vec2::new(umin + 0.02 * (umax - umin), vm)), card.point(Vec2::new(umax - 0.02 * (umax - umin), vm))) {
            (Some(a), Some(b)) => a.distance(b) as f64,
            _ => 0.01,
        };
        let n_c = ((width_m * 100.0 * conv.clumps_per_cm).round() as usize).max(1);
        for k in 0..n_c {
            let c = umin + ((k as f64 + 0.5 + rng.uniform(-0.3, 0.3)) * (umax - umin) as f64 / n_c as f64) as f32;
            clumps.push((c, rng.uniform(conv.clump_shade[0], conv.clump_shade[1]), rng.gauss(0.0, jit)));
        }
        spread = conv.clump_spread * (umax - umin) as f64 / n_c as f64;
    }
    let nrm = {
        let c = &card.cp;
        (c[1] - c[0]).cross(c[2] - c[0]).normalize_or_zero()
    };
    for _ in 0..n_try {
        let (u0, c_shade, c_off) = if clumps.is_empty() {
            (rng.uniform(umin as f64, umax as f64) as f32, 1.0, None)
        } else {
            let (c, s, o) = clumps[rng.below(clumps.len())];
            ((rng.gauss(c as f64, spread) as f32).clamp(umin, umax), s, Some(o))
        };
        let (t_start, t_end, rgb, opacity);
        if let Some(img) = img {
            let tex: Vec<[f32; 4]> = vs.iter().map(|&v| img.sample(card.tex_uv(Vec2::new(u0, v)))).collect();
            let a: Vec<f32> = tex.iter().map(|t| t[3]).collect();
            let amax = a.iter().cloned().fold(0.0f32, f32::max);
            let level = if conv.accept_on == "max" { amax } else { a.iter().sum::<f32>() / a.len() as f32 };
            if (level as f64) < rng.uniform(conv.accept[0], conv.accept[1]) {
                continue; // no strands in this column of the texture
            }
            // strand texels: relative to the column's peak (faint short-hair maps count too)
            let thr = 0.06f32.max(0.45 * amax);
            let hair: Vec<usize> = (0..a.len()).filter(|&i| a[i] > thr).collect();
            if hair.len() < 3 {
                continue;
            }
            let (first, last) = (hair[0], *hair.last().unwrap());
            t_end = (last + 1) as f32 / 32.0;
            // the strand starts where the texture's hair does (cards often begin transparent)
            let h0 = if conv.start_at_hair { first } else { 0 };
            t_start = h0 as f32 / 32.0;
            // colour from solid texels only: CC5 maps fade to pale colours where alpha is low
            let col_a = &a[h0..=last];
            let mut cols: Vec<[f32; 3]> = tex[h0..=last].iter().map(|t| [t[0], t[1], t[2]]).collect();
            let n = cols.len();
            let mut tt: Vec<f32> = (0..n).map(|i| if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 }).collect();
            if col_a.iter().filter(|&&x| x > 0.4).count() >= 2 {
                let keep: Vec<usize> = (0..n).filter(|&i| col_a[i] > 0.4).collect();
                cols = keep.iter().map(|&i| cols[i]).collect();
                tt = keep.iter().map(|&i| tt[i]).collect();
            }
            rgb = (0..np)
                .map(|i| {
                    let x = i as f32 / (np - 1) as f32;
                    Vec3::from(std::array::from_fn::<f32, 3, _>(|k| interp(x, &tt, &cols, k)))
                })
                .collect::<Vec<_>>();
            let solid: Vec<f32> = col_a.iter().cloned().filter(|&x| x > 0.05).collect();
            opacity = if solid.is_empty() { 1.0 } else { solid.iter().sum::<f32>() / solid.len() as f32 }.clamp(0.15, 1.0);
        } else {
            t_start = 0.0;
            t_end = 1.0;
            rgb = vec![Vec3::new(0.5, 0.4, 0.3); np];
            opacity = 1.0;
        }
        let pts: Vec<Vec3> = (0..np)
            .filter_map(|i| {
                let t = t_start + (t_end - t_start) * i as f32 / (np - 1) as f32;
                card.point(Vec2::new(u0, v_root + (v_tip - v_root) * t))
            })
            .collect();
        if pts.len() < 3 {
            continue;
        }
        // thickness: strands spread a little off the card plane
        let off = match c_off {
            None => nrm * rng.gauss(0.0, jit) as f32 + Vec3::from(std::array::from_fn(|_| rng.gauss(0.0, jit * 0.3) as f32)),
            Some(o) => nrm * (o + rng.gauss(0.0, jit * 0.15)) as f32 + Vec3::from(std::array::from_fn(|_| rng.gauss(0.0, jit * 0.1) as f32)),
        };
        let m = pts.len();
        let pts: Vec<Vec3> = pts.iter().enumerate().map(|(i, p)| *p + off * (0.3 + 0.7 * i as f32 / (m - 1) as f32)).collect();
        let pts = uniform_polyline(&pts, np);
        let width = rng.uniform(conv.width[0], conv.width[1]) * (opacity as f64).powf(conv.opacity_power);
        let shade = rng.uniform(conv.shade[0], conv.shade[1]) * c_shade;
        out.push(Strand {
            pts,
            id: rng.f() as f32,
            width: width as f32,
            shade: shade as f32,
            rgb: rgb.iter().map(|c| Vec3::new(srgb_to_linear(c.x), srgb_to_linear(c.y), srgb_to_linear(c.z))).collect(),
        });
    }
    out
}

/// Short strands on a "patch" card (short-hair texture painted over a wide
/// area): many strands where the texture has hair, each marching along the
/// card's flow map (CC5's: R = 0.5 + 0.5 du, G along the strand).
fn patch_strands(card: &Card, img: Option<&Tex>, flow: Option<&Tex>, conv: &Convert, rng: &mut Rng, np: usize) -> Vec<Strand> {
    let mut out = Vec::new();
    let (lo, hi) = card.bounds();
    let uv_area: f32 = card.cu.chunks_exact(3).map(|c| ((c[1].x - c[0].x) * (c[2].y - c[0].y) - (c[2].x - c[0].x) * (c[1].y - c[0].y)).abs() / 2.0).sum();
    if uv_area < 1e-9 {
        return out;
    }
    let scale = (card.area / uv_area).sqrt(); // metres per UV unit
    let n = (card.area as f64 * 1e4 * conv.patch_density) as usize;
    for _ in 0..n {
        let p0 = Vec2::new(rng.uniform(lo.x as f64, hi.x as f64) as f32, rng.uniform(lo.y as f64, hi.y as f64) as f32);
        if let Some(img) = img {
            if (img.sample(card.tex_uv(p0))[3] as f64) < rng.uniform(0.08, 0.5) {
                continue;
            }
        }
        let lk = (conv.patch_length * rng.uniform(0.5, 1.2)) as f32 / scale; // UV units
        let mut uv = vec![p0];
        for _ in 1..np {
            let q = *uv.last().unwrap();
            // CC5's flow map, in glTF UVs (v down): G encodes the direction down the image
            let mut d = match flow {
                Some(f) => {
                    let s = f.sample(card.tex_uv(q));
                    let d = Vec2::new(s[0] * 2.0 - 1.0, -(s[1] * 2.0 - 1.0));
                    if card.swap {
                        Vec2::new(d.y, d.x)
                    } else {
                        d
                    }
                }
                None => Vec2::new(0.0, 1.0),
            };
            d = if d.length() > 1e-3 { d.normalize() } else { Vec2::new(0.0, 1.0) };
            uv.push(q + d * lk / (np - 1) as f32);
        }
        // the strand ends where it leaves the card
        let pts: Vec<Vec3> = uv.iter().map_while(|q| card.point(*q)).collect();
        if pts.len() < 3 {
            continue;
        }
        let pts = uniform_polyline(&pts, np);
        let rgb: Vec<Vec3> = match img {
            Some(img) => {
                let tex: Vec<[f32; 3]> = uv.iter().map(|q| {
                    let t = img.sample(card.tex_uv(*q));
                    [t[0], t[1], t[2]]
                }).collect();
                let tt: Vec<f32> = (0..tex.len()).map(|i| i as f32 / (tex.len() - 1) as f32).collect();
                (0..np).map(|i| Vec3::from(std::array::from_fn::<f32, 3, _>(|k| interp(i as f32 / (np - 1) as f32, &tt, &tex, k)))).collect()
            }
            None => vec![Vec3::new(0.3, 0.25, 0.2); np],
        };
        out.push(Strand {
            pts,
            id: rng.f() as f32,
            width: rng.uniform(conv.width[0], conv.width[1]) as f32,
            shade: rng.uniform(conv.shade[0], conv.shade[1]) as f32,
            rgb: rgb.iter().map(|c| Vec3::new(srgb_to_linear(c.x), srgb_to_linear(c.y), srgb_to_linear(c.z))).collect(),
        });
    }
    out
}

/// `np.interp(x, xs, ys[:, k])` for increasing `xs`.
fn interp(x: f32, xs: &[f32], ys: &[[f32; 3]], k: usize) -> f32 {
    if xs.is_empty() {
        return 0.0;
    }
    if x <= xs[0] {
        return ys[0][k];
    }
    for i in 1..xs.len() {
        if x <= xs[i] {
            let f = if xs[i] > xs[i - 1] { (x - xs[i - 1]) / (xs[i] - xs[i - 1]) } else { 1.0 };
            return ys[i - 1][k] + (ys[i][k] - ys[i - 1][k]) * f;
        }
    }
    ys[ys.len() - 1][k]
}

/// The source's other material layers (scalp, caps): strands stay above them.
pub fn scalp_surface(pos: &[Vec3], tris: Vec<[u32; 3]>) -> Option<Surface> {
    Surface::new(pos.to_vec(), tris)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit quad card in the XY plane, UVs = positions.
    fn quad() -> Card {
        let p = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
        let tris = [[0, 1, 2], [0, 2, 3]];
        let cp = tris.iter().flat_map(|t| t.map(|i| p[i])).collect();
        let cu = tris.iter().flat_map(|t| t.map(|i| Vec2::new(p[i].x, p[i].y))).collect();
        Card { cp, cu, swap: false, area: 1.0 }
    }

    #[test]
    fn card_points_follow_uvs() {
        let c = quad();
        assert!(c.point(Vec2::new(0.25, 0.75)).unwrap().distance(Vec3::new(0.25, 0.75, 0.0)) < 1e-5);
        assert!(c.point(Vec2::new(0.9, 0.1)).unwrap().distance(Vec3::new(0.9, 0.1, 0.0)) < 1e-5);
        assert!(c.point(Vec2::new(1.5, 0.5)).is_none());
    }

    #[test]
    fn components_split_cards() {
        // two separate quads -> two cards; a wide one swaps its axes
        let pos = vec![
            Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y,
            Vec3::new(5.0, 0.0, 0.0), Vec3::new(7.0, 0.0, 0.0), Vec3::new(7.0, 0.5, 0.0), Vec3::new(5.0, 0.5, 0.0),
        ];
        let uv: Vec<Vec2> = pos.iter().map(|p| Vec2::new(p.x, p.y)).collect();
        let tris = vec![[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]];
        let cs = cards(&pos, &uv, &tris);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs.iter().filter(|c| c.swap).count(), 1);
        assert!((cs.iter().map(|c| c.area).sum::<f32>() - 2.0).abs() < 1e-5);
    }
}
