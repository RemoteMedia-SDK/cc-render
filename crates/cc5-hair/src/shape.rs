//! Shape edits on converted strands (length, comb, gravity, curl, frizz), then
//! the push off the head and the source's scalp layers (hair_build.py's
//! per-point fix in `cards_to_strands`).

use glam::Vec3;
use rayon::prelude::*;

use crate::head::{Body, Surface};
use crate::settings::ShapeEdits;
use crate::{uniform_polyline, Rng, Strand, HEAD_CENTER};

/// Apply `edits` (no-ops at their defaults), then keep every point outside the
/// body (0.8 mm) and above the scalp layers (0.6 mm).
pub fn finish(strands: &mut [Strand], edits: &ShapeEdits, body: &Body, scalp: Option<&Surface>, seed: u64) {
    let active = *edits != ShapeEdits { length: 1.0, ..Default::default() };
    strands.par_iter_mut().enumerate().for_each(|(i, s)| {
        if active {
            let mut rng = Rng::new(seed ^ (i as u64).wrapping_mul(0xd1b5_4a32_d192_ed03));
            edit(s, edits, &mut rng);
        }
        for q in s.pts.iter_mut() {
            *q = push(*q, body, scalp);
        }
    });
}

fn push(mut q: Vec3, body: &Body, scalp: Option<&Surface>) -> Vec3 {
    let h = body.nearest(q);
    if (q - h.point).dot(h.normal) < 0.0 || h.dist < 0.0008 {
        q = h.point + h.normal * 0.0008;
    }
    if let Some(s) = scalp {
        let h = s.nearest(q);
        if h.dist < 0.004 {
            let n = if h.normal.dot(h.point - HEAD_CENTER) < 0.0 { -h.normal } else { h.normal };
            if (q - h.point).dot(n) < 0.0006 {
                q = h.point + n * 0.0006;
            }
        }
    }
    q
}

fn edit(s: &mut Strand, e: &ShapeEdits, rng: &mut Rng) {
    let n = s.pts.len();
    if n < 2 {
        return;
    }
    let len0: f32 = s.pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    // length: trim the tip, or extend along the last segment
    if (e.length - 1.0).abs() > 1e-6 && len0 > 0.0 {
        let target = len0 * e.length.max(0.05) as f32;
        let mut pts = s.pts.clone();
        if target > len0 {
            let d = (pts[n - 1] - pts[n - 2]).normalize_or_zero();
            pts.push(pts[n - 1] + d * (target - len0));
        } else {
            pts = cut(&pts, target);
        }
        s.pts = uniform_polyline(&pts, n);
    }
    let len: f32 = s.pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    let t = |i: usize| i as f32 / (n - 1) as f32;
    // comb: segment directions blend towards the comb direction, more at the tip
    let comb = Vec3::new(e.comb[0] as f32, e.comb[1] as f32, e.comb[2] as f32).normalize_or_zero();
    if e.comb_strength > 0.0 && comb != Vec3::ZERO {
        let mut out = vec![s.pts[0]];
        for i in 1..n {
            let seg = s.pts[i] - s.pts[i - 1];
            let w = (e.comb_strength as f32 * t(i).powf(1.5)).clamp(0.0, 1.0);
            let d = seg.normalize_or_zero().lerp(comb, w).normalize_or_zero();
            out.push(out[i - 1] + d * seg.length());
        }
        s.pts = out;
    }
    // gravity: droop along world down (glTF: -Y), scaled by the strand's length
    if e.gravity != 0.0 {
        let k = e.gravity as f32 * (len / 0.1);
        for (i, p) in s.pts.iter_mut().enumerate() {
            p.y -= k * t(i) * t(i);
        }
    }
    // curl: a helix around the strand, fading in from the root
    if e.curl_radius > 0.0 && e.curl_period > 0.0 {
        let base = s.pts.clone();
        let phase = rng.uniform(0.0, std::f64::consts::TAU) as f32;
        let mut arc = 0.0f32;
        for i in 0..n {
            if i > 0 {
                arc += base[i].distance(base[i - 1]);
            }
            let tan = (base[(i + 1).min(n - 1)] - base[i.saturating_sub(1)]).normalize_or_zero();
            let side = tan.cross(if tan.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }).normalize_or_zero();
            let up = tan.cross(side);
            let th = phase + std::f32::consts::TAU * arc / e.curl_period as f32;
            let fade = smoothstep(t(i) / 0.2);
            s.pts[i] = base[i] + (side * th.cos() + up * th.sin()) * e.curl_radius as f32 * fade;
        }
    }
    // frizz: seeded offsets growing towards the tip
    if e.frizz > 0.0 {
        for (i, p) in s.pts.iter_mut().enumerate() {
            let g = Vec3::from(std::array::from_fn(|_| rng.gauss(0.0, 1.0) as f32));
            *p += g * e.frizz as f32 * t(i) * t(i);
        }
    }
    s.pts = uniform_polyline(&s.pts, n);
}

/// The polyline up to arc length `target`.
fn cut(pts: &[Vec3], target: f32) -> Vec<Vec3> {
    let mut out = vec![pts[0]];
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let l = w[0].distance(w[1]);
        if acc + l >= target {
            let f = if l > 0.0 { (target - acc) / l } else { 0.0 };
            out.push(w[0].lerp(w[1], f));
            return out;
        }
        acc += l;
        out.push(w[1]);
    }
    out
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strand() -> Strand {
        let pts: Vec<Vec3> = (0..11).map(|i| Vec3::new(0.0, 0.0, i as f32 * 0.01)).collect();
        Strand { rgb: vec![Vec3::ONE; pts.len()], pts, id: 0.5, width: 1e-4, shade: 1.0 }
    }

    fn length(s: &Strand) -> f32 {
        s.pts.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    #[test]
    fn length_trims_and_extends() {
        let mut rng = Rng::new(1);
        let mut s = strand();
        edit(&mut s, &ShapeEdits { length: 0.5, ..Default::default() }, &mut rng);
        assert!((length(&s) - 0.05).abs() < 1e-4, "{}", length(&s));
        let mut s = strand();
        edit(&mut s, &ShapeEdits { length: 1.5, ..Default::default() }, &mut rng);
        assert!((length(&s) - 0.15).abs() < 1e-4, "{}", length(&s));
        assert_eq!(s.pts.len(), 11);
    }

    #[test]
    fn gravity_pulls_tips_down() {
        let mut s = strand();
        edit(&mut s, &ShapeEdits { length: 1.0, gravity: 0.02, ..Default::default() }, &mut Rng::new(1));
        assert!(s.pts[0].y.abs() < 1e-6);
        assert!(s.pts[10].y < -0.015, "{:?}", s.pts[10]);
    }

    #[test]
    fn curl_keeps_its_radius() {
        let mut s = strand();
        edit(&mut s, &ShapeEdits { length: 1.0, curl_radius: 0.003, curl_period: 0.02, ..Default::default() }, &mut Rng::new(1));
        // past the fade-in, points sit ~3 mm off the original axis
        let r = (s.pts[8].x * s.pts[8].x + s.pts[8].y * s.pts[8].y).sqrt();
        assert!((r - 0.003).abs() < 0.0008, "{r}");
    }
}
