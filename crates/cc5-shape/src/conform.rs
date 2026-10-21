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
        let mut bind = Vec::with_capacity(asset.len() * k);
        let mut dist: Vec<(f64, u32)> = Vec::with_capacity(anchors.len());
        for p in asset {
            let p = DVec3::from(p.map(f64::from));
            dist.clear();
            dist.extend(anchor_bind.iter().enumerate().map(|(i, a)| (a.distance_squared(p), i as u32)));
            dist.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0));
            let near = &mut dist[..k];
            near.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            let d0 = near[0].0.sqrt();
            let sigma = (opts.sigma_min as f64).max(opts.sigma_scale as f64 * d0).max(1e-6);
            let w: Vec<f64> = near.iter().map(|&(d2, _)| (-((d2.sqrt() - d0) / sigma).powi(2)).exp()).collect();
            let sum: f64 = w.iter().sum();
            bind.extend(near.iter().zip(&w).map(|(&(_, i), &w)| (i, (w / sum) as f32)));
        }
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
            return self.asset_bind.iter().map(|p| (*p * s + t - *p).as_vec3().to_array()).collect();
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
            .iter()
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
