//! CPython bindings: `import cc5_shape`.
//!
//! ```python
//! import cc5_shape
//! pack = cc5_shape.Pack("path/to/pack")
//! w = pack.weights({"Character Height": 0.5, "Body Fat A": 0.3})
//! open("shaped.glb", "wb").write(pack.shape_glb(open("character.glb", "rb").read(), w))
//! verts, joints = pack.evaluate(w)   # bytes (f32 xyz per vertex), {joint: (x, y, z)}
//!
//! ex = cc5_shape.Expressions("path/to/expressions")
//! d = ex.evaluate({"Eye_Blink_L": 1.0, "Jaw_Open": 0.3})   # bytes, f32 xyz per vertex
//! arkit = cc5_shape.ArkitMap("beautiful.arkit_map.json", ex)
//! d = ex.evaluate(arkit.weights({"jawOpen": 0.4}))
//!
//! hd = cc5_shape.SubdivMap("path/to/hd_map")      # CC5 HD mesh (subdivision 1 or 2)
//! open("shaped_hd.glb", "wb").write(pack.shape_glb(open("character_hd.glb", "rb").read(), w, hd=hd))
//! ```
use std::collections::HashMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use crate::conform::{Conform, ConformOptions};
use crate::expr::{ArkitMap, ExpressionPack};
use crate::pack::ShapePack;
use crate::subdiv::SubdivMap;

fn err(e: anyhow::Error) -> PyErr {
    PyValueError::new_err(format!("{e:#}"))
}

fn f32_bytes<'py>(py: Python<'py>, v: &[[f32; 3]]) -> Bound<'py, PyBytes> {
    let mut bytes = Vec::with_capacity(v.len() * 12);
    for p in v {
        for c in p {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
    }
    PyBytes::new(py, &bytes)
}

/// A CC5 slider pack (`pack.json` + `pack.bin`).
#[pyclass(name = "Pack", module = "cc5_shape", frozen)]
struct PyPack {
    inner: ShapePack,
}

impl PyPack {
    fn check(&self, weights: &[f32]) -> PyResult<()> {
        if weights.len() != self.inner.sliders.len() {
            return Err(PyValueError::new_err(format!(
                "expected {} weights (one per slider), got {}",
                self.inner.sliders.len(),
                weights.len()
            )));
        }
        Ok(())
    }
}

#[pymethods]
impl PyPack {
    #[new]
    fn new(dir: std::path::PathBuf) -> PyResult<Self> {
        Ok(Self { inner: ShapePack::load(&dir).map_err(err)? })
    }

    /// Number of vertices in the pack's global vertex space.
    #[getter]
    fn vertex_count(&self) -> usize {
        self.inner.vertex_count
    }

    /// Joint names, in the order `evaluate` reports them.
    #[getter]
    fn joints(&self) -> Vec<String> {
        self.inner.joints.clone()
    }

    /// `[(mesh name, global vertex offset, vertex count)]`.
    #[getter]
    fn meshes(&self) -> Vec<(String, u32, u32)> {
        self.inner.meshes.iter().map(|m| (m.name.clone(), m.offset, m.count)).collect()
    }

