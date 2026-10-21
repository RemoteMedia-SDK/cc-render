//! Make meshes that were not captured from CC5 (hair cards, beards, brows from a
//! hair library) follow the body's shape: `asset_delta = conform.apply(body_delta)`.
//!
//! Each asset vertex is bound at load time to its `k` nearest *anchor* vertices
//! of the body (the scalp for scalp hair, the face for beards and brows) with
//! Gaussian weights `exp(-((d - d_min) / σ)²)`, `σ = max(sigma_min, sigma_scale · d_min)`:
//! verts on the skin follow their spot, hair far from it averages a wider area.
//! With `similarity`, the anchors' move is first fitted by one similarity
//! transform (scale, rotation, translation; Horn's method), applied to the asset
//! as a whole, and only the anchors' residual moves are blended. That keeps long
//! hair in proportion when the head scales or turns.
//!
//! Calibrated against CC5 itself (`check_conform_cc5.py`: the 14 Smart Hair
//! groups with whole recipes applied by CC5): scalp hair that deforms (hair caps,
//! short styles) anchored to the whole head + similarity: p50 0.4 mm, p99 3–4.7
//! mm; beards and brows on the face: p50 0.3–0.6 mm.
//!
//! [`ConformOptions::HEAD_BOX`]: CC5 moves some hair layers rigidly (Smart Hair
//! bangs, buns, the long outer layers; CC5 itself with whole recipes applied:
//! an exact per-axis scale + translation): it maps the head's bounding box. The
//! box of the head without ears and nose (`regions.json` "head_box") reproduces
//! it to p50 0.5–1.8 mm, p99 ≤ 2.2 mm, where surface conform is 15–20 mm off.
//! Layers lying on the scalp / skin (hair caps, beards, brows) deform with it
//! and keep the surface conform (p50 0.3–2 mm).

use anyhow::{bail, Result};
use glam::{DMat3, DVec3, DVec4};
use rayon::prelude::*;

#[derive(Debug, Clone, Copy)]
pub struct ConformOptions {
    /// Anchors per asset vertex.
    pub k: usize,
    /// Minimum Gaussian width (m).
    pub sigma_min: f32,
    /// Gaussian width per metre of distance from the anchors.
    pub sigma_scale: f32,
    /// Fit and apply one similarity transform of all anchors first.
    pub similarity: bool,
    /// Only map the anchors' axis-aligned bounding box (per-axis scale +
    /// translation); `k` and the Gaussians are unused.
    pub box_fit: bool,
}

impl ConformOptions {
    /// Scalp hair (anchors: the "head" region): falloff widening with the
    /// distance from the head (roots and a scalp cap stay on their spot, hair
    /// further out averages a wider area) + a whole-head similarity.
    pub const SCALP: Self = Self { k: 16, sigma_min: 0.005, sigma_scale: 0.5, similarity: true, box_fit: false };
    /// Beards, brows, anything lying on the face: follow the nearest skin.
    pub const SKIN: Self = Self { k: 16, sigma_min: 0.003, sigma_scale: 0.5, similarity: false, box_fit: false };
    /// Hair layers CC5 moves rigidly with the head's bounding box (anchors: the
    /// "head_box" region).
    pub const HEAD_BOX: Self = Self { k: 0, sigma_min: 0.0, sigma_scale: 0.0, similarity: false, box_fit: true };
}

pub struct Conform {
    opts: ConformOptions,
    /// Anchor indices into the body vertex array.
    anchors: Vec<u32>,
    anchor_bind: Vec<DVec3>,
    asset_bind: Vec<DVec3>,
    /// Per asset vertex: `k` (anchor slot, weight) pairs, flattened.
    bind: Vec<(u32, f32)>,
}

