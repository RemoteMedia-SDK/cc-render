//! CC5 facial expressions (morph targets) on the CPU: load an expression pack and
//! evaluate a weighted mix into per-vertex deltas.
//!
//! A CC5 HD character has ~400 facial morphs per face mesh; Bevy loads at most 64
//! morph targets per mesh, so they are kept out of the GLB (`fbx_to_glb.py` drops
//! them) and applied here instead, like the shaping sliders. The pack is built
//! from the reference FBX's shape keys by `build_expression_pack.py`:
//!
//! - `expressions.json` — `vertex_count` and `meshes` (as in `pack.json`), and per
//!   expression its name, the meshes it moves and its delta block;
//! - `expressions.bin` — per block: `u32 index[count]` then `f32 delta[count * 3]`.
//!
//! Deltas are in the slider pack's global vertex space (`global = mesh.offset +
//! _CCIDX`), glTF frame (Y-up, meters), at weight 1. One expression covers every
//! mesh that has a morph of that name (body, brows, lashes, tear line, eye
//! occlusion, eyes, tongue), so a mix is `base + Σ wᵢ·Δᵢ` for the whole head.
//! Morph targets are linear, so this is exact; only nonzero weights cost anything.
//!
//! [`ArkitMap`] resolves ARKit-52 weights (Audio2Face) to expression weights.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::pack::{add_block, block_deltas, block_indices, PackMesh};

#[derive(Debug, Clone, Deserialize)]
pub struct Expression {
    pub name: String,
    /// Meshes this expression moves (glTF mesh names).
    pub meshes: Vec<String>,
    /// `[byte_offset, count]` into `expressions.bin`.
    pub verts: [u64; 2],
}

#[derive(Debug, Deserialize)]
struct ExprJson {
    version: u32,
    vertex_count: usize,
    meshes: Vec<PackMesh>,
    expressions: Vec<Expression>,
}

pub struct ExpressionPack {
    pub vertex_count: usize,
    pub meshes: Vec<PackMesh>,
    pub expressions: Vec<Expression>,
    by_name: HashMap<String, usize>,
    bin: Vec<u8>,
}

impl ExpressionPack {
    pub fn load(dir: &Path) -> Result<Self> {
        let json_path = dir.join("expressions.json");
        let pack: ExprJson = serde_json::from_slice(
            &std::fs::read(&json_path).with_context(|| format!("reading {}", json_path.display()))?,
        )
        .with_context(|| format!("parsing {}", json_path.display()))?;
        if pack.version != 1 {
            bail!("unsupported expression pack version {}", pack.version);
        }
        let bin_path = dir.join("expressions.bin");
        let bin = std::fs::read(&bin_path).with_context(|| format!("reading {}", bin_path.display()))?;
        let by_name = pack.expressions.iter().enumerate().map(|(i, e)| (e.name.clone(), i)).collect();
        let me = Self { vertex_count: pack.vertex_count, meshes: pack.meshes, expressions: pack.expressions, by_name, bin };
        me.validate()?;
        Ok(me)
    }

    fn validate(&self) -> Result<()> {
        if self.by_name.len() != self.expressions.len() {
            bail!("duplicate expression names");
        }
        for e in &self.expressions {
            let [off, n] = e.verts;
            if (off + n * 16) as usize > self.bin.len() {
                bail!("{}: block past end of expressions.bin", e.name);
            }
            if let Some(bad) = block_indices(&self.bin, off, n).find(|&i| i as usize >= self.vertex_count) {
                bail!("{}: index {bad} out of range {}", e.name, self.vertex_count);
            }
        }
        Ok(())
    }

    /// Number of expressions.
    pub fn len(&self) -> usize {
        self.expressions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.expressions.is_empty()
    }