    /// Sliders as dicts: id, name, category, min, max, base (the captured design value).
    #[getter]
    fn sliders<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        self.inner
            .sliders
            .iter()
            .map(|s| {
                let d = PyDict::new(py);
                d.set_item("id", &s.id)?;
                d.set_item("name", &s.name)?;
                d.set_item("category", &s.category)?;
                d.set_item("min", s.min)?;
                d.set_item("max", s.max)?;
                d.set_item("base", s.base)?;
                Ok(d)
            })
            .collect()
    }

    /// Sliders grouped by category: `[(category, [slider index, ...])]`, sorted.
    fn categories(&self) -> Vec<(String, Vec<usize>)> {
        self.inner.categories()
    }

    /// Index of a slider by id, or by display name when that is unique.
    fn find(&self, key: &str) -> PyResult<usize> {
        self.inner.find(key).map_err(err)
    }

    /// The character as captured: one weight per slider.
    fn base_weights(&self) -> Vec<f32> {
        self.inner.base_weights()
    }

    /// Base weights with `{id or name: value}` applied (range-checked).
    #[pyo3(signature = (set=None))]
    fn weights(&self, set: Option<HashMap<String, f32>>) -> PyResult<Vec<f32>> {
        let set = set.unwrap_or_default();
        self.inner.weights_from(set.iter().map(|(k, &v)| (k.as_str(), v))).map_err(err)
    }

    /// Recipe JSON (str) for a weight vector: only sliders changed from the design, by id.
    fn recipe(&self, weights: Vec<f32>) -> PyResult<String> {
        self.check(&weights)?;
        Ok(serde_json::to_string_pretty(&self.inner.recipe(&weights)).unwrap())
    }

    /// Weights from a recipe JSON string (`recipe()` output or a plain {id or name: value}).
    fn weights_from_recipe(&self, recipe: &str) -> PyResult<Vec<f32>> {
        let v: serde_json::Value = serde_json::from_str(recipe).map_err(|e| err(e.into()))?;
        self.inner.weights_from_recipe(&v).map_err(err)
    }

    /// `(vertex deltas, joint deltas)` for a weight vector, in the glTF frame (meters):
    /// vertex deltas as little-endian f32 bytes (x, y, z per global vertex; e.g.
    /// `numpy.frombuffer(v, "<f4").reshape(-1, 3)`), joint deltas as `{name: (x, y, z)}`
    /// for joints that move.
    fn evaluate<'py>(&self, py: Python<'py>, weights: Vec<f32>) -> PyResult<(Bound<'py, PyBytes>, HashMap<String, (f32, f32, f32)>)> {
        self.check(&weights)?;
        let d = py.detach(|| self.inner.evaluate(&weights));
        let joints = self
            .inner
            .joints
            .iter()
            .zip(&d.joints)
            .filter(|(_, j)| j.iter().any(|c| *c != 0.0))
            .map(|(n, j)| (n.clone(), (j[0], j[1], j[2])))
            .collect();
        Ok((f32_bytes(py, &d.verts), joints))
    }

    /// Raw captured state of slider `i` (`which` = "min" or "max"), or None:
    /// `(weight, vertex indices u32 bytes, vertex deltas f32 xyz bytes,
    ///   joint indices u32 bytes, joint deltas f32 xyz bytes)`, glTF frame (m).
    fn state<'py>(&self, py: Python<'py>, i: usize, which: &str) -> Option<(f32, Bound<'py, PyBytes>, Bound<'py, PyBytes>, Bound<'py, PyBytes>, Bound<'py, PyBytes>)> {
        let s = self.inner.state(i, which)?;
        let u = |v: &[u32]| PyBytes::new(py, &v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
        let f = |v: &[[f32; 3]]| PyBytes::new(py, &v.iter().flatten().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
        Some((s.weight, u(&s.vert_index), f(&s.vert_delta), u(&s.joint_index), f(&s.joint_delta)))
    }

    /// A shaped copy of `glb` (which must carry `_CCIDX`) for a weight vector.
    /// With `hd` (a `SubdivMap`), `glb` is the character's CC5 HD export.
    #[pyo3(signature = (glb, weights, hd=None))]
    fn shape_glb<'py>(&self, py: Python<'py>, glb: &[u8], weights: Vec<f32>, hd: Option<&PySubdivMap>) -> PyResult<Bound<'py, PyBytes>> {
        self.check(&weights)?;
        if let Some(hd) = hd {
            hd.inner.check(&self.inner).map_err(err)?;
        }
        let out = py.detach(|| {
            let d = self.inner.evaluate(&weights);
            match hd {
                Some(hd) => crate::glb::shape_glb_hd(glb, &self.inner, &hd.inner, &hd.inner.expand(&d)),
                None => crate::glb::shape_glb(glb, &self.inner, &d),
            }
        });
        Ok(PyBytes::new(py, &out.map_err(err)?))
    }

    fn __repr__(&self) -> String {
        format!(
            "<cc5_shape.Pack: {} sliders, {} vertices, {} joints>",
            self.inner.sliders.len(),
            self.inner.vertex_count,
            self.inner.joints.len()
        )
    }
}

/// A CC5 expression pack (`expressions.json` + `expressions.bin`): facial morphs
/// as sparse deltas in the slider pack's global vertex space.
#[pyclass(name = "Expressions", module = "cc5_shape", frozen)]
struct PyExpressions {
    inner: ExpressionPack,
}

#[pymethods]
impl PyExpressions {
    #[new]
    fn new(dir: std::path::PathBuf) -> PyResult<Self> {
        Ok(Self { inner: ExpressionPack::load(&dir).map_err(err)? })
    }

