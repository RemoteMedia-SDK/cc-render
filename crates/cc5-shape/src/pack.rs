//! CC5 shaping sliders outside CC5: load an editor pack and evaluate a slider mix.
//!
//! The pack is built from a CC5 capture (`cc5_capture_sliders.py`, MODE "bind")
//! by `build_editor_pack.py`:
//!
//! - `pack.json` — slider tree (id, name, category, range, base weight) and, per
//!   slider, the captured min/max states as byte ranges into `pack.bin`;
//! - `pack.bin` — per block: `u32 index[count]` then `f32 delta[count * 3]`.
//!
//! Vertex deltas are in one global index space (CC5 bind-pose mesh order, meshes
//! concatenated); a GLB vertex maps to it through its `_CCIDX` attribute:
//! `global = mesh.offset + _CCIDX`. Joint deltas index `joints`. Everything is in
//! the glTF frame (Y-up, meters) and describes the *bind pose*: skeletal stretch
//! (e.g. Character Height) is already baked into the vertex deltas, so applying a
//! mix is `bind + Σ t_i · delta_i` for the mesh and the joints alike.
//!
//! CC5 sliders are linear on each side of their base weight (verified in CC5: a
//! slider at half weight and two sliders combined both reproduce the sum of the
//! captured deltas exactly), so a state captured at `weight` scales by
//! `(w - base) / (weight - base)`.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PackMesh {
    pub name: String,
    pub offset: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackState {
    pub weight: f32,
    /// `[byte_offset, count]` into `pack.bin`.
    pub verts: [u64; 2],
    pub joints: [u64; 2],
}

#[derive(Debug, Clone, Deserialize)]
pub struct Slider {
    pub id: String,
    pub name: String,
    pub category: String,
    pub min: f32,
    pub max: f32,
    /// Weight the character was captured at (its design value).
    pub base: f32,
    pub states: HashMap<String, PackState>,
}

#[derive(Debug, Deserialize)]
struct PackJson {
    version: u32,
    vertex_count: usize,
    meshes: Vec<PackMesh>,
    joints: Vec<String>,
    sliders: Vec<Slider>,
}

/// One captured slider state, see [`ShapePack::state`].
#[derive(Debug, Clone, PartialEq)]
pub struct StateDeltas {
    pub weight: f32,
    pub vert_index: Vec<u32>,
    pub vert_delta: Vec<[f32; 3]>,
    pub joint_index: Vec<u32>,
    pub joint_delta: Vec<[f32; 3]>,
}

/// Summed deltas for a slider mix, in the glTF frame (meters).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeDelta {
    /// One entry per global vertex.
    pub verts: Vec<[f32; 3]>,
    /// One entry per `ShapePack::joints` entry.
    pub joints: Vec<[f32; 3]>,
}

pub struct ShapePack {
    pub vertex_count: usize,
    pub meshes: Vec<PackMesh>,
    pub joints: Vec<String>,
    pub sliders: Vec<Slider>,
    bin: Vec<u8>,
}

impl ShapePack {
    pub fn load(dir: &Path) -> Result<Self> {
        let json_path = dir.join("pack.json");
        let pack: PackJson = serde_json::from_slice(
            &std::fs::read(&json_path).with_context(|| format!("reading {}", json_path.display()))?,
        )
        .with_context(|| format!("parsing {}", json_path.display()))?;
        if pack.version != 1 {
            bail!("unsupported pack version {}", pack.version);
        }
        let bin_path = dir.join("pack.bin");
        let bin = std::fs::read(&bin_path).with_context(|| format!("reading {}", bin_path.display()))?;
        let me = Self {
            vertex_count: pack.vertex_count,
            meshes: pack.meshes,
            joints: pack.joints,
            sliders: pack.sliders,
            bin,
        };
        me.validate()?;
        Ok(me)
    }

    fn validate(&self) -> Result<()> {
        for s in &self.sliders {
            for (which, st) in &s.states {
                for (what, [off, n], limit) in [
                    ("verts", st.verts, self.vertex_count),
                    ("joints", st.joints, self.joints.len()),
                ] {
                    let end = off + n * 16;
                    if end as usize > self.bin.len() {
                        bail!("{} {which} {what}: block past end of pack.bin", s.name);
                    }
                    if let Some(bad) = self.indices(off, n).find(|&i| i as usize >= limit) {
                        bail!("{} {which} {what}: index {bad} out of range {limit}", s.name);
                    }
                }
            }
        }
        Ok(())
    }

