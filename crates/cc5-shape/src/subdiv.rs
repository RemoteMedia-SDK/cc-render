//! Drive CC5's HD mesh (subdivision level 1 or 2) from a level-0 slider pack.
//!
//! CC5 HD characters render a Catmull-Clark subdivision of the base mesh plus
//! per-character sculpted detail. Subdivision is linear, so an HD vertex moves
//! by a fixed weighted sum of base-vertex moves: `hd_delta = S · base_delta`.
//! `build_hd_map.py` writes `S` per subdivided mesh, rows in CC5's HD vertex
//! order (= the HD character GLB's `_CCIDX`):
//!
//! - `hd_map.json` — `{"level", "vertex_count", "base_vertex_count", "meshes":
//!   [{"name", "offset", "count", "base_offset", "base_count", "stencil":
//!   [byte_offset, nnz] | null}]}`; meshes in the pack's order, `offset`s in the
//!   HD global vertex space;
//! - `hd_map.bin` — per stencil: `u32 indptr[count + 1]`, `u32 base_index[nnz]`
//!   (mesh-local), `f32 weight[nnz]`.
//!
//! Meshes without a stencil (hair, brows, lashes, clothes) map 1:1. The sculpted
//! detail is already in the HD GLB's bind positions. Sliders that only reshape
//! the base mesh are reproduced exactly (p99 ≤ 0.02 mm vs CC5); HD presets and
//! large body sliders also change the HD level directly, which a level-0 pack
//! cannot see (p99 up to ~0.8 mm, max ~3 mm).

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::pack::{ShapeDelta, ShapePack};

#[derive(Debug, Clone, Deserialize)]
pub struct HdMesh {
    pub name: String,
    pub offset: u32,
    pub count: u32,
    pub base_offset: u32,
    pub base_count: u32,
    stencil: Option<[u64; 2]>,
}

#[derive(Deserialize)]
struct MapJson {
    level: u32,
    vertex_count: usize,
    base_vertex_count: usize,
    meshes: Vec<HdMesh>,
}

/// CSR rows = HD vertices of one mesh, columns = its base vertices.
struct Stencil {
    indptr: Vec<u32>,
    indices: Vec<u32>,
    weights: Vec<f32>,
}

pub struct SubdivMap {
    /// CC5 subdivision level of the HD mesh (1 or 2).
    pub level: u32,
    /// HD global vertex count.
    pub vertex_count: usize,
    /// The slider pack's (level 0) global vertex count.
    pub base_vertex_count: usize,
    pub meshes: Vec<HdMesh>,
    stencils: Vec<Option<Stencil>>,
}

