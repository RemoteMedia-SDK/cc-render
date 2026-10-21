//! Character Creator 5 FBX exports, read with ufbx and written as the GLBs
//! cc-render loads: what Blender's FBX import + glTF export made
//! (`fbx_to_glb.py`), without Blender.
//!
//! - Space: metres, Y up. Meshes are in world space (bind pose) in identity mesh
//!   nodes; joints keep the FBX's own orientations, under an "Armature" root.
//! - Every vertex carries `_CCIDX`, its FBX control point (= CC5's vertex order),
//!   so seam-split vertices map back to CC5's mesh.
//! - Materials as Blender's importer + exporter made them: the diffuse map with
//!   the opacity map in its alpha (PNG), the normal map, roughness from the FBX
//!   shininess. With a CC5 sidecar JSON (`cc5json`), also CC5's tints and opacity
//!   and its shader settings as `extras.cc5` (glb_cc5_materials.py).
//! - The dump (`dump`) is `fbx_to_glb.py`'s: joints, parents, per-vertex weights
//!   and the rest positions in Blender's frame (Z up), for the pack builders.

pub mod cc5json;
pub mod dump;
pub mod expressions;
mod glb;
mod tga;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

pub use glb::GlbOptions;

pub struct Character {
    pub path: PathBuf,
    /// Parents before children.
    pub joints: Vec<Joint>,
    /// Sorted by name (Blender's object order).
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
}

pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    /// Bind-pose world matrix.
    pub world: Mat4,
}

pub struct Mesh {
    pub name: String,
    /// Control points, world space (bind pose).
    pub points: Vec<Vec3>,
    /// Per control point: (joint, weight) in joint order, weight > 0.
    pub weights: Vec<Vec<(u32, f32)>>,
    /// One per material used.
    pub prims: Vec<Prim>,
    /// Morph targets (facial expressions), at full weight.
    pub shapes: Vec<Shape>,
}

/// A morph target: control point and its world-space offset at weight 1.
pub struct Shape {
    pub name: String,
    pub deltas: Vec<(u32, Vec3)>,
}

/// Vertices split by (control point, normal, UV), triangles.
pub struct Prim {
    pub material: Option<usize>,
    pub cp: Vec<u32>,
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub tris: Vec<u32>,
}

pub struct Material {
    pub name: String,
    pub diffuse: Option<PathBuf>,
    pub opacity: Option<PathBuf>,
    pub normal: Option<PathBuf>,
    /// RGB: the diffuse colour when there is no diffuse map; A: 1 - transparency
    /// when there is no opacity map (as Blender's importer).
    pub factor: [f32; 4],
    /// Blender's importer: 1 - sqrt(shininess) / 10.
    pub roughness: f32,
}

fn mat4(m: &ufbx::Matrix) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(m.m00 as f32, m.m10 as f32, m.m20 as f32, 0.0),
        Vec4::new(m.m01 as f32, m.m11 as f32, m.m21 as f32, 0.0),
        Vec4::new(m.m02 as f32, m.m12 as f32, m.m22 as f32, 0.0),
        Vec4::new(m.m03 as f32, m.m13 as f32, m.m23 as f32, 1.0),
    )
}