    /// Expression names, in pack order (index = position).
    pub fn names(&self) -> impl Iterator<Item = &str> + '_ {
        self.expressions.iter().map(|e| e.name.as_str())
    }

    /// Index of the expression with this name.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    /// Index-weight pairs from `{name: weight}`; unknown names are an error.
    pub fn weights_from<'a>(&self, set: impl IntoIterator<Item = (&'a str, f32)>) -> Result<Vec<(usize, f32)>> {
        set.into_iter()
            .map(|(k, w)| Ok((self.find(k).with_context(|| format!("no expression named {k:?}"))?, w)))
            .collect()
    }

    /// Sparse delta of expression `i` at weight 1: global vertex indices and deltas.
    pub fn deltas(&self, i: usize) -> (Vec<u32>, Vec<[f32; 3]>) {
        let [off, n] = self.expressions[i].verts;
        (block_indices(&self.bin, off, n).collect(), block_deltas(&self.bin, off, n).collect())
    }

    /// `out[v] += Σ wᵢ·Δᵢ[v]` over `(expression index, weight)` pairs; zero weights
    /// are skipped. `out` has one entry per global vertex (e.g. a [`ShapeDelta`]'s
    /// `verts`, to layer expressions on a shaped character).
    ///
    /// [`ShapeDelta`]: crate::pack::ShapeDelta
    pub fn add_to(&self, weights: &[(usize, f32)], out: &mut [[f32; 3]]) {
        assert_eq!(out.len(), self.vertex_count, "one entry per global vertex");
        for &(i, w) in weights {
            if w != 0.0 {
                add_block(&self.bin, self.expressions[i].verts, w, out);
            }
        }
    }

    /// Per global vertex delta (glTF frame, meters) for `(expression index, weight)` pairs.
    pub fn evaluate(&self, weights: &[(usize, f32)]) -> Vec<[f32; 3]> {
        let mut out = vec![[0.0; 3]; self.vertex_count];
        self.add_to(weights, &mut out);
        out
    }
}

/// ARKit-52 blendshape names -> expression weights, from a runtime ARKit map
/// (`avatars/*.arkit_map*.json`):
/// `{"mapping": {arkit_name: [{"morph": name, "weight": scale, "meshes": [...]}]}}`.
/// Each ARKit weight drives every listed morph, scaled; `meshes` is ignored since
/// an expression already covers every mesh with that morph.
pub struct ArkitMap {
    /// ARKit name -> `(expression index, scale)`.
    pub mapping: HashMap<String, Vec<(usize, f32)>>,
    /// Morph names the map uses that the expression pack does not have.
    pub missing: Vec<String>,
}

#[derive(Deserialize)]
struct ArkitTarget {
    morph: String,
    #[serde(default = "one")]
    weight: f32,
}

fn one() -> f32 {
    1.0
}

