//! The GLB writer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use glam::Vec3;
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::cc5json::{Cc5Json, Maps};
use crate::Character;

#[derive(Default)]
pub struct GlbOptions {
    /// CC5's sidecar JSON: its tints and opacity (bake_alpha.py's first pass) and
    /// `extras.cc5` (glb_cc5_materials.py).
    pub cc5_json: Option<PathBuf>,
    /// Where the `extras.cc5` maps are written (default: next to the JSON,
    /// `<json stem>.cc5maps`); referenced as `<dir name>/<file>`.
    pub maps_dir: Option<PathBuf>,
}

#[derive(Default)]
struct Buf {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

const ARRAY: u32 = 34962;
const ELEMENTS: u32 = 34963;

impl Buf {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let mut v = json!({"buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len()});
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    fn accessor(&mut self, bytes: &[u8], component: u32, count: usize, ty: &str, target: Option<u32>) -> usize {
        let view = self.view(bytes, target);
        self.accessors.push(json!({"bufferView": view, "componentType": component, "count": count, "type": ty}));
        self.accessors.len() - 1
    }

    fn f32s(&mut self, v: &[f32], width: usize, ty: &str, target: Option<u32>) -> usize {
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.accessor(&bytes, 5126, v.len() / width, ty, target)
    }

    fn vec3s(&mut self, v: &[Vec3], min_max: bool) -> usize {
        let flat: Vec<f32> = v.iter().flat_map(|p| p.to_array()).collect();
        let a = self.f32s(&flat, 3, "VEC3", Some(ARRAY));
        if min_max && !v.is_empty() {
            let (lo, hi) = v.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
            self.accessors[a]["min"] = json!(lo.to_array());
            self.accessors[a]["max"] = json!(hi.to_array());
        }
        a
    }
}

/// What a material's base colour image is made of.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Img {
    /// A file as it is (JPEG / PNG; anything else re-encoded as PNG).
    File(PathBuf),
    /// Diffuse RGB (white without one) + opacity as alpha, PNG (as Blender's exporter).
    Merge(Option<PathBuf>, PathBuf),
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn png(img: &image::DynamicImage) -> Result<Vec<u8>> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut out = Vec::new();
    img.write_with_encoder(PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive))?;
    Ok(out)
}

fn open(p: &Path) -> Result<image::DynamicImage> {
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("tga")) {
        let b = std::fs::read(p).with_context(|| format!("reading {}", p.display()))?;
        return crate::tga::decode(&b).with_context(|| format!("decoding {}", p.display()));
    }
    image::open(p).with_context(|| format!("reading {}", p.display()))
}

impl Img {
    fn name(&self) -> String {
        match self {
            Img::File(p) => stem(p),
            Img::Merge(Some(d), o) => format!("{}-{}", stem(d), stem(o)),
            Img::Merge(None, o) => stem(o),
        }
    }

    /// (bytes, MIME type)
    fn encode(&self) -> Result<(Vec<u8>, &'static str)> {
        match self {
            Img::File(p) => file_bytes(p),
            Img::Merge(d, o) => {
                use image::imageops::{resize, FilterType};
                let mut op = open(o)?.to_luma8();
                let mut rgb = match d {
                    Some(d) => Some(open(d)?.to_rgb8()),
                    None => None,
                };
                let (w, h) = rgb.as_ref().map_or(op.dimensions(), |c| (c.width().max(op.width()), c.height().max(op.height())));
                if op.dimensions() != (w, h) {
                    op = resize(&op, w, h, FilterType::CatmullRom);
                }
                if let Some(c) = &mut rgb {
                    if c.dimensions() != (w, h) {
                        *c = resize(c, w, h, FilterType::CatmullRom);
                    }
                }
                let out = image::RgbaImage::from_fn(w, h, |x, y| {
                    let c = rgb.as_ref().map_or([255; 3], |c| c.get_pixel(x, y).0);
                    image::Rgba([c[0], c[1], c[2], op.get_pixel(x, y).0[0]])
                });
                Ok((png(&image::DynamicImage::ImageRgba8(out))?, "image/png"))
            }
        }
    }
}

