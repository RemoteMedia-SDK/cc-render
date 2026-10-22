//! A character GLB (or CC5 FBX export) as the converter reads it: mesh objects (one per mesh node,
//! primitives per material), materials with their CC5 settings
//! (`extras.cc5`, glb_cc5_materials.py) and embedded images, and the skin.
//!
//! Positions are the accessors' (metres, Y up: the CC5 GLBs keep their meshes in
//! world space and scale only the armature).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use glam::{Mat4, Vec2, Vec3};
use serde_json::Value;

pub struct Source {
    pub path: PathBuf,
    pub doc: gltf::Document,
    pub blob: Vec<u8>,
    pub objects: Vec<Object>,
    pub materials: Vec<Material>,
    /// Decoded images (embedded by index, files by path), kept for rebuilds.
    images: Mutex<HashMap<String, Arc<image::RgbaImage>>>,
}

/// A mesh node (Blender's "object").
pub struct Object {
    pub name: String,
    pub prims: Vec<Prim>,
}

pub struct Prim {
    pub material: Option<usize>,
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub tris: Vec<[u32; 3]>,
    /// Indices into the skin's joint list.
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

pub struct Material {
    pub name: String,
    /// `extras.cc5`: {shader, maps, srgb, vars}.
    pub cc5: Option<Value>,
    pub base_image: Option<usize>,
    pub base_factor: [f32; 4],
}

impl Source {
    /// A GLB, or a CC5 FBX export (converted in memory by cc5-fbx; CC5's
    /// material settings from the sidecar `<name>.json`, maps written to
    /// `<name>.cc5maps/`).
    pub fn load(path: &Path) -> Result<Self> {
        let is_fbx = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fbx"));
        let bytes = if is_fbx {
            let json = path.with_extension("json");
            let opts = cc5_fbx::GlbOptions { cc5_json: json.exists().then_some(json), maps_dir: Some(path.with_extension("cc5maps")) };
            cc5_fbx::Character::load(path)?.to_glb(&opts)?
        } else {
            std::fs::read(path).with_context(|| format!("reading {}", path.display()))?
        };
        Self::from_bytes(path, bytes)
    }

    /// A GLB already in memory (`path`: where it came from; relative map paths
    /// in `extras.cc5` resolve against its folder).
    pub fn from_bytes(path: &Path, bytes: Vec<u8>) -> Result<Self> {
        let g = gltf::Gltf::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
        let blob = g.blob.clone().ok_or_else(|| anyhow!("{}: no binary chunk", path.display()))?;
        let doc = g.document;
        let materials = doc
            .materials()
            .map(|m| Material {
                name: m.name().unwrap_or("").to_string(),
                cc5: m.extras().as_ref().and_then(|r| serde_json::from_str::<Value>(r.get()).ok()).and_then(|v| v.get("cc5").cloned()),
                base_image: m.pbr_metallic_roughness().base_color_texture().map(|t| t.texture().source().index()),
                base_factor: m.pbr_metallic_roughness().base_color_factor(),
            })
            .collect();
        let mut objects = Vec::new();
        for node in doc.nodes() {
            let Some(mesh) = node.mesh() else { continue };
            let name = node.name().or(mesh.name()).unwrap_or("").to_string();
            let mut prims = Vec::new();
            for p in mesh.primitives() {
                if p.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let r = p.reader(|b| (b.index() == 0).then_some(blob.as_slice()));
                let pos: Vec<Vec3> = r.read_positions().ok_or_else(|| anyhow!("{name}: no positions"))?.map(Vec3::from).collect();
                let n = pos.len();
                let nrm = r.read_normals().map(|i| i.map(Vec3::from).collect()).unwrap_or_else(|| vec![Vec3::Y; n]);
                let uv = r.read_tex_coords(0).map(|i| i.into_f32().map(Vec2::from).collect()).unwrap_or_else(|| vec![Vec2::ZERO; n]);
                let idx: Vec<u32> = r.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..n as u32).collect());
                let tris = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
                let joints = r.read_joints(0).map(|i| i.into_u16().collect()).unwrap_or_else(|| vec![[0; 4]; n]);
                let weights = r.read_weights(0).map(|i| i.into_f32().collect()).unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 0.0]; n]);
                prims.push(Prim { material: p.material().index(), pos, nrm, uv, tris, joints, weights });
            }
            objects.push(Object { name, prims });
        }
        Ok(Self { path: path.to_path_buf(), doc, blob, objects, materials, images: Mutex::new(HashMap::new()) })
    }

    pub fn object(&self, name: &str) -> Option<&Object> {
        self.objects.iter().find(|o| o.name == name)
    }

    /// An embedded image's encoded bytes and MIME type.
    pub fn image_bytes(&self, index: usize) -> Result<(&[u8], String)> {
        let img = self.doc.images().nth(index).ok_or_else(|| anyhow!("no image {index}"))?;
        match img.source() {
            gltf::image::Source::View { view, mime_type } => {
                let o = view.offset();
                Ok((&self.blob[o..o + view.length()], mime_type.to_string()))
            }
            gltf::image::Source::Uri { uri, .. } => bail!("image {index}: external {uri} (expected embedded)"),
        }
    }

    /// An embedded image, decoded (cached).
    pub fn image_rgba(&self, index: usize) -> Result<Arc<image::RgbaImage>> {
        let k = format!("#{index}");
        if let Some(i) = self.images.lock().unwrap().get(&k) {
            return Ok(i.clone());
        }
        let (b, _) = self.image_bytes(index)?;
        let img = Arc::new(image::load_from_memory(b)?.to_rgba8());
        self.images.lock().unwrap().insert(k, img.clone());
        Ok(img)
    }

    /// An image file (e.g. a CC5 flow map next to the GLB), decoded (cached).
    pub fn file_rgba(&self, path: &Path) -> Result<Arc<image::RgbaImage>> {
        let k = path.to_string_lossy().to_string();
        if let Some(i) = self.images.lock().unwrap().get(&k) {
            return Ok(i.clone());
        }
        let img = Arc::new(image::open(path).with_context(|| format!("reading {}", path.display()))?.to_rgba8());
        self.images.lock().unwrap().insert(k, img.clone());
        Ok(img)
    }

    /// Joint node names of the first skin, in skin order.
    pub fn skin_joints(&self) -> Vec<String> {
        self.doc.skins().next().map(|s| s.joints().map(|j| j.name().unwrap_or("").to_string()).collect()).unwrap_or_default()
    }

    /// World matrix of every node (parents first).
    pub fn world_matrices(&self) -> Vec<Mat4> {
        let mut out = vec![Mat4::IDENTITY; self.doc.nodes().len()];
        fn walk(n: gltf::Node, parent: Mat4, out: &mut [Mat4]) {
            let m = parent * Mat4::from_cols_array_2d(&n.transform().matrix());
            out[n.index()] = m;
            for c in n.children() {
                walk(c, m, out);
            }
        }
        for scene in self.doc.scenes() {
            for n in scene.nodes() {
                walk(n, Mat4::IDENTITY, &mut out);
            }
        }
        out
    }
}