impl Conform {
    /// `body`: the body's bind positions; `anchors`: indices into `body`;
    /// `asset`: the asset's bind positions, same space as `body`.
    pub fn bind(body: &[[f32; 3]], anchors: &[u32], asset: &[[f32; 3]], opts: ConformOptions) -> Result<Self> {
        if anchors.is_empty() {
            bail!("conform: no anchor vertices");
        }
        if let Some(&bad) = anchors.iter().find(|&&a| a as usize >= body.len()) {
            bail!("conform: anchor {bad} past the body's {} vertices", body.len());
        }
        let anchor_bind: Vec<DVec3> = anchors.iter().map(|&a| DVec3::from(body[a as usize].map(f64::from))).collect();
        let asset_bind: Vec<DVec3> = asset.iter().map(|p| DVec3::from(p.map(f64::from))).collect();
        if opts.box_fit {
            return Ok(Self { opts: ConformOptions { k: 0, ..opts }, anchors: anchors.to_vec(), anchor_bind, asset_bind, bind: Vec::new() });
        }
        let k = opts.k.min(anchors.len()).max(1);
        // k nearest anchors per vertex from a uniform grid (exact), in parallel:
        // millions of hair vertices against thousands of anchors
        let grid = AnchorGrid::new(&anchor_bind);
        let mut bind = vec![(0u32, 0f32); asset_bind.len() * k];
        bind.par_chunks_mut(k).zip(asset_bind.par_iter()).for_each_init(Vec::new, |near, (out, &p)| {
            grid.nearest(&anchor_bind, p, k, near);
            gaussian_weights(&near[..k], opts, out);
        });
        Ok(Self {
            opts: ConformOptions { k, ..opts },
            anchors: anchors.to_vec(),
            anchor_bind,
            asset_bind,
            bind,
        })
    }

    pub fn len(&self) -> usize {
        self.asset_bind.len()
    }

    pub fn is_empty(&self) -> bool {
        self.asset_bind.is_empty()
    }

