//! Darken strand points by their depth in the hair (hair_build.py `depth_shade`):
//! distance from the head relative to the outermost hair over the same patch
//! of scalp (cells of `cell` m by the nearest head point). Hair underneath and
//! at the roots reads darker, the outer surface keeps its colour: the depth CC5
//! paints into its cards. Thin hair (hairlines, partings: outermost hair under
//! `thick_ref`) is darkened in proportion, so hairlines stay light.

use std::collections::HashMap;

use glam::IVec3;
use rayon::prelude::*;

use crate::head::Body;
use crate::Strand;

pub fn depth_shade(strands: &mut [Strand], body: &Body, ao_min: f32, gamma: f32) {
    const CELL: f32 = 0.015;
    const THICK_REF: f32 = 0.012;
    let per: Vec<Vec<(f32, IVec3)>> = strands
        .par_iter()
        .map(|s| {
            s.pts
                .iter()
                .map(|p| {
                    let h = body.nearest(*p);
                    (h.dist, (h.point / CELL).floor().as_ivec3())
                })
                .collect()
        })
        .collect();
    let mut outer: HashMap<IVec3, f32> = HashMap::new();
    for pts in &per {
        for &(d, k) in pts {
            let e = outer.entry(k).or_insert(0.0);
            *e = e.max(d);
        }
    }
    strands.par_iter_mut().zip(&per).for_each(|(s, pts)| {
        for (c, &(d, k)) in s.rgb.iter_mut().zip(pts) {
            let big_d = outer[&k].max(1e-4);
            let ao = ao_min + (1.0 - ao_min) * (d / big_d).clamp(0.0, 1.0).powf(gamma);
            let ao = 1.0 - (1.0 - ao) * (big_d / THICK_REF).clamp(0.0, 1.0);
            *c *= ao;
        }
    });
}