    /// Global vertex offset of a mesh (by glTF mesh name).
    pub fn mesh_offset(&self, name: &str) -> Option<u32> {
        self.meshes.iter().find(|m| m.name == name).map(|m| m.offset)
    }

    /// Base weights: the character exactly as captured.
    pub fn base_weights(&self) -> Vec<f32> {
        self.sliders.iter().map(|s| s.base).collect()
    }

    /// Index of the slider with this id, or with this display name if that is unique.
    pub fn find(&self, key: &str) -> Result<usize> {
        if let Some(i) = self.sliders.iter().position(|s| s.id == key) {
            return Ok(i);
        }
        let hits: Vec<usize> = (0..self.sliders.len()).filter(|&i| self.sliders[i].name == key).collect();
        match hits.as_slice() {
            [i] => Ok(*i),
            [] => bail!("no slider named {key:?}"),
            _ => bail!(
                "{key:?} names {} sliders ({}); use an id",
                hits.len(),
                hits.iter().map(|&i| self.sliders[i].category.as_str()).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    /// Full weight vector: base weights with the given sliders (id or unique name) set.
    pub fn weights_from<'a>(&self, set: impl IntoIterator<Item = (&'a str, f32)>) -> Result<Vec<f32>> {
        let mut w = self.base_weights();
        for (key, value) in set {
            let i = self.find(key)?;
            let s = &self.sliders[i];
            if !(s.min..=s.max).contains(&value) {
                bail!("{key}: {value} outside {}..{}", s.min, s.max);
            }
            w[i] = value;
        }
        Ok(w)
    }

    /// A recipe for `weights`: the sliders that differ from the captured design, by id.
    /// `{"format": "cc5-shape recipe", "version": 1, "sliders": {id: value}}`
    pub fn recipe(&self, weights: &[f32]) -> serde_json::Value {
        self.recipe_on(&self.base_weights(), weights)
    }

    /// A recipe for `weights` made on top of `base` (e.g. a CC5 neutral body):
    /// `"base"` holds the sliders where `base` differs from the captured design,
    /// `"sliders"` the changes on top of it. Absent when `base` is the captured design.
    pub fn recipe_on(&self, base: &[f32], weights: &[f32]) -> serde_json::Value {
        let diff = |from: &[f32], to: &[f32]| -> serde_json::Map<String, serde_json::Value> {
            self.sliders
                .iter()
                .zip(from.iter().zip(to))
                .filter(|(_, (a, b))| (**a - **b).abs() > 1e-6)
                .map(|(s, (_, b))| (s.id.clone(), serde_json::json!(b)))
                .collect()
        };
        let mut r = serde_json::json!({"format": "cc5-shape recipe", "version": 1, "sliders": diff(base, weights)});
        let b = diff(&self.base_weights(), base);
        if !b.is_empty() {
            r["base"] = serde_json::Value::Object(b);
        }
        r
    }

    /// Weights from a recipe (`recipe()` / `recipe_on()` output: its `"base"`, then its
    /// `"sliders"`; or a plain `{id or name: value}` map). Unknown sliders are an
    /// error: the recipe was made for a different pack.
    pub fn weights_from_recipe(&self, recipe: &serde_json::Value) -> Result<Vec<f32>> {
        let mut set = Vec::new();
        let maps = [recipe.get("base"), Some(recipe.get("sliders").unwrap_or(recipe))];
        for map in maps.into_iter().flatten() {
            for (k, v) in map.as_object().context("recipe is not a JSON object")? {
                set.push((k.as_str(), v.as_f64().with_context(|| format!("{k}: not a number"))? as f32));
            }
        }
        self.weights_from(set)
    }

    /// Raw captured state `which` ("min" or "max") of slider `i`: its weight, and the
    /// sparse vertex and joint deltas at that weight (relative to the base weight).
    /// For building solvers on top of the linear model (e.g. fitting to a target).
    pub fn state(&self, i: usize, which: &str) -> Option<StateDeltas> {
        let st = self.sliders.get(i)?.states.get(which)?;
        let [vo, vn] = st.verts;
        let [jo, jn] = st.joints;
        Some(StateDeltas {
            weight: st.weight,
            vert_index: self.indices(vo, vn).collect(),
            vert_delta: self.deltas(vo, vn).collect(),
            joint_index: self.indices(jo, jn).collect(),
            joint_delta: self.deltas(jo, jn).collect(),
        })
    }

    fn indices(&self, off: u64, n: u64) -> impl Iterator<Item = u32> + '_ {
        block_indices(&self.bin, off, n)
    }

