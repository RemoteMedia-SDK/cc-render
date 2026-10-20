//! Write a shaped copy of a GLB: apply a [`ShapeDelta`] to the bind mesh and skeleton.
//!
//! The GLB must carry the `_CCIDX` vertex attribute (CC5 vertex index per glTF
//! vertex, written by `fbx_to_glb.py`) so seam-split glTF vertices map back to
//! the pack's global vertex space.
//!
//! - Vertex positions: `bind + delta`, with the world-space delta taken into each
//!   skinned mesh's bind space (`W(joint) · IBM(joint)`).
//! - Joints: every node's world translation moves by its pack delta, or by its
//!   parent's when it has none (helpers ride along). Rotations are unchanged, so
//!   a node's local translation changes by `parent_linear⁻¹ · (Δnode − Δparent)`.
//! - Inverse bind matrices: `IBM' = W'⁻¹ · W · IBM`, so the new bind mesh renders
//!   exactly as written in the bind pose.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Context, Result};
use glam::{Mat3, Mat4, Quat, Vec3};
use serde_json::Value;

use crate::pack::{ShapeDelta, ShapePack};
use crate::subdiv::SubdivMap;

const GLB_MAGIC: u32 = 0x4654_6C67;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

struct Glb {
    json: Value,
    bin: Vec<u8>,
}

fn parse(glb: &[u8]) -> Result<Glb> {
    let u32_at = |o: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(glb.get(o..o + 4).ok_or_else(|| anyhow!("truncated GLB"))?.try_into()?))
    };
    if u32_at(0)? != GLB_MAGIC {
        bail!("not a GLB file");
    }
    let (mut json, mut bin, mut off) = (None, None, 12usize);
    while off + 8 <= glb.len() {
        let (len, kind) = (u32_at(off)? as usize, u32_at(off + 4)?);
        let body = glb.get(off + 8..off + 8 + len).ok_or_else(|| anyhow!("truncated GLB chunk"))?;
        match kind {
            CHUNK_JSON => json = Some(serde_json::from_slice(body)?),
            CHUNK_BIN => bin = Some(body.to_vec()),
            _ => {}
        }
        off += 8 + len;
    }
    Ok(Glb { json: json.context("GLB has no JSON chunk")?, bin: bin.context("GLB has no BIN chunk")? })
}

fn write(glb: &Glb) -> Result<Vec<u8>> {
    let mut json = serde_json::to_vec(&glb.json)?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut bin = glb.bin.clone();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    for v in [GLB_MAGIC, 2, total as u32, json.len() as u32, CHUNK_JSON] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
    out.extend_from_slice(&bin);
    Ok(out)
}