fn v3(v: &ufbx::Vec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

impl Character {
    pub fn load(path: &Path) -> Result<Self> {
        let opts = ufbx::LoadOpts {
            target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
            target_unit_meters: 1.0,
            space_conversion: ufbx::SpaceConversion::ModifyGeometry,
            generate_missing_normals: true,
            ..Default::default()
        };
        let s = ufbx::load_file(&path.to_string_lossy(), opts).map_err(|e| anyhow!("{}: {:?}", path.display(), e.description))?;
        let dir = path.parent().unwrap_or(Path::new("."));

        // joints: the skeleton's bones plus any other node a skin binds to (CC5
        // accessories skin to their own mesh node); parents first
        let mut is_joint: HashMap<u32, Option<Mat4>> = HashMap::new();
        for n in s.nodes.iter() {
            if n.bone.is_some() {
                is_joint.insert(n.element.element_id, None);
            }
        }
        for c in s.skin_clusters.iter() {
            if let Some(b) = c.bone_node.as_ref() {
                is_joint.insert(b.element.element_id, Some(mat4(&c.bind_to_world)));
            }
        }
        let mut joints = Vec::new();
        let mut joint_of: HashMap<u32, usize> = HashMap::new();
        fn walk(n: &ufbx::Node, parent: Option<usize>, is_joint: &HashMap<u32, Option<Mat4>>, joints: &mut Vec<Joint>, joint_of: &mut HashMap<u32, usize>) {
            let mut p = parent;
            if let Some(bind) = is_joint.get(&n.element.element_id) {
                joint_of.insert(n.element.element_id, joints.len());
                joints.push(Joint { name: n.element.name.to_string(), parent, world: bind.unwrap_or_else(|| mat4(&n.node_to_world)) });
                p = Some(joints.len() - 1);
            }
            for c in n.children.iter() {
                walk(c, p, is_joint, joints, joint_of);
            }
        }
        walk(&s.root_node, None, &is_joint, &mut joints, &mut joint_of);

        let materials = s.materials.iter().map(|m| material(m, dir)).collect();

        let mut meshes = Vec::new();
        for node in s.nodes.iter() {
            let Some(mesh) = node.mesh.as_ref() else { continue };
            let g2w = mat4(&node.geometry_to_world);
            let nmat = Mat3::from_mat4(g2w).inverse().transpose();
            let points: Vec<Vec3> = mesh.vertices.iter().map(|p| g2w.transform_point3(v3(p))).collect();
            let weights = match mesh.skin_deformers.first() {
                Some(skin) => {
                    let cj: Vec<Option<u32>> = skin.clusters.iter().map(|c| c.bone_node.as_ref().and_then(|b| joint_of.get(&b.element.element_id)).map(|&j| j as u32)).collect();
                    skin.vertices
                        .iter()
                        .map(|sv| {
                            let mut w: Vec<(u32, f32)> = (sv.weight_begin..sv.weight_begin + sv.num_weights)
                                .filter_map(|i| {
                                    let w = &skin.weights[i as usize];
                                    let j = cj[w.cluster_index as usize]?;
                                    (w.weight > 0.0).then_some((j, w.weight as f32))
                                })
                                .collect();
                            w.sort_by_key(|x| x.0); // joint order, as Blender's vertex groups
                            w
                        })
                        .collect()
                }
                None => {
                    // unskinned: follow the nearest joint above the node
                    let mut j = None;
                    let mut p = node.parent.as_ref();
                    while let Some(n) = p {
                        if let Some(&i) = joint_of.get(&n.element.element_id) {
                            j = Some(i as u32);
                            break;
                        }
                        p = n.parent.as_ref();
                    }
                    vec![j.map(|j| vec![(j, 1.0)]).unwrap_or_default(); points.len()]
                }
            };
            let mat_index: Vec<usize> = mesh.materials.iter().map(|m| m.element.typed_id as usize).collect();
            let mut prims: Vec<Prim> = Vec::new();
            let mut keys: Vec<HashMap<(u32, [u32; 3], [u32; 2]), u32>> = Vec::new();
            let mut tri = vec![0u32; mesh.max_face_triangles * 3];
            let has_uv = mesh.vertex_uv.exists;
            for (fi, face) in mesh.faces.iter().enumerate() {
                let material = mesh.face_material.get(fi).and_then(|&m| mat_index.get(m as usize).copied());
                let pi = match prims.iter().position(|p| p.material == material) {
                    Some(i) => i,
                    None => {
                        prims.push(Prim { material, cp: vec![], pos: vec![], nrm: vec![], uv: vec![], tris: vec![] });
                        keys.push(HashMap::new());
                        prims.len() - 1
                    }
                };
                let nt = mesh.triangulate_face(&mut tri, *face) as usize;
                let p = &mut prims[pi];
                for &c in &tri[..nt * 3] {
                    let c = c as usize;
                    let cp = mesh.vertex_indices[c];
                    let n = (nmat * v3(&mesh.vertex_normal.values[mesh.vertex_normal.indices[c] as usize])).normalize_or_zero();
                    let uv = if has_uv {
                        let t = &mesh.vertex_uv.values[mesh.vertex_uv.indices[c] as usize];
                        Vec2::new(t.x as f32, 1.0 - t.y as f32)
                    } else {
                        Vec2::ZERO
                    };
                    let key = (cp, n.to_array().map(f32::to_bits), uv.to_array().map(f32::to_bits));
                    let v = *keys[pi].entry(key).or_insert_with(|| {
                        p.cp.push(cp);
                        p.pos.push(points[cp as usize]);
                        p.nrm.push(n);
                        p.uv.push(uv);
                        p.cp.len() as u32 - 1
                    });
                    p.tris.push(v);
                }
            }
            prims.sort_by_key(|p| p.material);
            let lin = Mat3::from_mat4(g2w);
            let mut shapes = Vec::new();
            for bd in mesh.blend_deformers.iter() {
                for ch in bd.channels.iter() {
                    // the shape at full weight (in-betweens are not used)
                    let shape: Option<&ufbx::BlendShape> = ch.keyframes.iter().find(|k| (k.target_weight - 1.0).abs() < 1e-6).map(|k| &*k.shape).or(ch.target_shape.as_deref());
                    let Some(shape) = shape else { continue };
                    let deltas = shape.offset_vertices.iter().zip(shape.position_offsets.iter()).map(|(&i, d)| (i, lin * v3(d))).collect();
                    shapes.push(Shape { name: ch.element.name.to_string(), deltas });
                }
            }
            meshes.push(Mesh { name: node.element.name.to_string(), points, weights, prims, shapes });
        }
        meshes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self { path: path.to_path_buf(), joints, meshes, materials })
    }

    pub fn mesh(&self, name: &str) -> Option<&Mesh> {
        self.meshes.iter().find(|m| m.name == name)
    }

    /// The GLB (see the crate docs).
    pub fn to_glb(&self, opts: &GlbOptions) -> Result<Vec<u8>> {
        glb::write(self, opts)
    }
}