    fn deltas(&self, off: u64, n: u64) -> impl Iterator<Item = [f32; 3]> + '_ {
        block_deltas(&self.bin, off, n)
    }

    fn add_block(&self, block: [u64; 2], t: f32, out: &mut [[f32; 3]]) {
        add_block(&self.bin, block, t, out);
    }

    /// Captured state and scale for slider `i` at weight `w`, if it moves at all.
    fn state_for(&self, i: usize, w: f32) -> Option<(&PackState, f32)> {
        let s = &self.sliders[i];
        let d = w - s.base;
        if d.abs() < 1e-6 {
            return None;
        }
        let st = s.states.get(if d > 0.0 { "max" } else { "min" })?;
        let span = st.weight - s.base;
        (span.abs() > 1e-9).then(|| (st, d / span))
    }

    /// Sum of deltas for `weights` (one per slider, in slider order).
    pub fn evaluate(&self, weights: &[f32]) -> ShapeDelta {
        assert_eq!(weights.len(), self.sliders.len(), "one weight per slider");
        let mut out = ShapeDelta {
            verts: vec![[0.0; 3]; self.vertex_count],
            joints: vec![[0.0; 3]; self.joints.len()],
        };
        for (i, &w) in weights.iter().enumerate() {
            if let Some((st, t)) = self.state_for(i, w) {
                self.add_block(st.verts, t, &mut out.verts);
                self.add_block(st.joints, t, &mut out.joints);
            }
        }
        out
    }

    /// Sliders grouped by category path, sorted, for building a UI tree.
    pub fn categories(&self) -> Vec<(String, Vec<usize>)> {
        let mut by: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, s) in self.sliders.iter().enumerate() {
            by.entry(s.category.as_str()).or_default().push(i);
        }
        let mut out: Vec<(String, Vec<usize>)> = by
            .into_iter()
            .map(|(c, mut v)| {
                v.sort_by(|&a, &b| self.sliders[a].name.cmp(&self.sliders[b].name));
                (c.to_string(), v)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

// Sparse delta blocks (`pack.bin`, `expressions.bin`): `u32 index[n]` then
// `f32 delta[n * 3]`, starting at byte `off`. Bounds are checked at load.

pub(crate) fn block_indices(bin: &[u8], off: u64, n: u64) -> impl Iterator<Item = u32> + '_ {
    let start = off as usize;
    (0..n as usize).map(move |i| {
        let b = start + 4 * i;
        u32::from_le_bytes(bin[b..b + 4].try_into().unwrap())
    })
}

pub(crate) fn block_deltas(bin: &[u8], off: u64, n: u64) -> impl Iterator<Item = [f32; 3]> + '_ {
    let start = (off + 4 * n) as usize;
    (0..n as usize).map(move |i| {
        let b = start + 12 * i;
        let f = |k: usize| f32::from_le_bytes(bin[b + 4 * k..b + 4 * k + 4].try_into().unwrap());
        [f(0), f(1), f(2)]
    })
}

