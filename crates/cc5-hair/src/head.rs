//! Nearest-point queries on the body (and on a source's scalp layers), and the
//! body's skin weights at a point (hair_build.py `Head`).

use anyhow::{anyhow, Result};
use glam::Vec3;
use parry3d::na::Point3;
use parry3d::query::PointQueryWithLocation;
use parry3d::shape::TriMesh;

use crate::source::Source;

/// A triangle surface with nearest-point queries.
pub struct Surface {
    pub pos: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    mesh: TriMesh,
}

/// The nearest surface point to a query point.
#[derive(Debug, Clone, Copy)]
pub struct Hit {
    pub point: Vec3,
    /// Unit face normal (by winding).
    pub normal: Vec3,
    pub dist: f32,
    pub tri: u32,
}

impl Surface {
    pub fn new(pos: Vec<Vec3>, tris: Vec<[u32; 3]>) -> Option<Self> {
        if tris.is_empty() {
            return None;
        }
        let mesh = TriMesh::new(pos.iter().map(|p| Point3::new(p.x, p.y, p.z)).collect(), tris.clone());
        Some(Self { pos, tris, mesh })
    }

    pub fn nearest(&self, p: Vec3) -> Hit {
        let (proj, (tri, _)) = self.mesh.project_local_point_and_get_location(&Point3::new(p.x, p.y, p.z), false);
        let point = Vec3::new(proj.point.x, proj.point.y, proj.point.z);
        let [a, b, c] = self.tris[tri as usize].map(|i| self.pos[i as usize]);
        let normal = (b - a).cross(c - a).normalize_or_zero();
        Hit { point, normal, dist: point.distance(p), tri }
    }
}

/// The body (`CC_Base_Body`) of the head GLB: nearest point and skin weights.
pub struct Body {
    pub surface: Surface,
    /// Per vertex: (joint index into `joints`, weight) above 1e-4.
    weights: Vec<Vec<(u16, f32)>>,
    pub joints: Vec<String>,
}

impl Body {
    pub fn from_source(src: &Source) -> Result<Self> {
        let obj = src.object("CC_Base_Body").ok_or_else(|| anyhow!("{}: no CC_Base_Body", src.path.display()))?;
        let (mut pos, mut tris, mut weights) = (Vec::new(), Vec::new(), Vec::new());
        for p in &obj.prims {
            let o = pos.len() as u32;
            pos.extend(&p.pos);
            tris.extend(p.tris.iter().map(|t| t.map(|i| i + o)));
            weights.extend(p.joints.iter().zip(&p.weights).map(|(j, w)| {
                (0..4).filter(|&k| w[k] > 1e-4).map(|k| (j[k], w[k])).collect::<Vec<_>>()
            }));
        }
        let surface = Surface::new(pos, tris).ok_or_else(|| anyhow!("CC_Base_Body has no triangles"))?;
        Ok(Self { surface, weights, joints: src.skin_joints() })
    }

    pub fn nearest(&self, p: Vec3) -> Hit {
        self.surface.nearest(p)
    }

    /// The body's weights at the surface point nearest `p` (barycentric blend of
    /// the triangle's vertices), top 4, normalised.
    pub fn skin_at(&self, p: Vec3) -> [(u16, f32); 4] {
        let h = self.nearest(p);
        let t = self.surface.tris[h.tri as usize];
        let [a, b, c] = t.map(|i| self.surface.pos[i as usize]);
        let bary = barycentric(h.point, a, b, c);
        let mut acc: Vec<(u16, f32)> = Vec::new();
        for (vi, bw) in t.iter().zip(bary) {
            for &(j, w) in &self.weights[*vi as usize] {
                match acc.iter_mut().find(|e| e.0 == j) {
                    Some(e) => e.1 += bw.max(0.0) * w,
                    None => acc.push((j, bw.max(0.0) * w)),
                }
            }
        }
        acc.sort_by(|x, y| y.1.total_cmp(&x.1));
        acc.truncate(4);
        let s: f32 = acc.iter().map(|e| e.1).sum::<f32>().max(1e-12);
        let mut out = [(0u16, 0.0f32); 4];
        for (o, e) in out.iter_mut().zip(&acc) {
            *o = (e.0, e.1 / s);
        }
        if acc.is_empty() {
            out[0].1 = 1.0;
        }
        out
    }
}

/// Barycentric weights of `p` (on the triangle's plane) in triangle `a b c`.
pub fn barycentric(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> [f32; 3] {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-20 {
        return [1.0, 0.0, 0.0];
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    [1.0 - v - w, v, w]
}
