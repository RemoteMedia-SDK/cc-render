//! Write a [`BuiltPart`] as a library part GLB (what hair_build.py's export +
//! `patch_glb` + quantize_glb.py produced): the head's armature and skin
//! (joints re-bound by name in the editor), one skinned mesh per part mesh,
//! strands as ribbons, attributes quantized (u8 colours / weights, u16 UVs and
//! indices, primitives of at most 65 536 vertices).

use std::path::Path;

use anyhow::{anyhow, Result};
use glam::{Vec2, Vec3};
use serde_json::{json, Value};

use crate::part::{BuiltPart, Geometry};
use crate::source::Source;
use crate::{Strand, HEAD_CENTER};

/// Ribbon vertices of strands (hair_build.py `make_strand_object`): two rows per
/// strand across its width, facing out from the head; UV = (strand ID, t from
/// root 0 to tip 1); colour = texture colour × shade.
pub struct Ribbons {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub col: Vec<[f32; 4]>,
    pub skin: Vec<[(u16, f32); 4]>,
    pub tris: Vec<[u32; 3]>,
}

pub fn ribbons(strands: &[Strand], skin: &[[(u16, f32); 4]], taper: f32, root_fade: f32) -> Ribbons {
    let n: usize = strands.iter().map(|s| 2 * s.pts.len()).sum();
    let mut r = Ribbons { pos: Vec::with_capacity(n), nrm: Vec::with_capacity(n), uv: Vec::with_capacity(n), col: Vec::with_capacity(n), skin: Vec::with_capacity(n), tris: Vec::new() };
    for (s, sk) in strands.iter().zip(skin) {
        let m = s.pts.len();
        let o = r.pos.len() as u32;
        let p = &s.pts;
        let d: Vec<Vec3> = (0..m)
            .map(|i| {
                let g = if i == 0 { p[1] - p[0] } else if i == m - 1 { p[m - 1] - p[m - 2] } else { (p[i + 1] - p[i - 1]) / 2.0 };
                g.normalize_or_zero()
            })
            .collect();
        for sgn in [-1.0f32, 1.0] {
            for i in 0..m {
                let tt = i as f32 / (m - 1) as f32;
                let mut w = s.width * (1.0 - taper * tt * tt);
                if root_fade > 0.0 {
                    let x = (tt / root_fade).clamp(0.0, 1.0);
                    w *= x * x * (3.0 - 2.0 * x);
                }
                // across the strand, facing out from the head
                let mut side = d[i].cross(p[i] - HEAD_CENTER);
                if side.length() < 1e-9 {
                    side = d[i].cross(Vec3::X);
                }
                let side = side.normalize_or_zero();
                r.pos.push(p[i] + side * (0.5 * sgn * w));
                r.nrm.push(d[i].cross(side).normalize_or_zero());
                r.uv.push(Vec2::new(0.02 + 0.96 * s.id, tt));
                let c = (s.rgb[i] * s.shade).clamp(Vec3::ZERO, Vec3::ONE);
                r.col.push([c.x, c.y, c.z, 1.0]);
                r.skin.push(*sk);
            }
        }
        let (a, b) = (o, o + m as u32);
        for k in 0..m as u32 - 1 {
            r.tris.push([a + k, a + k + 1, b + k + 1]);
            r.tris.push([a + k, b + k + 1, b + k]);
        }
    }
    r
}

/// GLB under construction: JSON arrays and the binary chunk.
struct Doc {
    json: Value,
    bin: Vec<u8>,
}

impl Doc {
    fn push(&mut self, key: &str, v: Value) -> usize {
        let a = self.json[key].as_array_mut().unwrap();
        a.push(v);
        a.len() - 1
    }

    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let off = self.bin.len();
        self.bin.extend_from_slice(bytes);
        let mut v = json!({"buffer": 0, "byteOffset": off, "byteLength": bytes.len()});
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.push("bufferViews", v)
    }

    fn accessor(&mut self, bytes: &[u8], target: Option<u32>, ctype: u32, count: usize, ty: &str, normalized: bool, minmax: Option<(Vec3, Vec3)>) -> usize {
        let view = self.view(bytes, target);
        let mut a = json!({"bufferView": view, "componentType": ctype, "count": count, "type": ty});
        if normalized {
            a["normalized"] = json!(true);
        }
        if let Some((lo, hi)) = minmax {
            a["min"] = json!(lo.to_array());
            a["max"] = json!(hi.to_array());
        }
        self.push("accessors", a)
    }
}

fn f32_bytes(v: impl IntoIterator<Item = f32>) -> Vec<u8> {
    v.into_iter().flat_map(f32::to_le_bytes).collect()
}

/// Weights as u8 summing to 255 per vertex, with u8 joint indices.
fn skin_bytes(skin: &[[(u16, f32); 4]]) -> (Vec<u8>, Vec<u8>) {
    let mut j = Vec::with_capacity(skin.len() * 4);
    let mut w = Vec::with_capacity(skin.len() * 4);
    for s in skin {
        let mut q: [i32; 4] = s.map(|e| (e.1 * 255.0).round() as i32);
        q[0] += 255 - q.iter().sum::<i32>();
        for k in 0..4 {
            j.push(s[k].0 as u8);
            w.push(q[k].clamp(0, 255) as u8);
        }
    }
    (j, w)
}

