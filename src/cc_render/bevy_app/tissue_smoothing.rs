//! Opt-in smoothing of the embedded skin displacement. Skin follows each cage
//! tetrahedron linearly, so a large swing creases it along cage faces (2-3 cm
//! apart). Taubin-smoothing the displacement over the welded skin graph removes
//! those kinks; skin outside the tissue stays fixed and tissue nodes, contacts
//! and physics are unchanged. `CC_AVATAR_TISSUE_SURFACE_SMOOTHING` sets the
//! number of Taubin passes (default 0: off).
use bevy::prelude::*;
use std::collections::{BTreeSet, HashMap};

const LAMBDA: f32 = 0.5;
const MU: f32 = -0.53;

pub(super) struct DisplacementSmoothing {
    passes: usize,
    /// Mesh vertices of each welded node; UV seams split vertices that share
    /// a position, and smoothing across the seam must treat them as one.
    members: Vec<Vec<usize>>,
    neighbours: Vec<Vec<usize>>,
    movable: Vec<bool>,
}

impl DisplacementSmoothing {
    pub fn from_env(base: &[Vec3], triangles: &[[usize; 3]], affected: &BTreeSet<usize>) -> anyhow::Result<Option<Self>> {
        let passes = match std::env::var("CC_AVATAR_TISSUE_SURFACE_SMOOTHING") {
            Ok(value) => value.parse::<usize>().map_err(|_| anyhow::anyhow!("tissue surface smoothing must be a pass count"))?,
            Err(_) => 0,
        };
        anyhow::ensure!(passes <= 256, "tissue surface smoothing passes exceed 256");
        Ok((passes > 0).then(|| Self::new(passes, base, triangles, affected)))
    }

    pub fn new(passes: usize, base: &[Vec3], triangles: &[[usize; 3]], affected: &BTreeSet<usize>) -> Self {
        let mut node_of = HashMap::new();
        let mut members: Vec<Vec<usize>> = Vec::new();
        let key = |p: Vec3| (p * 1e6).round().as_ivec3().to_array();
        for &v in triangles.iter().flatten().collect::<BTreeSet<_>>() {
            let node = *node_of.entry(key(base[v])).or_insert_with(|| {
                members.push(Vec::new());
                members.len() - 1
            });
            members[node].push(v);
        }
        let node = |v: usize| node_of[&key(base[v])];
        let mut neighbours = vec![BTreeSet::new(); members.len()];
        for t in triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (node(a), node(b));
                if a != b {
                    neighbours[a].insert(b);
                    neighbours[b].insert(a);
                }
            }
        }
        let movable = members.iter().map(|m| m.iter().any(|v| affected.contains(v))).collect();
        Self { passes, members, neighbours: neighbours.into_iter().map(|n| n.into_iter().collect()).collect(), movable }
    }

    /// Smooth `delta` in place; vertices outside the tissue keep zero.
    pub fn apply(&self, delta: &mut [Vec3]) {
        let mut field: Vec<Vec3> = self.members.iter().zip(&self.movable).map(|(m, &movable)| {
            if movable { m.iter().map(|&v| delta[v]).sum::<Vec3>() / m.len() as f32 } else { Vec3::ZERO }
        }).collect();
        let mut next = field.clone();
        for _ in 0..self.passes {
            for factor in [LAMBDA, MU] {
                for (i, around) in self.neighbours.iter().enumerate() {
                    if self.movable[i] && !around.is_empty() {
                        let mean = around.iter().map(|&j| field[j]).sum::<Vec3>() / around.len() as f32;
                        next[i] = field[i] + factor * (mean - field[i]);
                    }
                }
                std::mem::swap(&mut field, &mut next);
            }
        }
        for (node, vertices) in self.members.iter().enumerate() {
            if self.movable[node] {
                for &v in vertices {
                    delta[v] = field[node];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip of quads; vertex 2*i is bottom, 2*i+1 top. Seam duplicates
    /// the middle column so welding is exercised.
    fn strip(columns: usize) -> (Vec<Vec3>, Vec<[usize; 3]>) {
        let mut base = Vec::new();
        for i in 0..columns {
            base.push(Vec3::new(i as f32, 0., 0.));
            base.push(Vec3::new(i as f32, 1., 0.));
        }
        let mut triangles = Vec::new();
        for i in 0..columns - 1 {
            let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            triangles.push([a, c, b]);
            triangles.push([b, c, d]);
        }
        (base, triangles)
    }

    #[test]
    fn kinks_are_smoothed_and_fixed_skin_stays_put() {
        let (base, triangles) = strip(9);
        let affected: BTreeSet<_> = (2..16).collect();
        let smoothing = DisplacementSmoothing::new(8, &base, &triangles, &affected);
        // A sharp tent: one column displaced, the rest flat.
        let mut delta = vec![Vec3::ZERO; base.len()];
        delta[8] = Vec3::Z;
        delta[9] = Vec3::Z;
        smoothing.apply(&mut delta);
        assert_eq!(delta[0], Vec3::ZERO);
        assert_eq!(delta[17], Vec3::ZERO);
        assert!(delta[8].z < 0.6 && delta[6].z > 0.05 && delta[10].z > 0.05);
        assert!((delta[8] - delta[9]).length() < 1e-6);
    }

    #[test]
    fn seam_duplicates_move_together() {
        let (mut base, mut triangles) = strip(5);
        // Split vertex 4 into a seam duplicate used by the right half.
        base.push(base[4]);
        for t in triangles.iter_mut().skip(4) {
            for v in t.iter_mut() {
                if *v == 4 {
                    *v = 10;
                }
            }
        }
        let affected: BTreeSet<_> = [2, 3, 4, 5, 6, 7, 10].into();
        let smoothing = DisplacementSmoothing::new(4, &base, &triangles, &affected);
        let mut delta = vec![Vec3::ZERO; base.len()];
        delta[4] = Vec3::Y;
        delta[10] = Vec3::Y;
        smoothing.apply(&mut delta);
        assert_eq!(delta[4], delta[10]);
    }

    #[test]
    fn uniform_interior_motion_is_kept_away_from_the_boundary() {
        let (base, triangles) = strip(41);
        let affected: BTreeSet<_> = (2..80).collect();
        let smoothing = DisplacementSmoothing::new(4, &base, &triangles, &affected);
        let mut delta: Vec<_> = (0..base.len()).map(|v| if affected.contains(&v) { Vec3::X } else { Vec3::ZERO }).collect();
        smoothing.apply(&mut delta);
        assert!((delta[40] - Vec3::X).length() < 1e-3);
    }
}