impl ArkitMap {
    pub fn load(path: &Path, pack: &ExpressionPack) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("parsing {}", path.display()))?;
        Self::from_json(&v, pack)
    }

    pub fn from_json(v: &serde_json::Value, pack: &ExpressionPack) -> Result<Self> {
        let raw: HashMap<String, Vec<ArkitTarget>> =
            serde_json::from_value(v.get("mapping").context("ARKit map has no \"mapping\"")?.clone())?;
        let (mut mapping, mut missing) = (HashMap::new(), Vec::new());
        for (arkit, targets) in raw {
            let mut resolved = Vec::new();
            for t in targets {
                match pack.find(&t.morph) {
                    Some(i) => resolved.push((i, t.weight)),
                    None => missing.push(t.morph),
                }
            }
            mapping.insert(arkit, resolved);
        }
        missing.sort();
        missing.dedup();
        Ok(Self { mapping, missing })
    }

    /// Expression weights for ARKit weights (summed where morphs are shared);
    /// unknown ARKit names are an error.
    pub fn weights<'a>(&self, arkit: impl IntoIterator<Item = (&'a str, f32)>) -> Result<Vec<(usize, f32)>> {
        let mut acc: Vec<(usize, f32)> = Vec::new();
        for (name, w) in arkit {
            let targets = self.mapping.get(name).with_context(|| format!("no ARKit blendshape {name:?} in the map"))?;
            for &(i, s) in targets {
                match acc.iter_mut().find(|(j, _)| *j == i) {
                    Some(e) => e.1 += w * s,
                    None => acc.push((i, w * s)),
                }
            }
        }
        Ok(acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(bin: &mut Vec<u8>, idx: &[u32], d: &[[f32; 3]]) -> [u64; 2] {
        let off = bin.len() as u64;
        for i in idx {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        for v in d {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        [off, idx.len() as u64]
    }

    fn write_pack(dir: &Path) {
        let mut bin = Vec::new();
        let blink = block(&mut bin, &[0, 3], &[[0.0, -0.01, 0.0], [0.0, -0.02, 0.0]]);
        let jaw = block(&mut bin, &[1, 3], &[[0.0, -0.1, 0.0], [0.0, 0.0, 0.04]]);
        let none = block(&mut bin, &[], &[]);
        std::fs::write(dir.join("expressions.bin"), &bin).unwrap();
        let json = serde_json::json!({
            "version": 1, "vertex_count": 4,
            "meshes": [{"name": "Body", "offset": 0, "count": 3}, {"name": "Lash", "offset": 3, "count": 1}],
            "expressions": [
                {"name": "Eye_Blink_L", "meshes": ["Body", "Lash"], "verts": blink},
                {"name": "Jaw_Open", "meshes": ["Body", "Lash"], "verts": jaw},
                {"name": "V_None", "meshes": [], "verts": none}
            ]
        });
        std::fs::write(dir.join("expressions.json"), serde_json::to_vec(&json).unwrap()).unwrap();
    }

    #[test]
    fn evaluates_sparse_weighted_sum() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path());
        let p = ExpressionPack::load(dir.path()).unwrap();
        assert_eq!(p.names().collect::<Vec<_>>(), ["Eye_Blink_L", "Jaw_Open", "V_None"]);
        assert_eq!(p.find("Jaw_Open"), Some(1));
        assert_eq!(p.find("Nope"), None);
        assert_eq!(p.evaluate(&[]), vec![[0.0; 3]; 4]);

        let d = p.evaluate(&[(0, 1.0), (1, 0.5), (2, 1.0)]);
        assert_eq!(d[0], [0.0, -0.01, 0.0]);
        assert_eq!(d[1], [0.0, -0.05, 0.0]);
        assert_eq!(d[2], [0.0; 3]);
        assert_eq!(d[3], [0.0, -0.02, 0.02]);

        // Layered on an existing (shape) delta.
        let mut out = vec![[1.0, 0.0, 0.0]; 4];
        p.add_to(&p.weights_from([("Jaw_Open", 1.0)]).unwrap(), &mut out);
        assert_eq!(out[1], [1.0, -0.1, 0.0]);
        assert!(p.weights_from([("Nope", 1.0)]).unwrap_err().to_string().contains("no expression"));
        assert_eq!(p.deltas(0), (vec![0, 3], vec![[0.0, -0.01, 0.0], [0.0, -0.02, 0.0]]));
    }

    #[test]
    fn resolves_arkit_weights() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path());
        let p = ExpressionPack::load(dir.path()).unwrap();
        let map = ArkitMap::from_json(
            &serde_json::json!({"mapping": {
                "eyeBlinkLeft": [{"morph": "Eye_Blink_L", "weight": 1.0, "meshes": ["Body"]}],
                "jawOpen": [{"morph": "Jaw_Open", "weight": 0.5}, {"morph": "Eye_Blink_L", "weight": 0.25}],
                "mouthClose": [{"morph": "Mouth_Close", "weight": 1.0}]
            }}),
            &p,
        )
        .unwrap();
        assert_eq!(map.missing, ["Mouth_Close"]);
        let w = map.weights([("eyeBlinkLeft", 0.5), ("jawOpen", 1.0), ("mouthClose", 1.0)]).unwrap();
        assert_eq!(w, vec![(0, 0.75), (1, 0.5)]);
        assert!(map.weights([("tongueOut", 1.0)]).is_err());
    }

    #[test]
    fn rejects_out_of_range_index() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path());
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("expressions.json")).unwrap()).unwrap();
        json["vertex_count"] = 3.into();
        std::fs::write(dir.path().join("expressions.json"), serde_json::to_vec(&json).unwrap()).unwrap();
        let err = ExpressionPack::load(dir.path()).err().unwrap().to_string();
        assert!(err.contains("out of range"), "{err}");
    }
}