/// One primitive's attributes from vertex range `vr` of a mesh's arrays.
#[allow(clippy::too_many_arguments)]
fn primitive(doc: &mut Doc, pos: &[Vec3], nrm: &[Vec3], uv: &[Vec2], col: Option<&[[f32; 4]]>, skin: &[[(u16, f32); 4]], tris: &[[u32; 3]], material: usize) -> Value {
    let (lo, hi) = pos.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let n = pos.len();
    let p = doc.accessor(&f32_bytes(pos.iter().flat_map(|v| v.to_array())), Some(34962), 5126, n, "VEC3", false, Some((lo, hi)));
    let nr = doc.accessor(&f32_bytes(nrm.iter().flat_map(|v| v.to_array())), Some(34962), 5126, n, "VEC3", false, None);
    let in_unit = uv.iter().all(|u| u.x >= 0.0 && u.x <= 1.0 && u.y >= 0.0 && u.y <= 1.0);
    let t = if in_unit {
        let b: Vec<u8> = uv.iter().flat_map(|u| [u.x, u.y]).flat_map(|x| ((x * 65535.0).round() as u16).to_le_bytes()).collect();
        doc.accessor(&b, Some(34962), 5123, n, "VEC2", true, None)
    } else {
        doc.accessor(&f32_bytes(uv.iter().flat_map(|u| u.to_array())), Some(34962), 5126, n, "VEC2", false, None)
    };
    let (jb, wb) = skin_bytes(skin);
    let j = doc.accessor(&jb, Some(34962), 5121, n, "VEC4", false, None);
    let w = doc.accessor(&wb, Some(34962), 5121, n, "VEC4", true, None);
    let mut attrs = json!({"POSITION": p, "NORMAL": nr, "TEXCOORD_0": t, "JOINTS_0": j, "WEIGHTS_0": w});
    if let Some(col) = col {
        let b: Vec<u8> = col.iter().flat_map(|c| c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        attrs["COLOR_0"] = json!(doc.accessor(&b, Some(34962), 5121, n, "VEC4", true, None));
    }
    let ib: Vec<u8> = tris.iter().flatten().flat_map(|&i| (i as u16).to_le_bytes()).collect();
    let idx = doc.accessor(&ib, Some(34963), 5123, tris.len() * 3, "SCALAR", false, None);
    json!({"attributes": attrs, "indices": idx, "material": material, "mode": 4})
}

/// Split triangles into primitives whose vertex ranges fit u16 indices.
/// Triangles must reference increasing vertex ranges (ribbons, cards are so
/// laid out; a triangle never spans more than 65 536 vertices).
fn windows(tris: &[[u32; 3]], nverts: usize) -> Vec<(usize, usize, Vec<[u32; 3]>)> {
    let mut out = Vec::new();
    let (mut lo, mut cur): (u32, Vec<[u32; 3]>) = (0, Vec::new());
    let mut hi = 0u32;
    for t in tris {
        let (tl, th) = (*t.iter().min().unwrap(), *t.iter().max().unwrap());
        if !cur.is_empty() && (th.max(hi) - lo.min(tl) >= 65535) {
            out.push((lo as usize, hi as usize + 1, std::mem::take(&mut cur)));
        }
        if cur.is_empty() {
            lo = tl;
            hi = th;
        }
        lo = lo.min(tl);
        hi = hi.max(th);
        cur.push(*t);
    }
    if !cur.is_empty() {
        out.push((lo as usize, hi as usize + 1, cur));
    }
    if out.is_empty() && nverts > 0 {
        out.push((0, nverts.min(65535), Vec::new()));
    }
    out.into_iter().map(|(a, b, ts)| (a, b, ts.into_iter().map(|t| t.map(|i| i - a as u32)).collect())).collect()
}

/// Write `part` to `path`, with `head`'s armature (nodes + skin).
pub fn write(part: &BuiltPart, head: &Source, path: &Path) -> Result<()> {
    write_with(part, &part.materials, head, path)
}

/// [`write`] with other materials (same count and order: e.g. edited colours).
pub fn write_with(part: &BuiltPart, materials: &[crate::part::PartMaterial], head: &Source, path: &Path) -> Result<()> {
    let mut doc = Doc {
        json: json!({"asset": {"version": "2.0", "generator": "cc5-hair"}, "scene": 0, "scenes": [{"nodes": []}],
                     "nodes": [], "meshes": [], "materials": [], "accessors": [], "bufferViews": [], "buffers": [],
                     "skins": [], "images": [], "textures": [], "samplers": []}),
        bin: Vec::new(),
    };
    // the head's armature: every node that is not a mesh node, same hierarchy and transforms
    let skin = head.doc.skins().next().ok_or_else(|| anyhow!("{}: no skin", head.path.display()))?;
    let mesh_nodes: std::collections::HashSet<usize> = head.doc.nodes().filter(|n| n.mesh().is_some()).map(|n| n.index()).collect();
    let mut remap = std::collections::HashMap::new();
    for n in head.doc.nodes() {
        if !mesh_nodes.contains(&n.index()) {
            remap.insert(n.index(), remap.len());
        }
    }
    let mut nodes = vec![Value::Null; remap.len()];
    for n in head.doc.nodes() {
        let Some(&i) = remap.get(&n.index()) else { continue };
        let (t, r, s) = n.transform().decomposed();
        let mut v = json!({"name": n.name().unwrap_or(""), "translation": t, "rotation": r, "scale": s});
        let kids: Vec<usize> = n.children().filter_map(|c| remap.get(&c.index()).copied()).collect();
        if !kids.is_empty() {
            v["children"] = json!(kids);
        }
        nodes[i] = v;
    }
    let roots: Vec<usize> = head.doc.scenes().next().map(|s| s.nodes().filter_map(|n| remap.get(&n.index()).copied()).collect()).unwrap_or_default();
    let armature = *roots.first().ok_or_else(|| anyhow!("head: no armature root"))?;
    doc.json["nodes"] = Value::Array(nodes);
    let ibm = skin.reader(|b| (b.index() == 0).then_some(head.blob.as_slice())).read_inverse_bind_matrices().ok_or_else(|| anyhow!("head skin: no inverse binds"))?;
    let ibm_bytes = f32_bytes(ibm.flat_map(|m| m.into_iter().flatten()));
    let nj = skin.joints().count();
    let ibm_acc = doc.accessor(&ibm_bytes, None, 5126, nj, "MAT4", false, None);
    let joints: Vec<usize> = skin.joints().map(|j| remap[&j.index()]).collect();
    doc.push("skins", json!({"joints": joints, "inverseBindMatrices": ibm_acc}));
    doc.json["scenes"][0]["nodes"] = json!(roots);
    doc.push("samplers", json!({"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}));
    // materials
    for m in materials {
        let mut v = json!({"name": m.name, "doubleSided": true,
                           "pbrMetallicRoughness": {"metallicFactor": 0.0, "roughnessFactor": 0.5, "baseColorFactor": m.base_factor}});
        if m.blend {
            v["alphaMode"] = json!("BLEND"); // one soft layer on the skin, like CC5's scalp
        } else {
            v["alphaMode"] = json!("MASK");
            v["alphaCutoff"] = json!(0.35);
        }
        if let Some((bytes, mime)) = &m.base_image {
            let view = doc.view(bytes, None);
            let img = doc.push("images", json!({"bufferView": view, "mimeType": mime}));
            let tex = doc.push("textures", json!({"source": img, "sampler": 0}));
            v["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index": tex});
        }
        if let Some(c) = &m.cc5 {
            v["extras"] = json!({"cc5": c});
        }
        doc.push("materials", v);
    }
    // meshes, each on a node under the armature
    let mut mesh_node_ids = Vec::new();
    for pm in &part.meshes {
        let (pos, nrm, uv, col, skin, tris) = match &pm.geom {
            Geometry::Strands { strands, skin, taper, root_fade } => {
                let r = ribbons(strands, skin, *taper, *root_fade);
                (r.pos, r.nrm, r.uv, Some(r.col), r.skin, r.tris)
            }
            Geometry::Cards { pos, nrm, uv, tris, skin } => (pos.clone(), nrm.clone(), uv.clone(), None, skin.clone(), tris.clone()),
        };
        let mut prims = Vec::new();
        for (a, b, ts) in windows(&tris, pos.len()) {
            prims.push(primitive(&mut doc, &pos[a..b], &nrm[a..b], &uv[a..b], col.as_ref().map(|c| &c[a..b]), &skin[a..b], &ts, pm.material));
        }
        let mi = doc.push("meshes", json!({"name": pm.name, "primitives": prims}));
        mesh_node_ids.push(doc.push("nodes", json!({"name": pm.name, "mesh": mi, "skin": 0})));
    }
    let arm = &mut doc.json["nodes"][armature];
    let mut kids: Vec<usize> = arm.get("children").and_then(|c| serde_json::from_value(c.clone()).ok()).unwrap_or_default();
    kids.extend(mesh_node_ids);
    arm["children"] = json!(kids);
    for k in ["images", "textures"] {
        if doc.json[k].as_array().is_some_and(|a| a.is_empty()) {
            doc.json.as_object_mut().unwrap().remove(k);
        }
    }
    while doc.bin.len() % 4 != 0 {
        doc.bin.push(0);
    }
    doc.json["buffers"] = json!([{"byteLength": doc.bin.len()}]);
    let mut js = serde_json::to_vec(&doc.json)?;
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + doc.bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&0x4654_6C67u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&js);
    out.extend_from_slice(&(doc.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&doc.bin);
    std::fs::write(path, out)?;
    Ok(())
}

/// Copy the part's CC5 map files into the library.
pub fn copy_files(part: &BuiltPart, lib: &Path) -> Result<()> {
    for (rel, src) in &part.files {
        let dst = lib.join(rel);
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(src, &dst)?;
    }
    Ok(())
}