/// `out[index] += t * delta` for one block.
pub(crate) fn add_block(bin: &[u8], [off, n]: [u64; 2], t: f32, out: &mut [[f32; 3]]) {
    for (i, d) in block_indices(bin, off, n).zip(block_deltas(bin, off, n)) {
        let o = &mut out[i as usize];
        o[0] += t * d[0];
        o[1] += t * d[1];
        o[2] += t * d[2];
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
        let h_max_v = block(&mut bin, &[0, 2], &[[0.0, -0.1, 0.0], [0.0, -0.2, 0.0]]);
        let h_max_j = block(&mut bin, &[1], &[[0.0, -0.1, 0.0]]);
        let h_min_v = block(&mut bin, &[2], &[[0.0, 0.4, 0.0]]);
        let h_min_j = block(&mut bin, &[], &[]);
        let f_max_v = block(&mut bin, &[1], &[[0.05, 0.0, 0.0]]);
        let f_max_j = block(&mut bin, &[], &[]);
        std::fs::write(dir.join("pack.bin"), &bin).unwrap();
        let json = serde_json::json!({
            "version": 1, "vertex_count": 3,
            "meshes": [{"name": "Body", "offset": 0, "count": 3}],
            "joints": ["Hip", "Head"],
            "sliders": [
                {"id": "h", "name": "Height", "category": "Body/Full", "min": -1.0, "max": 1.0, "base": 0.0,
                 "states": {"max": {"weight": 1.0, "verts": h_max_v, "joints": h_max_j},
                            "min": {"weight": -1.0, "verts": h_min_v, "joints": h_min_j}}},
                {"id": "f", "name": "Fat", "category": "Body/Full", "min": 0.0, "max": 1.0, "base": 0.0,
                 "states": {"max": {"weight": 1.0, "verts": f_max_v, "joints": f_max_j}}}
            ]
        });
        std::fs::write(dir.join("pack.json"), serde_json::to_vec(&json).unwrap()).unwrap();
    }

    #[test]
    fn evaluates_linear_mix_per_side() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path());
        let p = ShapePack::load(dir.path()).unwrap();
        assert_eq!(p.evaluate(&p.base_weights()).verts, vec![[0.0; 3]; 3]);

        let d = p.evaluate(&[0.5, 1.0]);
        assert_eq!(d.verts[0], [0.0, -0.05, 0.0]);
        assert_eq!(d.verts[1], [0.05, 0.0, 0.0]);
        assert_eq!(d.verts[2], [0.0, -0.1, 0.0]);
        assert_eq!(d.joints, vec![[0.0; 3], [0.0, -0.05, 0.0]]);

        // Below base uses the min state, scaled by its own span.
        let d = p.evaluate(&[-0.25, 0.0]);
        assert_eq!(d.verts[2], [0.0, 0.1, 0.0]);
        assert_eq!(p.categories(), vec![("Body/Full".to_string(), vec![1, 0])]);
    }

    #[test]
    fn resolves_weights_by_id_or_unique_name() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path());
        let p = ShapePack::load(dir.path()).unwrap();
        assert_eq!(p.weights_from([("Height", 0.5), ("f", 1.0)]).unwrap(), vec![0.5, 1.0]);
        assert!(p.weights_from([("Height", 2.0)]).unwrap_err().to_string().contains("outside"));
        assert!(p.find("Nope").unwrap_err().to_string().contains("no slider"));
    }

    #[test]
    fn recipe_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path());
        let p = ShapePack::load(dir.path()).unwrap();
        let r = p.recipe(&[0.5, 0.0]);
        assert_eq!(r["sliders"], serde_json::json!({"h": 0.5}));
        assert_eq!(p.weights_from_recipe(&r).unwrap(), vec![0.5, 0.0]);
        assert_eq!(p.weights_from_recipe(&serde_json::json!({"Fat": 1.0})).unwrap(), vec![0.0, 1.0]);
        assert!(p.weights_from_recipe(&serde_json::json!({"sliders": {"zz": 1.0}})).is_err());
        // on top of another base: the base is kept apart, and round-trips
        let base = vec![0.0, 1.0];
        let r = p.recipe_on(&base, &[0.5, 1.0]);
        assert_eq!(r["base"], serde_json::json!({p.sliders[1].id.clone(): 1.0}));
        assert_eq!(r["sliders"], serde_json::json!({p.sliders[0].id.clone(): 0.5}));
        assert_eq!(p.weights_from_recipe(&r).unwrap(), vec![0.5, 1.0]);
        assert!(p.recipe_on(&p.base_weights(), &[0.5, 0.0]).get("base").is_none());
    }

    #[test]
    fn rejects_out_of_range_index() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path());
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("pack.json")).unwrap()).unwrap();
        json["vertex_count"] = 2.into();
        std::fs::write(dir.path().join("pack.json"), serde_json::to_vec(&json).unwrap()).unwrap();
        let err = ShapePack::load(dir.path()).err().unwrap().to_string();
        assert!(err.contains("out of range"), "{err}");
    }
}