/// Byte offset and stride of each element of a float accessor with `comps` components.
fn accessor_layout(json: &Value, acc: usize, comps: usize) -> Result<(usize, usize, usize)> {
    let a = &json["accessors"][acc];
    if a["componentType"].as_u64() != Some(5126) {
        bail!("accessor {acc} is not float");
    }
    let bv = &json["bufferViews"][a["bufferView"].as_u64().context("sparse accessors unsupported")? as usize];
    let start = bv["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
    let stride = bv["byteStride"].as_u64().map(|s| s as usize).unwrap_or(4 * comps);
    Ok((start, stride, a["count"].as_u64().context("accessor count")? as usize))
}

fn read_f32s<const N: usize>(g: &Glb, acc: usize) -> Result<Vec<[f32; N]>> {
    let (start, stride, count) = accessor_layout(&g.json, acc, N)?;
    (0..count)
        .map(|i| {
            let b = start + i * stride;
            let mut v = [0f32; N];
            for (k, x) in v.iter_mut().enumerate() {
                let s = g.bin.get(b + 4 * k..b + 4 * k + 4).ok_or_else(|| anyhow!("accessor past BIN"))?;
                *x = f32::from_le_bytes(s.try_into()?);
            }
            Ok(v)
        })
        .collect()
}

fn write_f32s<const N: usize>(g: &mut Glb, acc: usize, vals: &[[f32; N]]) -> Result<()> {
    let (start, stride, count) = accessor_layout(&g.json, acc, N)?;
    if count != vals.len() {
        bail!("accessor {acc}: {} values for {count} elements", vals.len());
    }
    for (i, v) in vals.iter().enumerate() {
        let b = start + i * stride;
        for (k, x) in v.iter().enumerate() {
            g.bin[b + 4 * k..b + 4 * k + 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    Ok(())
}

fn node_local(node: &Value) -> Mat4 {
    if let Some(m) = node["matrix"].as_array() {
        let a: Vec<f32> = m.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
        return Mat4::from_cols_slice(&a);
    }
    let v = |k: &str, d: &[f32]| -> Vec<f32> {
        node[k].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or(d.to_vec())
    };
    let (t, r, s) = (v("translation", &[0.0; 3]), v("rotation", &[0.0, 0.0, 0.0, 1.0]), v("scale", &[1.0; 3]));
    Mat4::from_scale_rotation_translation(Vec3::new(s[0], s[1], s[2]), Quat::from_xyzw(r[0], r[1], r[2], r[3]), Vec3::new(t[0], t[1], t[2]))
}

/// Shaped copy of `glb` for `delta` (from `pack.evaluate`).
pub fn shape_glb(glb: &[u8], pack: &ShapePack, delta: &ShapeDelta) -> Result<Vec<u8>> {
    if delta.verts.len() != pack.vertex_count {
        bail!("delta has {} vertices, pack {}", delta.verts.len(), pack.vertex_count);
    }
    shape(glb, &pack.joints, &|n| pack.mesh_offset(n), delta)
}

/// Shaped copy of an HD character GLB (CC5 export at `map.level`, `_CCIDX` in
/// CC5's HD vertex order) for an HD delta (`map.expand(&pack.evaluate(w))`).
pub fn shape_glb_hd(glb: &[u8], pack: &ShapePack, map: &SubdivMap, hd_delta: &ShapeDelta) -> Result<Vec<u8>> {
    if hd_delta.verts.len() != map.vertex_count {
        bail!("delta has {} vertices, HD map {}", hd_delta.verts.len(), map.vertex_count);
    }
    shape(glb, &pack.joints, &|n| map.mesh_offset(n), hd_delta)
}

fn shape(glb: &[u8], joints: &[String], mesh_offset: &dyn Fn(&str) -> Option<u32>, delta: &ShapeDelta) -> Result<Vec<u8>> {
    let mut g = parse(glb)?;
    let nodes = g.json["nodes"].as_array().cloned().unwrap_or_default();
    let mut parent = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for c in n["children"].as_array().into_iter().flatten() {
            parent[c.as_u64().context("child index")? as usize] = Some(i);
        }
    }
    // Hierarchy order (parents first) and base world matrices.
    let mut order: Vec<usize> = Vec::with_capacity(nodes.len());
    let mut placed = vec![false; nodes.len()];
    while order.len() < nodes.len() {
        let before = order.len();
        for i in 0..nodes.len() {
            if !placed[i] && parent[i].map_or(true, |p| placed[p]) {
                placed[i] = true;
                order.push(i);
            }
        }
        if order.len() == before {
            bail!("node hierarchy has a cycle");
        }
    }
    let local: Vec<Mat4> = nodes.iter().map(node_local).collect();
    let mut world = vec![Mat4::IDENTITY; nodes.len()];
    for &i in &order {
        world[i] = parent[i].map_or(local[i], |p| world[p] * local[i]);
    }

    // World translation delta per node: its pack delta, else its parent's.
    let pack_joint: HashMap<&str, usize> = joints.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    let mut dworld = vec![Vec3::ZERO; nodes.len()];
    for &i in &order {
        let own = nodes[i]["name"].as_str().and_then(|n| pack_joint.get(n));
        dworld[i] = match own {
            Some(&j) => Vec3::from(delta.joints[j]),
            None => parent[i].map_or(Vec3::ZERO, |p| dworld[p]),
        };
    }
    for &i in &order {
        if dworld[i] == parent[i].map_or(Vec3::ZERO, |p| dworld[p]) {
            continue;
        }
        if nodes[i].get("matrix").is_some() {
            bail!("node {i} uses a matrix; only TRS joints are supported");
        }
        let plin = parent[i].map_or(Mat3::IDENTITY, |p| Mat3::from_mat4(world[p]));
        let d = plin.inverse() * (dworld[i] - parent[i].map_or(Vec3::ZERO, |p| dworld[p]));
        let n = &mut g.json["nodes"][i];
        let t = n["translation"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect::<Vec<_>>()).unwrap_or(vec![0.0; 3]);
        n["translation"] = serde_json::json!([t[0] + d.x, t[1] + d.y, t[2] + d.z]);
    }
    let world_new = |i: usize| Mat4::from_translation(dworld[i]) * world[i];

    // Inverse bind matrices; remember each skin's bind-space transform.
    let skins = g.json["skins"].as_array().cloned().unwrap_or_default();
    let mut mesh_space = HashMap::new();
    for (s, skin) in skins.iter().enumerate() {
        let joints: Vec<usize> = skin["joints"].as_array().context("skin joints")?.iter().filter_map(|j| j.as_u64().map(|j| j as usize)).collect();
        let acc = skin["inverseBindMatrices"].as_u64().context("skin without inverseBindMatrices")? as usize;
        let ibm: Vec<[f32; 16]> = read_f32s(&g, acc)?;
        let base: Vec<Mat4> = ibm.iter().map(Mat4::from_cols_array).collect();
        mesh_space.insert(s, world[joints[0]] * base[0]);
        let new: Vec<[f32; 16]> = joints.iter().zip(&base).map(|(&j, b)| (world_new(j).inverse() * world[j] * *b).to_cols_array()).collect();
        write_f32s(&mut g, acc, &new)?;
    }

    // Vertex positions.
    let mut skin_of_mesh = HashMap::new();
    for n in &nodes {
        if let (Some(m), Some(s)) = (n["mesh"].as_u64(), n["skin"].as_u64()) {
            skin_of_mesh.insert(m as usize, s as usize);
        }
    }
    let meshes = g.json["meshes"].as_array().cloned().unwrap_or_default();
    for (m, mesh) in meshes.iter().enumerate() {
        let name = mesh["name"].as_str().unwrap_or_default();
        let Some(offset) = mesh_offset(name) else { continue };
        let to_mesh = skin_of_mesh.get(&m).and_then(|s| mesh_space.get(s)).map_or(Mat3::IDENTITY, |w| Mat3::from_mat4(w.inverse()));
        for (p, prim) in mesh["primitives"].as_array().into_iter().flatten().enumerate() {
            let attrs = &prim["attributes"];
            let (Some(pos_acc), Some(idx_acc)) = (attrs["POSITION"].as_u64(), attrs["_CCIDX"].as_u64()) else {
                bail!("mesh {name} primitive {p} lacks POSITION or _CCIDX");
            };
            let ccidx: Vec<[f32; 1]> = read_f32s(&g, idx_acc as usize)?;
            let mut pos: Vec<[f32; 3]> = read_f32s(&g, pos_acc as usize)?;
            for (v, [ci]) in pos.iter_mut().zip(&ccidx) {
                let gi = offset as usize + *ci as usize;
                let d = to_mesh * Vec3::from(*delta.verts.get(gi).ok_or_else(|| anyhow!("{name}: _CCIDX {ci} past pack"))?);
                *v = (Vec3::from(*v) + d).into();
            }
            write_f32s(&mut g, pos_acc as usize, &pos)?;
            let (lo, hi) = pos.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(Vec3::from(*v)), hi.max(Vec3::from(*v))));
            g.json["accessors"][pos_acc as usize]["min"] = serde_json::json!([lo.x, lo.y, lo.z]);
            g.json["accessors"][pos_acc as usize]["max"] = serde_json::json!([hi.x, hi.y, hi.z]);
        }
    }
    write(&g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::ShapeDelta;

    /// Two-joint chain (Hip -> Head, Head rotated 90° about Y), one skinned mesh
    /// with 3 glTF vertices mapping to 2 CC5 vertices (index 1 seam-split).
    fn tiny_glb() -> Vec<u8> {
        let mut bin = Vec::new();
        let mut push = |vals: &[f32]| {
            let off = bin.len();
            for v in vals {
                bin.extend_from_slice(&v.to_le_bytes());
            }
            off
        };
        let pos = push(&[0.0, 1.0, 0.0, 0.0, 1.7, 0.0, 0.0, 1.7, 0.0]);
        let cc = push(&[0.0, 1.0, 1.0]);
        let hip = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let head = hip * Mat4::from_rotation_translation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::new(0.0, 0.7, 0.0));
        let ibm = push(&[hip.inverse().to_cols_array(), head.inverse().to_cols_array()].concat());
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "nodes": [
                {"name": "Hip", "translation": [0.0, 1.0, 0.0], "children": [1]},
                {"name": "Head", "translation": [0.0, 0.7, 0.0], "rotation": [0.0, 0.70710677, 0.0, 0.70710677]},
                {"name": "Body", "mesh": 0, "skin": 0}
            ],
            "skins": [{"joints": [0, 1], "inverseBindMatrices": 2}],
            "meshes": [{"name": "CC_Base_Body", "primitives": [{"attributes": {"POSITION": 0, "_CCIDX": 1}}]}],
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": pos, "byteLength": 36},
                {"buffer": 0, "byteOffset": cc, "byteLength": 12},
                {"buffer": 0, "byteOffset": ibm, "byteLength": 128}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "SCALAR"},
                {"bufferView": 2, "componentType": 5126, "count": 2, "type": "MAT4"}
            ]
        });
        write(&Glb { json, bin }).unwrap()
    }

    fn pack(dir: &std::path::Path) -> ShapePack {
        std::fs::write(dir.join("pack.bin"), []).unwrap();
        let json = serde_json::json!({"version": 1, "vertex_count": 2,
            "meshes": [{"name": "CC_Base_Body", "offset": 0, "count": 2}],
            "joints": ["Head", "Hip"], "sliders": []});
        std::fs::write(dir.join("pack.json"), serde_json::to_vec(&json).unwrap()).unwrap();
        ShapePack::load(dir).unwrap()
    }

    #[test]
    fn moves_mesh_joints_and_keeps_skin_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let pack = pack(dir.path());
        // Hip down 0.1, Head down 0.15 (so the Hip->Head offset shrinks by 0.05);
        // vertex 1 (both seam copies) down 0.15.
        let delta = ShapeDelta {
            verts: vec![[0.0, -0.1, 0.0], [0.0, -0.15, 0.0]],
            joints: vec![[0.0, -0.15, 0.0], [0.0, -0.1, 0.0]],
        };
        let out = parse(&shape_glb(&tiny_glb(), &pack, &delta).unwrap()).unwrap();
        let t = |i: usize| out.json["nodes"][i]["translation"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect::<Vec<_>>();
        assert!((t(0)[1] - 0.9).abs() < 1e-6, "{:?}", t(0));
        assert!((t(1)[1] - 0.65).abs() < 1e-6, "{:?}", t(1));
        let pos: Vec<[f32; 3]> = read_f32s(&out, 0).unwrap();
        assert!((pos[0][1] - 0.9).abs() < 1e-6 && (pos[1][1] - 1.55).abs() < 1e-6 && (pos[2][1] - 1.55).abs() < 1e-6, "{pos:?}");
        assert!((out.json["accessors"][0]["max"][1].as_f64().unwrap() - 1.55).abs() < 1e-6);

        // Bind renders the written mesh: world(joint) · IBM' is the same for both joints.
        let local: Vec<Mat4> = out.json["nodes"].as_array().unwrap().iter().map(node_local).collect();
        let world = [local[0], local[0] * local[1]];
        let ibm: Vec<[f32; 16]> = read_f32s(&out, 2).unwrap();
        let m: Vec<Mat4> = world.iter().zip(&ibm).map(|(w, i)| *w * Mat4::from_cols_array(i)).collect();
        assert!(m[0].abs_diff_eq(m[1], 1e-5) && m[0].abs_diff_eq(Mat4::IDENTITY, 1e-5), "{m:?}");
    }

    #[test]
    fn rejects_mesh_without_ccidx() {
        let dir = tempfile::tempdir().unwrap();
        let pack = pack(dir.path());
        let mut g = parse(&tiny_glb()).unwrap();
        g.json["meshes"][0]["primitives"][0]["attributes"].as_object_mut().unwrap().remove("_CCIDX");
        let delta = ShapeDelta { verts: vec![[0.0; 3]; 2], joints: vec![[0.0; 3]; 2] };
        let err = shape_glb(&write(&g).unwrap(), &pack, &delta).unwrap_err().to_string();
        assert!(err.contains("_CCIDX"), "{err}");
    }
}
