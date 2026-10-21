//! Convert Character Creator 5 hair into strand hair parts for cc-render's
//! avatar editor, in Rust (the port of hair_build.py's "convert" path, so the
//! editor can rebuild hair live).
//!
//! Stages (each a plain function, so callers can cache their outputs):
//! [`source::Source`] (a CC5 character GLB or FBX export) -> [`cards::cards_to_strands`]
//! (strands following CC5's cards) -> [`shape::finish`] (shape edits, push off
//! the head) -> [`shade::depth_shade`] -> [`part::build`] (meshes, materials,
//! catalog entry) -> [`glb_out::write`] (a library part GLB).
//!
//! Frame: glTF (metres, Y up), the CC5 GLBs' mesh space.

pub mod cards;
pub mod follow;
pub mod glb_out;
pub mod head;
pub mod part;
pub mod settings;
pub mod shade;
pub mod shape;
pub mod source;

use glam::Vec3;
use rand::{rngs::StdRng, Rng as _, SeedableRng};

/// Between the eyes of the CC base character (glTF frame).
/// A CC5 Smart Hair group's export in `dir`: `character.glb` (prepared), else
/// the raw `character.fbx`.
pub fn group_source(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    ["character.glb", "character.fbx"].iter().map(|f| dir.join(f)).find(|p| p.exists())
}

pub const HEAD_CENTER: Vec3 = Vec3::new(0.0, 1.64, 0.005);

/// One strand: a polyline from the root, with per-point linear colour.
#[derive(Debug, Clone)]
pub struct Strand {
    pub pts: Vec<Vec3>,
    /// Random 0..1 (the ID map's value: per-strand variation in the shader).
    pub id: f32,
    /// Ribbon width at the root (m).
    pub width: f32,
    /// Brightness factor.
    pub shade: f32,
    /// Linear RGB per point.
    pub rgb: Vec<Vec3>,
}

/// Seeded random numbers (Python's `random.Random` calls the port needs).
pub struct Rng(StdRng);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(StdRng::seed_from_u64(seed))
    }
    /// Uniform in 0..1.
    pub fn f(&mut self) -> f64 {
        self.0.gen::<f64>()
    }
    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
    /// Normal deviate (Box-Muller).
    pub fn gauss(&mut self, mu: f64, sigma: f64) -> f64 {
        let u1 = self.f().max(1e-300);
        let u2 = self.f();
        mu + sigma * (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
    pub fn below(&mut self, n: usize) -> usize {
        self.0.gen_range(0..n)
    }
}

/// sRGB-encoded 0..1 -> linear.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// `n` points evenly spaced along a polyline.
pub fn uniform_polyline(pts: &[Vec3], n: usize) -> Vec<Vec3> {
    if pts.len() < 2 || n < 2 {
        return vec![pts.first().copied().unwrap_or(Vec3::ZERO); n.max(1)];
    }
    let mut s = vec![0.0f32];
    for w in pts.windows(2) {
        s.push(s.last().unwrap() + w[0].distance(w[1]));
    }
    let total = *s.last().unwrap();
    let mut out = Vec::with_capacity(n);
    let mut j = 0;
    for i in 0..n {
        let t = total * i as f32 / (n - 1) as f32;
        while j + 1 < s.len() - 1 && s[j + 1] < t {
            j += 1;
        }
        let seg = s[j + 1] - s[j];
        let f = if seg > 0.0 { ((t - s[j]) / seg).clamp(0.0, 1.0) } else { 0.0 };
        out.push(pts[j].lerp(pts[j + 1], f));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polyline_resamples_evenly() {
        let pts = [Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 3.0, 0.0)];
        let out = uniform_polyline(&pts, 5);
        assert_eq!(out.len(), 5);
        assert!(out[0].distance(Vec3::ZERO) < 1e-6);
        assert!(out[4].distance(Vec3::new(1.0, 3.0, 0.0)) < 1e-5);
        // 4 units of length: one point per unit
        assert!(out[1].distance(Vec3::new(1.0, 0.0, 0.0)) < 1e-5);
        assert!(out[2].distance(Vec3::new(1.0, 1.0, 0.0)) < 1e-5);
    }
}