fn material(m: &ufbx::Material, dir: &Path) -> Material {
    let tex = |props: &[&str]| {
        m.textures.iter().find(|t| props.contains(&t.material_prop.as_ref())).and_then(|t| texture_path(&t.texture, dir))
    };
    let diffuse = tex(&["DiffuseColor"]);
    let opacity = tex(&["TransparentColor", "TransparencyFactor"]);
    let normal = tex(&["NormalMap", "Bump"]);
    let dc = &m.fbx.diffuse_color.value_vec4;
    let rgb = if diffuse.is_some() { [1.0; 3] } else { [dc.x as f32, dc.y as f32, dc.z as f32] };
    let a = if opacity.is_some() { 1.0 } else { 1.0 - m.fbx.transparency_factor.value_vec4.x as f32 };
    let shininess = m.fbx.specular_exponent.value_vec4.x.max(0.0) as f32;
    Material {
        name: m.element.name.to_string(),
        diffuse,
        opacity,
        normal,
        factor: [rgb[0], rgb[1], rgb[2], a.clamp(0.0, 1.0)],
        roughness: (1.0 - shininess.sqrt() / 10.0).clamp(0.0, 1.0),
    }
}

/// A texture's file: as written, else relative to the FBX, else in its `.fbm`.
fn texture_path(t: &ufbx::Texture, dir: &Path) -> Option<PathBuf> {
    let fix = |s: &str| s.replace('\\', "/");
    let cands = [
        PathBuf::from(fix(&t.absolute_filename)),
        dir.join(fix(&t.relative_filename)),
        PathBuf::from(fix(&t.filename)),
    ];
    cands.into_iter().find(|p| !p.as_os_str().is_empty() && p.is_file()).or_else(|| {
        let name = Path::new(&fix(&t.relative_filename)).file_name()?.to_owned();
        std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path().join(&name)).find(|p| p.is_file())
    })
}

/// Read a CC5 FBX and write its GLB.
pub fn convert(fbx: &Path, glb_path: &Path, opts: &GlbOptions) -> Result<Character> {
    let c = Character::load(fbx)?;
    let bytes = c.to_glb(opts)?;
    std::fs::write(glb_path, bytes).with_context(|| format!("writing {}", glb_path.display()))?;
    Ok(c)
}