    /// Number of vertices in the global vertex space.
    #[getter]
    fn vertex_count(&self) -> usize {
        self.inner.vertex_count
    }

    /// `[(mesh name, global vertex offset, vertex count)]`.
    #[getter]
    fn meshes(&self) -> Vec<(String, u32, u32)> {
        self.inner.meshes.iter().map(|m| (m.name.clone(), m.offset, m.count)).collect()
    }

    /// Expression names, in pack order.
    #[getter]
    fn names(&self) -> Vec<String> {
        self.inner.names().map(str::to_owned).collect()
    }

    /// Index of an expression by name.
    fn find(&self, name: &str) -> PyResult<usize> {
        self.inner.find(name).ok_or_else(|| PyValueError::new_err(format!("no expression named {name:?}")))
    }

    /// Meshes expression `name` moves.
    fn meshes_of(&self, name: &str) -> PyResult<Vec<String>> {
        Ok(self.inner.expressions[self.find(name)?].meshes.clone())
    }

    /// Sparse delta of one expression at weight 1: `(u32 indices bytes, f32 xyz bytes)`.
    fn deltas<'py>(&self, py: Python<'py>, name: &str) -> PyResult<(Bound<'py, PyBytes>, Bound<'py, PyBytes>)> {
        let (idx, d) = self.inner.deltas(self.find(name)?);
        let idx: Vec<u8> = idx.iter().flat_map(|i| i.to_le_bytes()).collect();
        Ok((PyBytes::new(py, &idx), f32_bytes(py, &d)))
    }

    /// Vertex deltas for `{name: weight}` (glTF frame, meters) as little-endian f32
    /// bytes, x, y, z per global vertex.
    fn evaluate<'py>(&self, py: Python<'py>, weights: HashMap<String, f32>) -> PyResult<Bound<'py, PyBytes>> {
        let w = self.inner.weights_from(weights.iter().map(|(k, &v)| (k.as_str(), v))).map_err(err)?;
        let d = py.detach(|| self.inner.evaluate(&w));
        Ok(f32_bytes(py, &d))
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __repr__(&self) -> String {
        format!("<cc5_shape.Expressions: {} expressions, {} vertices>", self.inner.len(), self.inner.vertex_count)
    }
}

/// Map from the pack's (level 0) vertex space to CC5's HD mesh (`hd_map.json` + `.bin`).
#[pyclass(name = "SubdivMap", module = "cc5_shape", frozen)]
struct PySubdivMap {
    inner: SubdivMap,
}

#[pymethods]
impl PySubdivMap {
    #[new]
    fn new(dir: std::path::PathBuf) -> PyResult<Self> {
        Ok(Self { inner: SubdivMap::load(&dir).map_err(err)? })
    }

    /// CC5 subdivision level (1 or 2).
    #[getter]
    fn level(&self) -> u32 {
        self.inner.level
    }

    /// HD global vertex count.
    #[getter]
    fn vertex_count(&self) -> usize {
        self.inner.vertex_count
    }

    /// `[(mesh name, HD offset, HD count)]`.
    #[getter]
    fn meshes(&self) -> Vec<(String, u32, u32)> {
        self.inner.meshes.iter().map(|m| (m.name.clone(), m.offset, m.count)).collect()
    }

    /// Base-space per-vertex f32 xyz bytes (e.g. `Pack.evaluate`'s vertex deltas)
    /// -> HD-space f32 xyz bytes.
    fn expand<'py>(&self, py: Python<'py>, verts: &[u8]) -> PyResult<Bound<'py, PyBytes>> {
        if verts.len() != self.inner.base_vertex_count * 12 {
            return Err(PyValueError::new_err(format!(
                "expected {} bytes ({} base vertices x 3 f32), got {}",
                self.inner.base_vertex_count * 12,
                self.inner.base_vertex_count,
                verts.len()
            )));
        }
        let v: Vec<[f32; 3]> = verts
            .chunks_exact(12)
            .map(|c| std::array::from_fn(|k| f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap())))
            .collect();
        let out = py.detach(|| self.inner.expand_verts(&v));
        Ok(f32_bytes(py, &out))
    }

    fn __repr__(&self) -> String {
        format!("<cc5_shape.SubdivMap: level {}, {} HD vertices from {}>", self.inner.level, self.inner.vertex_count, self.inner.base_vertex_count)
    }
}