    /// Asset vertex deltas for a body delta (one entry per body vertex, same
    /// space as the bind positions).
    pub fn apply(&self, body_delta: &[[f32; 3]]) -> Vec<[f32; 3]> {
        let d: Vec<DVec3> = self.anchors.iter().map(|&a| DVec3::from(body_delta[a as usize].map(f64::from))).collect();
        if self.opts.box_fit {
            let bounds = |pts: &mut dyn Iterator<Item = DVec3>| pts.fold((DVec3::MAX, DVec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
            let (lo0, hi0) = bounds(&mut self.anchor_bind.iter().copied());
            let (lo1, hi1) = bounds(&mut self.anchor_bind.iter().zip(&d).map(|(p, d)| *p + *d));
            let ext = hi0 - lo0;
            let s = DVec3::select(ext.cmpgt(DVec3::splat(1e-9)), (hi1 - lo1) / ext, DVec3::ONE);
            let t = lo1 - s * lo0;
            return self.asset_bind.par_iter().map(|p| (*p * s + t - *p).as_vec3().to_array()).collect();
        }
        let (global, residual): (Option<Similarity>, Vec<DVec3>) = if self.opts.similarity && d.iter().any(|v| *v != DVec3::ZERO) {
            let moved: Vec<DVec3> = self.anchor_bind.iter().zip(&d).map(|(p, d)| *p + *d).collect();
            let s = Similarity::fit(&self.anchor_bind, &moved);
            let r = self.anchor_bind.iter().zip(&d).map(|(p, d)| *d - (s.apply(*p) - *p)).collect();
            (Some(s), r)
        } else {
            (None, d)
        };
        let k = self.opts.k;
        self.asset_bind
            .par_iter()
            .enumerate()
            .map(|(v, p)| {
                let mut acc = global.as_ref().map_or(DVec3::ZERO, |s| s.apply(*p) - *p);
                for &(i, w) in &self.bind[v * k..(v + 1) * k] {
                    acc += residual[i as usize] * w as f64;
                }
                acc.as_vec3().to_array()
            })
            .collect()
    }
}

/// Weights `exp(-((d - d_min) / σ)²)` (normalised) for the `k` nearest anchors
/// `near` (squared distance, anchor slot), nearest first.
fn gaussian_weights(near: &[(f64, u32)], opts: ConformOptions, out: &mut [(u32, f32)]) {
    let d0 = near[0].0.sqrt();
    let sigma = (opts.sigma_min as f64).max(opts.sigma_scale as f64 * d0).max(1e-6);
    let mut sum = 0.0;
    for (o, &(d2, i)) in out.iter_mut().zip(near) {
        let w = (-((d2.sqrt() - d0) / sigma).powi(2)).exp();
        *o = (i, w as f32);
        sum += w;
    }
    for o in out.iter_mut() {
        o.1 = (o.1 as f64 / sum) as f32;
    }
}

/// Anchor points bucketed in a uniform grid, for exact k-nearest queries.
struct AnchorGrid {
    origin: DVec3,
    cell: f64,
    dims: [i64; 3],
    /// CSR: anchors of cell `c` are `items[starts[c]..starts[c + 1]]`.
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl AnchorGrid {
    fn new(pts: &[DVec3]) -> Self {
        let (lo, hi) = pts.iter().fold((DVec3::MAX, DVec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let ext = (hi - lo).max(DVec3::splat(1e-6));
        // about one anchor per cell over the bounding volume (surface anchors
        // fill only a shell of it: a few per occupied cell), at most 128 a side
        let cell = (ext.x * ext.y * ext.z / pts.len() as f64).cbrt().max(ext.max_element() / 128.0).max(1e-4);
        let dims = [0, 1, 2].map(|a| ((ext[a] / cell).floor() as i64 + 1).max(1));
        let n = (dims[0] * dims[1] * dims[2]) as usize;
        let index = |p: &DVec3| {
            let c = ((*p - lo) / cell).floor();
            let c = [0, 1, 2].map(|a| (c[a] as i64).clamp(0, dims[a] - 1));
            ((c[2] * dims[1] + c[1]) * dims[0] + c[0]) as usize
        };
        let mut starts = vec![0u32; n + 1];
        for p in pts {
            starts[index(p) + 1] += 1;
        }
        for c in 0..n {
            starts[c + 1] += starts[c];
        }
        let mut fill = starts.clone();
        let mut items = vec![0u32; pts.len()];
        for (i, p) in pts.iter().enumerate() {
            let c = index(p);
            items[fill[c] as usize] = i as u32;
            fill[c] += 1;
        }
        Self { origin: lo, cell, dims, starts, items }
    }

    /// The `k` anchors nearest `p` into `out[..k]` (squared distance, index),
    /// nearest first. Searches cubic shells of cells around `p`'s cell: after
    /// shell `r` every unseen anchor is at least `r` cells away, so the search
    /// stops once the k-th distance is within that.
    fn nearest(&self, pts: &[DVec3], p: DVec3, k: usize, out: &mut Vec<(f64, u32)>) {
        out.clear();
        let c = ((p - self.origin) / self.cell).floor();
        let c = [c.x as i64, c.y as i64, c.z as i64];
        // shells beyond this reach no cell of the grid
        let r_max = (0..3).map(|a| (c[a]).max(self.dims[a] - 1 - c[a]).max(0)).max().unwrap_or(0);
        let mut r = 0i64;
        loop {
            for z in c[2] - r..=c[2] + r {
                if z < 0 || z >= self.dims[2] {
                    continue;
                }
                for y in c[1] - r..=c[1] + r {
                    if y < 0 || y >= self.dims[1] {
                        continue;
                    }
                    let edge = (z - c[2]).abs() == r || (y - c[1]).abs() == r;
                    // inside the shell only the two x ends are new
                    let xs: &mut dyn Iterator<Item = i64> = if edge { &mut (c[0] - r..=c[0] + r) } else if r == 0 { &mut std::iter::once(c[0]) } else { &mut [c[0] - r, c[0] + r].into_iter() };
                    for x in xs {
                        if x < 0 || x >= self.dims[0] {
                            continue;
                        }
                        let cell = ((z * self.dims[1] + y) * self.dims[0] + x) as usize;
                        for &i in &self.items[self.starts[cell] as usize..self.starts[cell + 1] as usize] {
                            out.push((pts[i as usize].distance_squared(p), i));
                        }
                    }
                }
            }
            if out.len() >= k {
                out.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                out.truncate(k);
                let reach = r as f64 * self.cell;
                if out[k - 1].0 <= reach * reach || r >= r_max {
                    break;
                }
            } else if r >= r_max {
                break;
            }
            r += 1;
        }
        out.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    }
}

/// `q ≈ s·R·p + t`.
struct Similarity {
    s: f64,
    r: DMat3,
    t: DVec3,
}

impl Similarity {
    fn apply(&self, p: DVec3) -> DVec3 {
        self.s * (self.r * p) + self.t
    }

    /// Horn's quaternion method (least squares), scale from the rotated spread.
    fn fit(p: &[DVec3], q: &[DVec3]) -> Self {
        let n = p.len() as f64;
        let (mp, mq) = (p.iter().sum::<DVec3>() / n, q.iter().sum::<DVec3>() / n);
        let mut m = [[0.0f64; 3]; 3]; // Σ a bᵀ
        let mut aa = 0.0;
        for (a, b) in p.iter().zip(q) {
            let (a, b) = (*a - mp, *b - mq);
            aa += a.length_squared();
            for (i, ai) in a.to_array().iter().enumerate() {
                for (j, bj) in b.to_array().iter().enumerate() {
                    m[i][j] += ai * bj;
                }
            }
        }
        let [[sxx, sxy, sxz], [syx, syy, syz], [szx, szy, szz]] = m;
        let nmat = [
            [sxx + syy + szz, syz - szy, szx - sxz, sxy - syx],
            [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
            [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
            [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz],
        ];
        let qv = max_eigenvector4(nmat);
        let rot = glam::DQuat::from_xyzw(qv.y, qv.z, qv.w, qv.x).normalize();
        let r = DMat3::from_quat(rot);
        let mut ab = 0.0;
        for (a, b) in p.iter().zip(q) {
            ab += (*b - mq).dot(r * (*a - mp));
        }
        let s = if aa > 0.0 { ab / aa } else { 1.0 };
        Self { s, r, t: mq - s * (r * mp) }
    }
}

/// Eigenvector of the largest eigenvalue of a symmetric 4×4 (cyclic Jacobi).
fn max_eigenvector4(mut a: [[f64; 4]; 4]) -> DVec4 {
    let mut v = [[0.0f64; 4]; 4];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..50 {
        let off: f64 = (0..4).flat_map(|i| (0..4).filter(move |&j| j != i).map(move |j| (i, j))).map(|(i, j)| a[i][j] * a[i][j]).sum();
        if off < 1e-24 {
            break;
        }
        for p in 0..3 {
            for q in p + 1..4 {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..4 {
                    let (akp, akq) = (a[k][p], a[k][q]);
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..4 {
                    let (apk, aqk) = (a[p][k], a[q][k]);
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    let best = (0..4).max_by(|&i, &j| a[i][i].total_cmp(&a[j][j])).unwrap();
    DVec4::new(v[0][best], v[1][best], v[2][best], v[3][best])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Vec<[f32; 3]> {
        let mut g = Vec::new();
        for i in 0..10 {
            for j in 0..10 {
                g.push([i as f32 * 0.02 - 0.09, j as f32 * 0.02 - 0.09, 0.0]);
            }
        }
        g
    }

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < tol)
    }

    #[test]
    fn translation_carries_assets_along() {
        let body = grid();
        let anchors: Vec<u32> = (0..body.len() as u32).collect();
        let asset = [[0.0, 0.0, 0.01], [0.03, -0.02, 0.2]];
        for opts in [ConformOptions::SCALP, ConformOptions::SKIN] {
            let c = Conform::bind(&body, &anchors, &asset, opts).unwrap();
            let out = c.apply(&vec![[0.01, -0.02, 0.005]; body.len()]);
            assert!(out.iter().all(|d| close(*d, [0.01, -0.02, 0.005], 1e-5)), "{out:?}");
        }
    }

    #[test]
    fn similarity_scales_far_vertices() {
        let body = grid();
        let anchors: Vec<u32> = (0..body.len() as u32).collect();
        let asset = [[0.0, 0.0, 0.3]];
        // the body grows 10% about the origin; a vertex 30 cm out moves 3 cm
        let delta: Vec<[f32; 3]> = body.iter().map(|p| p.map(|x| x * 0.1)).collect();
        let c = Conform::bind(&body, &anchors, &asset, ConformOptions::SCALP).unwrap();
        // a flat grid fixes no scale along z; the fitted scale applies uniformly
        assert!(close(c.apply(&delta)[0], [0.0, 0.0, 0.03], 1e-4), "{:?}", c.apply(&delta));
        // rotation by 90° about z: (0.05, 0, 0.3) -> (0, 0.05, 0.3)
        let rot: Vec<[f32; 3]> = body.iter().map(|p| [-p[1] - p[0], p[0] - p[1], 0.0]).collect();
        let c = Conform::bind(&body, &anchors, &[[0.05, 0.0, 0.3]], ConformOptions::SCALP).unwrap();
        assert!(close(c.apply(&rot)[0], [-0.05, 0.05, 0.0], 1e-4), "{:?}", c.apply(&rot));
    }

    #[test]
    fn head_box_scales_per_axis() {
        let body = grid();
        let anchors: Vec<u32> = (0..body.len() as u32).collect();
        // the grid grows 10% along x and moves 1 cm along z
        let delta: Vec<[f32; 3]> = body.iter().map(|p| [p[0] * 0.1, 0.0, 0.01]).collect();
        let c = Conform::bind(&body, &anchors, &[[0.2, 0.05, 0.3]], ConformOptions::HEAD_BOX).unwrap();
        assert!(close(c.apply(&delta)[0], [0.02, 0.0, 0.01], 1e-5), "{:?}", c.apply(&delta));
    }

    /// The grid search gives the brute-force nearest anchors and weights.
    #[test]
    fn grid_bind_matches_brute_force() {
        // xorshift: deterministic pseudo-random points without a dependency
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        // anchors on a sphere (like a scalp), assets around and far outside it
        let body: Vec<[f32; 3]> = (0..3000)
            .map(|_| {
                let (u, v) = (rnd() * std::f64::consts::TAU, rnd() * 2.0 - 1.0);
                let r = (1.0 - v * v).sqrt();
                [(0.1 * r * u.cos()) as f32, (0.1 * r * u.sin()) as f32, (0.1 * v) as f32]
            })
            .collect();
        let anchors: Vec<u32> = (0..body.len() as u32).collect();
        let asset: Vec<[f32; 3]> = (0..2000).map(|i| {
            let f = if i % 10 == 0 { 3.0 } else { 0.3 };
            [((rnd() - 0.5) * f) as f32, ((rnd() - 0.5) * f) as f32, ((rnd() - 0.5) * f) as f32]
        }).collect();
        for opts in [ConformOptions::SKIN, ConformOptions::SCALP] {
            let c = Conform::bind(&body, &anchors, &asset, opts).unwrap();
            let anchor_bind: Vec<DVec3> = body.iter().map(|p| DVec3::from(p.map(f64::from))).collect();
            for (v, p) in asset.iter().enumerate() {
                let p = DVec3::from(p.map(f64::from));
                let mut all: Vec<(f64, u32)> = anchor_bind.iter().enumerate().map(|(i, a)| (a.distance_squared(p), i as u32)).collect();
                all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let mut want = vec![(0u32, 0f32); c.opts.k];
                gaussian_weights(&all[..c.opts.k], opts, &mut want);
                assert_eq!(&c.bind[v * c.opts.k..(v + 1) * c.opts.k], &want[..], "vertex {v}");
            }
        }
    }

    #[test]
    fn skin_vertices_follow_their_spot() {
        let body = grid();
        let anchors: Vec<u32> = (0..body.len() as u32).collect();
        // only one body vertex moves; an asset vertex right on it follows, a far one does not
        let mut delta = vec![[0.0f32; 3]; body.len()];
        delta[55] = [0.0, 0.0, 0.01];
        let asset = [[body[55][0], body[55][1], 0.001], [body[0][0], body[0][1], 0.001]];
        let c = Conform::bind(&body, &anchors, &asset, ConformOptions::SKIN).unwrap();
        let out = c.apply(&delta);
        assert!(out[0][2] > 0.009, "{out:?}");
        assert!(out[1][2].abs() < 1e-6, "{out:?}");
    }
}