fn u32s(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

impl SubdivMap {
    pub fn load(dir: &Path) -> Result<Self> {
        let json_path = dir.join("hd_map.json");
        let j: MapJson = serde_json::from_slice(
            &std::fs::read(&json_path).with_context(|| format!("reading {}", json_path.display()))?,
        )
        .with_context(|| format!("parsing {}", json_path.display()))?;
        let bin_path = dir.join("hd_map.bin");
        let bin = std::fs::read(&bin_path).with_context(|| format!("reading {}", bin_path.display()))?;
        let mut stencils = Vec::with_capacity(j.meshes.len());
        for m in &j.meshes {
            let Some([off, nnz]) = m.stencil else {
                if m.count != m.base_count {
                    bail!("{}: {} HD vs {} base vertices but no stencil", m.name, m.count, m.base_count);
                }
                stencils.push(None);
                continue;
            };
            let (off, nnz, rows) = (off as usize, nnz as usize, m.count as usize);
            let end = off + 4 * (rows + 1) + 8 * nnz;
            if end > bin.len() {
                bail!("{}: stencil past end of hd_map.bin", m.name);
            }
            let indptr = u32s(&bin[off..off + 4 * (rows + 1)]);
            let i0 = off + 4 * (rows + 1);
            let indices = u32s(&bin[i0..i0 + 4 * nnz]);
            let weights = bin[i0 + 4 * nnz..end].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
            if indptr.last().copied() != Some(nnz as u32) || indptr.windows(2).any(|w| w[0] > w[1]) {
                bail!("{}: malformed stencil row pointers", m.name);
            }
            if let Some(bad) = indices.iter().find(|&&i| i >= m.base_count) {
                bail!("{}: stencil column {bad} out of range {}", m.name, m.base_count);
            }
            stencils.push(Some(Stencil { indptr, indices, weights }));
        }
        let end = j.meshes.iter().map(|m| (m.offset + m.count) as usize).max().unwrap_or(0);
        if end > j.vertex_count {
            bail!("hd_map.json: meshes extend past vertex_count");
        }
        Ok(Self { level: j.level, vertex_count: j.vertex_count, base_vertex_count: j.base_vertex_count, meshes: j.meshes, stencils })
    }

    /// Error unless this map was built for `pack`'s vertex space.
    pub fn check(&self, pack: &ShapePack) -> Result<()> {
        if self.base_vertex_count != pack.vertex_count {
            bail!("HD map is for {} base vertices, pack has {}", self.base_vertex_count, pack.vertex_count);
        }
        for m in &self.meshes {
            match pack.meshes.iter().find(|p| p.name == m.name) {
                Some(p) if p.offset == m.base_offset && p.count == m.base_count => {}
                _ => bail!("HD map mesh {} does not match the pack", m.name),
            }
        }
        Ok(())
    }

    /// HD global vertex offset of a mesh (by glTF mesh name).
    pub fn mesh_offset(&self, name: &str) -> Option<u32> {
        self.meshes.iter().find(|m| m.name == name).map(|m| m.offset)
    }

    /// Per-vertex values (deltas) in the base space -> the HD space.
    pub fn expand_verts(&self, base: &[[f32; 3]]) -> Vec<[f32; 3]> {
        assert_eq!(base.len(), self.base_vertex_count, "one entry per base vertex");
        let mut out = vec![[0.0; 3]; self.vertex_count];
        for (m, st) in self.meshes.iter().zip(&self.stencils) {
            let src = &base[m.base_offset as usize..(m.base_offset + m.base_count) as usize];
            let dst = &mut out[m.offset as usize..(m.offset + m.count) as usize];
            match st {
                None => dst.copy_from_slice(src),
                Some(s) => {
                    for (r, d) in dst.iter_mut().enumerate() {
                        let (a, b) = (s.indptr[r] as usize, s.indptr[r + 1] as usize);
                        let mut acc = [0.0f32; 3];
                        for k in a..b {
                            let (w, v) = (s.weights[k], src[s.indices[k] as usize]);
                            if v != [0.0; 3] {
                                acc[0] += w * v[0];
                                acc[1] += w * v[1];
                                acc[2] += w * v[2];
                            }
                        }
                        *d = acc;
                    }
                }
            }
        }
        out
    }

    /// A base-space [`ShapeDelta`] (from `ShapePack::evaluate`) in the HD space;
    /// joints are unchanged.
    pub fn expand(&self, d: &ShapeDelta) -> ShapeDelta {
        ShapeDelta { verts: self.expand_verts(&d.verts), joints: d.joints.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mesh "A" (2 verts) maps 1:1; mesh "B" (2 base verts) subdivides into 3.
    fn map(dir: &Path) -> SubdivMap {
        let mut bin = Vec::new();
        for v in [0u32, 1, 2, 4] {
            bin.extend(v.to_le_bytes()); // indptr: rows {0}, {1}, {0, 1}
        }
        for v in [0u32, 1, 0, 1] {
            bin.extend(v.to_le_bytes());
        }
        for w in [1.0f32, 1.0, 0.5, 0.5] {
            bin.extend(w.to_le_bytes());
        }
        std::fs::write(dir.join("hd_map.bin"), bin).unwrap();
        std::fs::write(
            dir.join("hd_map.json"),
            r#"{"level": 1, "vertex_count": 5, "base_vertex_count": 4, "meshes": [
                {"name": "A", "offset": 0, "count": 2, "base_offset": 0, "base_count": 2, "stencil": null},
                {"name": "B", "offset": 2, "count": 3, "base_offset": 2, "base_count": 2, "stencil": [0, 4]}]}"#,
        )
        .unwrap();
        SubdivMap::load(dir).unwrap()
    }

    #[test]
    fn expands_identity_and_stencil_meshes() {
        let dir = tempfile::tempdir().unwrap();
        let m = map(dir.path());
        let out = m.expand_verts(&[[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 4.0, 2.0]]);
        assert_eq!(out, vec![[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 4.0, 2.0], [0.0, 3.0, 1.0]]);
        assert_eq!(m.mesh_offset("B"), Some(2));
    }

    #[test]
    fn rejects_out_of_range_columns() {
        let dir = tempfile::tempdir().unwrap();
        map(dir.path());
        let mut bin = std::fs::read(dir.path().join("hd_map.bin")).unwrap();
        bin[16..20].copy_from_slice(&7u32.to_le_bytes());
        std::fs::write(dir.path().join("hd_map.bin"), bin).unwrap();
        let err = SubdivMap::load(dir.path()).err().unwrap().to_string();
        assert!(err.contains("out of range"), "{err}");
    }
}