fn read_f32x3(b: &[u8], what: &str) -> PyResult<Vec<[f32; 3]>> {
    if b.len() % 12 != 0 {
        return Err(PyValueError::new_err(format!("{what}: {} bytes is not a whole number of f32 xyz", b.len())));
    }
    Ok(b.chunks_exact(12).map(|c| std::array::from_fn(|k| f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap()))).collect())
}

/// Bind an uncaptured mesh (hair cards, a beard) to body anchor vertices.
/// `body` / `asset`: f32 xyz bytes (bind positions, same space); `anchors`:
/// indices into `body`; `mode`: "scalp", "skin" or "head_box" (see `ConformOptions`).
#[pyclass(name = "Conform", module = "cc5_shape", frozen)]
struct PyConform {
    inner: Conform,
    body_len: usize,
}

#[pymethods]
impl PyConform {
    #[new]
    #[pyo3(signature = (body, anchors, asset, mode="scalp", k=None, sigma_min=None, sigma_scale=None, similarity=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(py: Python<'_>, body: &[u8], anchors: Vec<u32>, asset: &[u8], mode: &str, k: Option<usize>, sigma_min: Option<f32>,
           sigma_scale: Option<f32>, similarity: Option<bool>) -> PyResult<Self> {
        let mut o = match mode {
            "scalp" => ConformOptions::SCALP,
            "skin" => ConformOptions::SKIN,
            "head_box" => ConformOptions::HEAD_BOX,
            _ => return Err(PyValueError::new_err(format!("mode {mode:?}: expected \"scalp\", \"skin\" or \"head_box\""))),
        };
        o.k = k.unwrap_or(o.k);
        o.sigma_min = sigma_min.unwrap_or(o.sigma_min);
        o.sigma_scale = sigma_scale.unwrap_or(o.sigma_scale);
        o.similarity = similarity.unwrap_or(o.similarity);
        let (body, asset) = (read_f32x3(body, "body")?, read_f32x3(asset, "asset")?);
        let inner = py.detach(|| Conform::bind(&body, &anchors, &asset, o)).map_err(err)?;
        Ok(Self { inner, body_len: body.len() })
    }

    /// Body vertex deltas (f32 xyz bytes) -> asset vertex deltas (f32 xyz bytes).
    fn apply<'py>(&self, py: Python<'py>, body_delta: &[u8]) -> PyResult<Bound<'py, PyBytes>> {
        let d = read_f32x3(body_delta, "body_delta")?;
        if d.len() != self.body_len {
            return Err(PyValueError::new_err(format!("body_delta has {} vertices, the body {}", d.len(), self.body_len)));
        }
        let out = py.detach(|| self.inner.apply(&d));
        Ok(f32_bytes(py, &out))
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }
}

/// ARKit-52 -> expression weights, from a runtime ARKit map JSON.
#[pyclass(name = "ArkitMap", module = "cc5_shape", frozen)]
struct PyArkitMap {
    inner: ArkitMap,
    names: Vec<String>,
}

#[pymethods]
impl PyArkitMap {
    #[new]
    fn new(path: std::path::PathBuf, expressions: &PyExpressions) -> PyResult<Self> {
        let inner = ArkitMap::load(&path, &expressions.inner).map_err(err)?;
        Ok(Self { inner, names: expressions.inner.names().map(str::to_owned).collect() })
    }

    /// ARKit names in the map, sorted.
    #[getter]
    fn arkit_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.mapping.keys().cloned().collect();
        v.sort();
        v
    }

    /// Morph names the map uses that the expression pack lacks.
    #[getter]
    fn missing(&self) -> Vec<String> {
        self.inner.missing.clone()
    }

    /// `{expression name: weight}` for `{arkit name: weight}` (for `Expressions.evaluate`).
    fn weights(&self, arkit: HashMap<String, f32>) -> PyResult<HashMap<String, f32>> {
        let w = self.inner.weights(arkit.iter().map(|(k, &v)| (k.as_str(), v))).map_err(err)?;
        Ok(w.into_iter().map(|(i, v)| (self.names[i].clone(), v)).collect())
    }
}

#[pymodule]
fn cc5_shape(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPack>()?;
    m.add_class::<PyExpressions>()?;
    m.add_class::<PyArkitMap>()?;
    m.add_class::<PySubdivMap>()?;
    m.add_class::<PyConform>()?;
    Ok(())
}