/// An image file's bytes as glTF takes them: JPEG / PNG as they are, others as PNG.
pub(crate) fn file_bytes(p: &Path) -> Result<(Vec<u8>, &'static str)> {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => Ok((std::fs::read(p).with_context(|| format!("reading {}", p.display()))?, "image/jpeg")),
        "png" => Ok((std::fs::read(p).with_context(|| format!("reading {}", p.display()))?, "image/png")),
        _ => Ok((png(&open(p)?)?, "image/png")),
    }
}

pub fn write(c: &Character, opts: &GlbOptions) -> Result<Vec<u8>> {
    let mut b = Buf::default();
    let nj = c.joints.len();

    // images (each once), encoded in parallel
    let mut imgs: Vec<Img> = Vec::new();
    let img_of = |i: Img, imgs: &mut Vec<Img>| match imgs.iter().position(|x| *x == i) {
        Some(k) => k,
        None => {
            imgs.push(i);
            imgs.len() - 1
        }
    };
    let mut mat_imgs = Vec::new();
    for m in &c.materials {
        let base = match (&m.diffuse, &m.opacity) {
            (d, Some(o)) => Some(img_of(Img::Merge(d.clone(), o.clone()), &mut imgs)),
            (Some(d), None) => Some(img_of(Img::File(d.clone()), &mut imgs)),
            (None, None) => None,
        };
        let normal = m.normal.as_ref().map(|n| img_of(Img::File(n.clone()), &mut imgs));
        mat_imgs.push((base, normal));
    }
    let encoded: Vec<(Vec<u8>, &str)> = imgs.par_iter().map(Img::encode).collect::<Result<_>>()?;
    let mut images = Vec::new();
    for (i, (bytes, mime)) in imgs.iter().zip(&encoded) {
        let view = b.view(bytes, None);
        images.push(json!({"bufferView": view, "mimeType": mime, "name": i.name()}));
    }
    let textures: Vec<Value> = (0..images.len()).map(|i| json!({"sampler": 0, "source": i})).collect();

    // materials
    let cc5 = opts.cc5_json.as_deref().map(Cc5Json::load).transpose()?;
    let mut maps = cc5.as_ref().map(|j| {
        let dir = opts.maps_dir.clone().unwrap_or_else(|| j.path.with_extension("cc5maps"));
        Maps::new(&j.path, dir)
    });
    let mut materials = Vec::new();
    for (m, (base, normal)) in c.materials.iter().zip(&mat_imgs) {
        let mut factor = m.factor;
        let mut blend = m.opacity.is_some() || factor[3] < 1.0;
        let mut v = json!({"name": m.name, "doubleSided": true,
                           "pbrMetallicRoughness": {"metallicFactor": 0.0, "roughnessFactor": m.roughness}});
        if let (Some(j), Some(maps)) = (&cc5, &mut maps) {
            j.apply(&m.name, &mut factor, &mut blend);
            if let Some(x) = j.extras(&m.name, maps)? {
                v["extras"] = json!({"cc5": x});
            }
        }
        if let Some(t) = base {
            v["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index": t});
        }
        if factor != [1.0; 4] {
            v["pbrMetallicRoughness"]["baseColorFactor"] = json!(factor);
        }
        if let Some(t) = normal {
            v["normalTexture"] = json!({"index": t});
        }
        if blend {
            v["alphaMode"] = json!("BLEND");
        }
        materials.push(v);
    }
    if let Some(m) = &maps {
        for p in &m.missing {
            eprintln!("  cc5 map missing: {}", p.display());
        }
    }

    // nodes: joints, then mesh nodes, then the "Armature" root
    let mut nodes = Vec::new();
    for (i, j) in c.joints.iter().enumerate() {
        let local = j.parent.map_or(j.world, |p| c.joints[p].world.inverse() * j.world);
        let (s, r, t) = local.to_scale_rotation_translation();
        let children: Vec<usize> = (0..nj).filter(|&k| c.joints[k].parent == Some(i)).collect();
        let mut n = json!({"name": j.name, "translation": t.to_array(), "rotation": r.normalize().to_array(), "scale": s.to_array()});
        if !children.is_empty() {
            n["children"] = json!(children);
        }
        nodes.push(n);
    }
    let ibm: Vec<f32> = c.joints.iter().flat_map(|j| j.world.inverse().to_cols_array()).collect();
    let ibm = b.f32s(&ibm, 16, "MAT4", None);
    let wide = nj > 256;
    let mut meshes = Vec::new();
    for (mi, m) in c.meshes.iter().enumerate() {
        let mut prims = Vec::new();
        for p in &m.prims {
            let ccidx: Vec<f32> = p.cp.iter().map(|&i| i as f32).collect();
            let a_idx = b.f32s(&ccidx, 1, "SCALAR", Some(ARRAY));
            let a_pos = b.vec3s(&p.pos, true);
            let a_nrm = b.vec3s(&p.nrm, false);
            let uv: Vec<f32> = p.uv.iter().flat_map(|u| u.to_array()).collect();
            let a_uv = b.f32s(&uv, 2, "VEC2", Some(ARRAY));
            let (mut js, mut ws) = (Vec::new(), Vec::new());
            for &cp in &p.cp {
                let mut w = m.weights[cp as usize].clone();
                w.sort_by(|a, b| b.1.total_cmp(&a.1));
                w.truncate(4);
                let sum: f32 = w.iter().map(|x| x.1).sum();
                if w.is_empty() || sum <= 0.0 {
                    w = vec![(0, 1.0)];
                }
                let sum: f32 = w.iter().map(|x| x.1).sum();
                for k in 0..4 {
                    let (j, wt) = w.get(k).map_or((0, 0.0), |&(j, wt)| (j, wt / sum));
                    js.push(j);
                    ws.push(wt);
                }
            }
            let a_j = if wide {
                let bytes: Vec<u8> = js.iter().flat_map(|&j| (j as u16).to_le_bytes()).collect();
                b.accessor(&bytes, 5123, p.cp.len(), "VEC4", Some(ARRAY))
            } else {
                let bytes: Vec<u8> = js.iter().map(|&j| j as u8).collect();
                b.accessor(&bytes, 5121, p.cp.len(), "VEC4", Some(ARRAY))
            };
            let a_w = b.f32s(&ws, 4, "VEC4", Some(ARRAY));
            let a_i = if p.cp.len() <= u16::MAX as usize {
                let bytes: Vec<u8> = p.tris.iter().flat_map(|&i| (i as u16).to_le_bytes()).collect();
                b.accessor(&bytes, 5123, p.tris.len(), "SCALAR", Some(ELEMENTS))
            } else {
                let bytes: Vec<u8> = p.tris.iter().flat_map(|&i| i.to_le_bytes()).collect();
                b.accessor(&bytes, 5125, p.tris.len(), "SCALAR", Some(ELEMENTS))
            };
            let mut prim = json!({"attributes": {"_CCIDX": a_idx, "POSITION": a_pos, "NORMAL": a_nrm, "TEXCOORD_0": a_uv,
                                                 "JOINTS_0": a_j, "WEIGHTS_0": a_w}, "indices": a_i});
            if let Some(mat) = p.material {
                prim["material"] = json!(mat);
            }
            prims.push(prim);
        }
        meshes.push(json!({"name": m.name, "primitives": prims}));
        nodes.push(json!({"name": m.name, "mesh": mi, "skin": 0}));
    }
    let mut root_children: Vec<usize> = (0..nj).filter(|&k| c.joints[k].parent.is_none()).collect();
    root_children.extend(nj..nj + c.meshes.len());
    nodes.push(json!({"name": "Armature", "children": root_children}));
    let root = nodes.len() - 1;

    while b.bin.len() % 4 != 0 {
        b.bin.push(0);
    }
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": "cc5-fbx (ufbx)"},
        "scene": 0,
        "scenes": [{"name": "Scene", "nodes": [root]}],
        "nodes": nodes,
        "meshes": meshes,
        "skins": [{"name": "Armature", "joints": (0..nj).collect::<Vec<_>>(), "inverseBindMatrices": ibm}],
        "accessors": b.accessors,
        "bufferViews": b.views,
        "buffers": [{"byteLength": b.bin.len()}],
    });
    if !materials.is_empty() {
        doc["materials"] = json!(materials);
    }
    if !images.is_empty() {
        doc["images"] = json!(images);
        doc["textures"] = json!(textures);
        doc["samplers"] = json!([{"magFilter": 9729, "minFilter": 9987}]);
    }
    Ok(glb_bytes(&doc, &b.bin))
}

pub(crate) fn glb_bytes(doc: &Value, bin: &[u8]) -> Vec<u8> {
    let mut js = serde_json::to_vec(doc).unwrap();
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(bin);
    out
}
