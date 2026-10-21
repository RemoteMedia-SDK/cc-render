//! `--hair-lib dir`: Smart-Hair-style hair parts on the edited character.
//!
//! The library (`cc5-hair`, settings in `remotemedia-sdk/scripts/avatars/cc5_editor/hair/`):
//! - `catalog.json`: `{"parts": [{"id", "slot", "kind": "scalp"|"beard"|"brow", "file",
//!   "follow"?: {mesh: "head_box"}}]}`, one GLB per part: card / strand meshes
//!   skinned to the CC skeleton, `extras.cc5` RLHair materials (maps under `atlas/`).
//!   Strands are centre lines (glTF LINES), drawn as GPU ribbons ([`Ribbon`]);
//! - `styles.json` (optional): `{"styles": [{"name", "parts": [part ids]}]}`;
//! - `regions.json`: CC_Base_Body anchor vertices (`head`, `face`, `head_box`;
//!   `hair_regions.py`).
//!
//! One part per slot. A chosen part is spawned, its skin re-bound to our joints
//! by name (as `--style` assets are) and every mesh bound with a
//! [`Conform`]: scalp parts to the head, beards and brows to the face,
//! the meshes of layers CC5 moves rigidly (`follow`) to the head's bounding box.
//! `apply_shape` then moves the cards with the body (shape + expressions) and
//! compensates their inverse bind matrices like ours. Choosing a scalp / brow
//! part hides the character's own hair / brows.

use super::*;
use cc5_shape::{Conform, ConformOptions};
use serde::Deserialize;

/// Slots in UI order.
pub const SLOTS: [&str; 10] = ["Base", "Top", "Rear", "Bangs", "Accessory", "Beard", "Moustache", "Goatee", "Sideburns", "Brows"];

#[derive(Deserialize, Clone)]
pub struct PartInfo {
    pub id: String,
    pub slot: String,
    #[serde(default = "scalp")]
    pub kind: String,
    pub file: String,
    /// the part brings its own painted skin layers (CC5 brows: Brows_Base /
    /// Brows_Color), so the character's are hidden too
    #[serde(default)]
    pub painted_layers: bool,
    /// meshes conformed otherwise than by `kind`: `"head_box"` for hair layers
    /// CC5 moves rigidly with the head (`cc5_layer_follow.py`),
    /// `"bone:<joint>@x,y,z"` for layers CC5 attaches to a bone at that world
    /// point (needs `bone_xf.json`)
    #[serde(default)]
    pub follow: HashMap<String, String>,
}

fn scalp() -> String {
    "scalp".into()
}

#[derive(Deserialize, Clone)]
pub struct StyleInfo {
    pub name: String,
    pub parts: Vec<String>,
    /// hide the character's own hair even without a scalp part (beards / brows
    /// alone, as on CC5's bald base characters)
    #[serde(default)]
    pub bald: bool,
}

#[derive(Deserialize)]
struct Catalog {
    parts: Vec<PartInfo>,
}

#[derive(Deserialize, Default)]
struct Styles {
    styles: Vec<StyleInfo>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct Regions {
    scalp: Vec<u32>,
    face: Vec<u32>,
    #[serde(default)]
    head_box: Vec<u32>,
    /// the whole head: scalp hair's anchors (older libraries: `scalp`)
    #[serde(default)]
    head: Vec<u32>,
}

/// `bone_xf.json` (`bone_xf_lib.py`, captured by `cc5_capture_bone_xf.py`): the
/// world rotation and scale of CC5's bones per slider state, which the pack
/// (joint translations only) lacks. Mixed like the pack mixes translations.
#[derive(Deserialize, Default)]
pub(crate) struct BoneXf {
    /// bone -> [qx, qy, qz, qw, sx, sy, sz] at the captured shape
    base: HashMap<String, [f32; 7]>,
    /// slider id -> "min" | "max" -> bone -> [rotation vector, scale ratio - 1]
    sliders: HashMap<String, HashMap<String, HashMap<String, [f32; 6]>>>,
}

impl BoneXf {
    /// `bone`'s world (rotation, scale) at the captured shape and for `weights`:
    /// rotation vectors and scale ratios add, each slider's scaled by its weight's
    /// fraction of the captured state.
    fn mixed(&self, bone: &str, pack: &cc5_shape::ShapePack, weights: &[f32]) -> Option<((Quat, Vec3), (Quat, Vec3))> {
        let b = self.base.get(bone)?;
        let (r0, s0) = (Quat::from_xyzw(b[0], b[1], b[2], b[3]), Vec3::new(b[4], b[5], b[6]));
        let (mut rv, mut ds) = (Vec3::ZERO, Vec3::ZERO);
        for (s, &w) in pack.sliders.iter().zip(weights) {
            let d = w - s.base;
            let st = if d > 0.0 { "max" } else { "min" };
            if d.abs() < 1e-9 {
                continue;
            }
            let (Some(state), Some(v)) = (s.states.get(st), self.sliders.get(&s.id).and_then(|m| m.get(st)?.get(bone))) else {
                continue;
            };
            let t = d / (state.weight - s.base);
            rv += Vec3::new(v[0], v[1], v[2]) * t;
            ds += Vec3::new(v[3], v[4], v[5]) * t;
        }
        Some(((r0, s0), (Quat::from_scaled_axis(rv) * r0, s0 * (Vec3::ONE + ds))))
    }
}

/// A mesh CC5 attaches to a bone (the Viking braid rides the jaw): moved
/// rigidly by the bone's rotation about `pivot` (its attach point, world),
/// which follows the bone's full transform (scale included).
pub(crate) struct BoneFollow {
    joint: String,
    /// our rig's joint (its bind position and pack delta)
    rig: usize,
    pivot: Vec3,
    world: Vec<Vec3>,
}

pub(crate) struct PartMesh {
    pub(crate) handle: Handle<Mesh>,
    /// Bind positions (mesh space): the vertices, or a ribbon mesh's strand points.
    pub(crate) base: Vec<[f32; 3]>,
    pub(crate) to_mesh: Mat3,
    pub(crate) conform: Conform,
    pub(crate) bone: Option<BoneFollow>,
    pub(crate) ribbon: Option<Ribbon>,
}

/// A strand mesh drawn as GPU ribbons (cc5_materials' `Cc5Hair::ribbons`): two
/// vertices per strand point, both at the point, TANGENT = (strand direction,
/// -/+ half-width); the hair vertex shaders move them apart facing the view.
/// The conform moves only the points; [`Ribbon::write`] expands them.
pub(crate) struct Ribbon {
    /// Per point: its neighbours along the strand (itself at the ends).
    prev: Vec<u32>,
    next: Vec<u32>,
    half: Vec<f32>,
}

impl Ribbon {
    fn new(n: usize, segs: &[[u32; 2]], half: Vec<f32>) -> Self {
        let mut prev: Vec<u32> = (0..n as u32).collect();
        let mut next = prev.clone();
        for &[a, b] in segs {
            next[a as usize] = b;
            prev[b as usize] = a;
        }
        Self { prev, next, half }
    }

    fn direction(&self, pts: &[[f32; 3]], i: usize) -> Vec3 {
        let d = Vec3::from(pts[self.next[i] as usize]) - Vec3::from(pts[self.prev[i] as usize]);
        d.try_normalize().unwrap_or(Vec3::Y)
    }

    /// Positions and tangents of the ribbon vertices for strand points `pts`.
    pub(crate) fn write(&self, mesh: &mut Mesh, pts: &[[f32; 3]]) {
        use rayon::prelude::*;
        let n = 2 * pts.len();
        // in place when the mesh has them (every reshape), else new
        let mut pos = match mesh.remove_attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(v)) if v.len() == n => v,
            _ => vec![[0.0; 3]; n],
        };
        let mut tan = match mesh.remove_attribute(Mesh::ATTRIBUTE_TANGENT) {
            Some(VertexAttributeValues::Float32x4(v)) if v.len() == n => v,
            _ => vec![[0.0; 4]; n],
        };
        pos.par_chunks_mut(2).zip(tan.par_chunks_mut(2)).enumerate().for_each(|(i, (p, t))| {
            let d = self.direction(pts, i);
            p[0] = pts[i];
            p[1] = pts[i];
            t[0] = d.extend(-self.half[i]).to_array();
            t[1] = d.extend(self.half[i]).to_array();
        });
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tan);
    }
}

/// The ribbon render mesh of strand centre lines (mesh space = the part's bind
/// space, where the lighting normals point out from the head).
#[allow(clippy::too_many_arguments)]
pub(crate) fn ribbon_mesh(
    pts: &[[f32; 3]],
    uv: &[[f32; 2]],
    col: &[[f32; 4]],
    joints: &[[u16; 4]],
    weights: &[[f32; 4]],
    half: Vec<f32>,
    segs: &[[u32; 2]],
) -> (Mesh, Ribbon) {
    let r = Ribbon::new(pts.len(), segs, half);
    let two = |v: &[[f32; 2]]| v.iter().flat_map(|x| [*x, *x]).collect::<Vec<_>>();
    let nrm: Vec<[f32; 3]> = (0..pts.len())
        .flat_map(|i| {
            // out from the head, across the strand (fixed ribbons showed their back
            // face, whose flipped normal was this)
            let d = r.direction(pts, i);
            let out = Vec3::from(pts[i]) - cc5_hair::HEAD_CENTER;
            let n = (out - d * d.dot(out)).try_normalize().unwrap_or_else(|| d.any_orthonormal_vector()).to_array();
            [n, n]
        })
        .collect();
    let mut mesh = Mesh::new(bevy::render::mesh::PrimitiveTopology::TriangleList, bevy::render::render_asset::RenderAssetUsages::default());
    r.write(&mut mesh, pts);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, two(uv));
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col.iter().flat_map(|x| [*x, *x]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints.iter().flat_map(|x| [*x, *x]).collect()));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights.iter().flat_map(|x| [*x, *x]).collect::<Vec<_>>());
    // two triangles per segment, front-facing towards the view (see hair_vertex.wgsl)
    let idx: Vec<u32> = segs.iter().flat_map(|&[a, b]| [2 * a, 2 * b + 1, 2 * b, 2 * a, 2 * a + 1, 2 * b + 1]).collect();
    mesh.insert_indices(bevy::render::mesh::Indices::U32(idx));
    (mesh, r)
}

/// A glTF LINES strand mesh (cc5-hair: TEXCOORD_1.x = half-width over
/// `HALF_WIDTH_MAX`) as a ribbon mesh; also returns the strand points.
fn ribbon_from_lines(m: &Mesh) -> Option<(Mesh, Ribbon, Vec<[f32; 3]>)> {
    use VertexAttributeValues as V;
    if m.primitive_topology() != bevy::render::mesh::PrimitiveTopology::LineList {
        return None;
    }
    let Some(V::Float32x3(pts)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else { return None };
    let n = pts.len();
    let Some(V::Float32x2(uv)) = m.attribute(Mesh::ATTRIBUTE_UV_0) else { return None };
    let half: Vec<f32> = match m.attribute(Mesh::ATTRIBUTE_UV_1) {
        Some(V::Float32x2(w)) => w.iter().map(|w| w[0] * cc5_hair::glb_out::HALF_WIDTH_MAX).collect(),
        _ => vec![5e-5; n],
    };
    let col = match m.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(V::Float32x4(c)) => c.clone(),
        _ => vec![[1.0; 4]; n],
    };
    let joints = match m.attribute(Mesh::ATTRIBUTE_JOINT_INDEX) {
        Some(V::Uint16x4(j)) => j.clone(),
        _ => vec![[0; 4]; n],
    };
    let weights = match m.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT) {
        Some(V::Float32x4(w)) => w.clone(),
        _ => vec![[1.0, 0.0, 0.0, 0.0]; n],
    };
    let idx: Vec<u32> = m.indices()?.iter().map(|i| i as u32).collect();
    let segs: Vec<[u32; 2]> = idx.chunks_exact(2).map(|s| [s[0], s[1]]).collect();
    let (mesh, r) = ribbon_mesh(pts, uv, &col, &joints, &weights, half, &segs);
    Some((mesh, r, pts.clone()))
}

/// The conform of a part mesh (world bind positions): by the part's kind, or
/// its `follow` mode ("head_box"; "bone:" meshes keep the kind's conform for
/// expressions).
pub(crate) fn conform_for(regions: &Regions, kind: &str, follow: Option<&str>, body: &[[f32; 3]], world: &[[f32; 3]]) -> anyhow::Result<Conform> {
    let (anchors, opts) = match (follow, kind) {
        (Some("head_box"), _) if !regions.head_box.is_empty() => (&regions.head_box, ConformOptions::HEAD_BOX),
        (_, "scalp") if !regions.head.is_empty() => (&regions.head, ConformOptions::SCALP),
        (_, "scalp") => (&regions.scalp, ConformOptions::SCALP),
        _ => (&regions.face, ConformOptions::SKIN),
    };
    Conform::bind(body, anchors, world, opts)
}

/// "bone:<joint>@x,y,z": the mesh rides that joint for shape (needs the joint on
/// our rig and `bone_xf.json`).
pub(crate) fn bone_follow(follow: Option<&str>, rig: &Rig, bones: Option<&BoneXf>, world: &[[f32; 3]]) -> Option<BoneFollow> {
    let (joint, at) = follow?.strip_prefix("bone:")?.split_once('@')?;
    let p: Vec<f32> = at.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let rig_i = rig.joints.iter().position(|j| j.name == joint)?;
    (p.len() == 3 && bones.is_some_and(|b| b.base.contains_key(joint))).then(|| BoneFollow {
        joint: joint.to_string(),
        rig: rig_i,
        pivot: Vec3::new(p[0], p[1], p[2]),
        world: world.iter().map(|w| Vec3::from(*w)).collect(),
    })
}

struct Spawned {
    root: Entity,
    frames: u32,
    ready: bool,
    meshes: Vec<PartMesh>,
    skins: Vec<SkinRec>,
}

#[derive(Resource)]
pub struct HairLib {
    pub(crate) dir: PathBuf,
    pub parts: Vec<PartInfo>,
    pub styles: Vec<StyleInfo>,
    pub(crate) regions: Regions,
    /// per-slider bone rotation / scale (optional `bone_xf.json`): `bone:` follows
    pub(crate) bones: Option<BoneXf>,
    /// slot -> chosen part (index into `parts`)
    pub chosen: HashMap<String, usize>,
    /// the applied style hides our own hair (`StyleInfo::bald`)
    pub bald: bool,
    spawned: HashMap<usize, Spawned>,
    /// CC_Base_Body bind positions (world), mesh-local (pack) order; from the rig.
    pub(crate) body: Option<Vec<[f32; 3]>>,
    pub status: String,
}

/// Root of a spawned hair part (index into `HairLib::parts`).
#[derive(Component)]
pub struct HairPartRoot(pub(crate) usize);

impl HairLib {
    pub fn load(dir: &std::path::Path) -> anyhow::Result<Self> {
        let dir = std::fs::canonicalize(dir)?;
        let read = |f: &str| -> anyhow::Result<Vec<u8>> {
            std::fs::read(dir.join(f)).map_err(|e| anyhow::anyhow!("{}: {e}", dir.join(f).display()))
        };
        let catalog: Catalog = serde_json::from_slice(&read("catalog.json")?)?;
        let styles: Styles = match read("styles.json") {
            Ok(b) => serde_json::from_slice(&b)?,
            Err(_) => Styles::default(),
        };
        let regions: Regions = serde_json::from_slice(&read("regions.json")?)?;
        let bones = match read("bone_xf.json") {
            Ok(b) => Some(serde_json::from_slice::<BoneXf>(&b)?),
            Err(_) => None,
        };
        let n = catalog.parts.len();
        Ok(Self {
            dir,
            status: format!("{n} hair parts, {} styles", styles.styles.len()),
            parts: catalog.parts,
            styles: styles.styles,
            regions,
            bones,
            chosen: HashMap::new(),
            bald: false,
            spawned: HashMap::new(),
            body: None,
        })
    }

    /// Choose a style's parts (all slots it does not name are cleared).
    pub fn apply_style(&mut self, name: &str) -> anyhow::Result<()> {
        let style = self.styles.iter().find(|s| s.name == name).ok_or_else(|| anyhow::anyhow!("no hair style {name:?}"))?;
        let mut chosen = HashMap::new();
        for id in &style.parts {
            let i = self.parts.iter().position(|p| &p.id == id).ok_or_else(|| anyhow::anyhow!("style {name}: no part {id}"))?;
            chosen.insert(self.parts[i].slot.clone(), i);
        }
        self.chosen = chosen;
        self.bald = style.bald;
        Ok(())
    }

    /// Every chosen part is spawned and bound.
    pub fn settled(&self) -> bool {
        self.chosen.values().all(|i| self.spawned.get(i).is_some_and(|s| s.ready))
    }

    fn kind_chosen(&self, kind: &str) -> bool {
        self.chosen.values().any(|&i| self.parts[i].kind == kind)
    }
}

impl HairLib {
    /// The catalog index of part `entry` (a `catalog.json` entry), added if new.
    pub(crate) fn part_index(&mut self, entry: &serde_json::Value) -> anyhow::Result<usize> {
        let info: PartInfo = serde_json::from_value(entry.clone())?;
        Ok(match self.parts.iter().position(|p| p.id == info.id) {
            Some(i) => {
                self.parts[i] = info;
                i
            }
            None => {
                self.parts.push(info);
                self.parts.len() - 1
            }
        })
    }

    /// Install a part built in memory as part `i` (spawned under `root`, already
    /// bound): replaces a spawned copy, whose root is returned for despawning.
    pub(crate) fn put_built(&mut self, i: usize, root: Entity, meshes: Vec<PartMesh>, skins: Vec<SkinRec>) -> Option<Entity> {
        let old = self.spawned.insert(i, Spawned { root, frames: 3, ready: true, meshes, skins });
        old.map(|s| s.root)
    }
}

/// Spawn / despawn parts to match the choice; hide our own hair and brows when replaced.
pub fn hair_sync(
    mut lib: ResMut<HairLib>,
    rig: Res<Rig>,
    assets: Res<AssetServer>,
    mut commands: Commands,
    prims: Query<(Entity, &Name, Option<&bevy::gltf::GltfMaterialName>), With<Mesh3d>>,
    parents: Query<&Parent>,
    ours: Query<(), With<OurCharacter>>,
    mut editor: ResMut<Editor>,
) {
    if !rig.ready {
        return;
    }
    let lib = &mut *lib;
    let chosen: Vec<usize> = lib.chosen.values().copied().collect();
    let gone: Vec<usize> = lib.spawned.keys().copied().filter(|i| !chosen.contains(i)).collect();
    for i in gone {
        if let Some(s) = lib.spawned.remove(&i) {
            commands.entity(s.root).despawn_recursive();
        }
    }
    for i in chosen {
        lib.spawned.entry(i).or_insert_with(|| {
            let path = bevy::asset::AssetPath::from_path(&lib.dir.join(&lib.parts[i].file)).into_owned();
            // hidden until bound (hair_bind): no unshaped hair, no strand lines before their ribbons
            let root = commands.spawn((SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))), HairPartRoot(i), Visibility::Hidden)).id();
            Spawned { root, frames: 0, ready: false, meshes: Vec::new(), skins: Vec::new() }
        });
    }
    // our own hair / brows: hidden while a library part replaces them
    let (hide_hair, hide_brows) = (lib.kind_chosen("scalp") || lib.bald, lib.kind_chosen("brow"));
    let hide_painted = lib.chosen.values().any(|&i| lib.parts[i].kind == "brow" && lib.parts[i].painted_layers);
    for (e, name, material) in &prims {
        if !ours.contains(root_of(e, &parents)) {
            continue;
        }
        let n = name.as_str().to_lowercase();
        let m = material.map(|m| m.0.to_lowercase()).unwrap_or_default();
        let hide = if n.starts_with("cc_base_") {
            continue;
        } else if n.contains("hair") || n.contains("bang") {
            hide_hair
        } else if n.contains("brow") {
            // the brow hair cards; CC5's painted brow layers (Brows_Base /
            // Brows_Color) stay under library brows that have none of their own
            hide_brows && (hide_painted || !(m.contains("base") || m.contains("color")))
        } else {
            continue;
        };
        commands.entity(e).insert(if hide { Visibility::Hidden } else { Visibility::Inherited });
    }
    if lib.body.is_none() {
        lib.body = body_bind_world(&rig, &editor);
        if lib.body.is_none() && !lib.chosen.is_empty() {
            editor.status = "hair: CC_Base_Body not found in the level-0 rig (hair parts need it; not with --hd)".into();
        }
    }
}

/// CC_Base_Body bind positions in world space, pack (mesh-local) order.
fn body_bind_world(rig: &Rig, editor: &Editor) -> Option<Vec<[f32; 3]>> {
    if editor.hd.is_some() {
        return None;
    }
    let pm = editor.pack.meshes.iter().find(|m| m.name == "CC_Base_Body")?;
    let mut out = vec![[0.0; 3]; pm.count as usize];
    let mut seen = 0;
    for m in rig.meshes.iter().filter(|m| m.name.split('.').next() == Some("CC_Base_Body")) {
        for (p, &g) in m.base.iter().zip(&m.global) {
            if let Some(o) = out.get_mut((g - pm.offset) as usize) {
                *o = m.bind_space.transform_point3(Vec3::from(*p)).into();
                seen += 1;
            }
        }
    }
    (seen > 0).then_some(out)
}

/// Bind newly spawned parts: skins to our joints, meshes to the body (conform).
#[allow(clippy::too_many_arguments)]
pub fn hair_bind(
    mut lib: ResMut<HairLib>,
    rig: Res<Rig>,
    mut editor: ResMut<Editor>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut ibms: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut prims: Query<(Entity, &Mesh3d, &mut SkinnedMesh, Option<&Name>)>,
    joints: Query<(&Name, &GlobalTransform), Without<Mesh3d>>,
    parents: Query<&Parent>,
    roots: Query<&HairPartRoot>,
    mut commands: Commands,
) {
    let lib = &mut *lib;
    let Some(body) = lib.body.as_ref() else { return };
    if !rig.ready || lib.spawned.values().all(|s| s.ready) {
        return;
    }
    let our_joint: HashMap<&str, (usize, Entity, Mat4)> =
        rig.joints.iter().enumerate().map(|(i, j)| (j.name.as_str(), (i, j.entity, Mat4::from(j.base_world)))).collect();
    // primitives per spawned part
    let mut per_part: HashMap<usize, Vec<Entity>> = HashMap::new();
    for (e, ..) in prims.iter() {
        if let Ok(r) = roots.get(root_of(e, &parents)) {
            per_part.entry(r.0).or_default().push(e);
        }
    }
    for (&i, s) in lib.spawned.iter_mut().filter(|(_, s)| !s.ready) {
        let ents = per_part.get(&i).cloned().unwrap_or_default();
        let loaded = !ents.is_empty()
            && ents.iter().all(|&e| {
                prims.get(e).is_ok_and(|(_, m, sk, _)| meshes.get(&m.0).is_some() && ibms.get(&sk.inverse_bindposes).is_some())
            });
        s.frames = if loaded { s.frames + 1 } else { 0 };
        if s.frames < 3 {
            continue; // let the part's scene finish spawning and propagate transforms
        }
        let part = &lib.parts[i];
        let mut unbound = 0;
        let started = std::time::Instant::now();
        for e in ents {
            let Ok((_, mesh3d, mut skin, name)) = prims.get_mut(e) else { continue };
            // primitive entities are named `<mesh>` or `<mesh>.<primitive>`
            let mesh_name = name.map(|n| n.as_str()).unwrap_or("");
            let follow = part
                .follow
                .iter()
                .find(|(m, _)| mesh_name == m.as_str() || mesh_name.strip_prefix(m.as_str()).is_some_and(|r| r.starts_with('.')))
                .map(|(_, f)| f.as_str());
            let old = ibms.get(&skin.inverse_bindposes).unwrap().to_vec();
            let Ok((_, j0)) = joints.get(skin.joints[0]) else { continue };
            let bind_space = j0.compute_matrix() * old[0];
            // re-bind to our joints by name (see style_setup)
            let mut new = old.clone();
            let mut rig_joints = Vec::with_capacity(skin.joints.len());
            for (k, j) in skin.joints.iter_mut().enumerate() {
                let Ok((jn, jg)) = joints.get(*j) else { continue };
                match our_joint.get(jn.as_str()) {
                    Some(&(ri, ours_e, ours_w)) => {
                        let theirs_w = jg.compute_matrix();
                        let shift = Mat4::from_translation(ours_w.w_axis.truncate() - theirs_w.w_axis.truncate());
                        new[k] = ours_w.inverse() * shift * theirs_w * old[k];
                        *j = ours_e;
                        rig_joints.push(ri);
                    }
                    None => unbound += 1,
                }
            }
            let handle = ibms.add(SkinnedMeshInverseBindposes::from(new.clone()));
            skin.inverse_bindposes = handle.clone();
            if rig_joints.len() == skin.joints.len() {
                s.skins.push(SkinRec { handle, joints: rig_joints, base: new });
            }
            // strand centre lines: drawn as GPU ribbons, shaped by their points
            let lines = meshes.get(&mesh3d.0).and_then(ribbon_from_lines);
            let (pos, ribbon) = match lines {
                Some((mesh, ribbon, pts)) => {
                    *meshes.get_mut(&mesh3d.0).unwrap() = mesh;
                    (pts, Some(ribbon))
                }
                None => {
                    let Some(VertexAttributeValues::Float32x3(pos)) = meshes.get(&mesh3d.0).and_then(|m| m.attribute(Mesh::ATTRIBUTE_POSITION)) else {
                        continue;
                    };
                    (pos.clone(), None)
                }
            };
            let pos = &pos;
            let world: Vec<[f32; 3]> = pos.iter().map(|p| bind_space.transform_point3(Vec3::from(*p)).into()).collect();
            // "bone:<joint>@x,y,z": rigid with that bone for shape (the conform
            // still carries expressions)
            let bone = bone_follow(follow, &rig, lib.bones.as_ref(), &world);
            if follow.is_some_and(|f| f.starts_with("bone:")) && bone.is_none() {
                lib.status = format!("{}: {mesh_name}: bone follow needs its joint and bone_xf.json; conformed instead", part.id);
            }
            match conform_for(&lib.regions, &part.kind, follow, body, &world) {
                Ok(conform) => s.meshes.push(PartMesh {
                    handle: mesh3d.0.clone(),
                    base: pos.clone(),
                    to_mesh: Mat3::from_mat4(bind_space.inverse()),
                    conform,
                    bone,
                    ribbon,
                }),
                Err(e) => lib.status = format!("{}: {e:#}", part.id),
            }
        }
        s.ready = true;
        commands.entity(s.root).insert(Visibility::Inherited);
        info!("hair: {} bound in {:.1} s", part.id, started.elapsed().as_secs_f32());
        lib.status = format!(
            "{}: {} meshes ({} vertices) bound in {:.1} s{}",
            part.id,
            s.meshes.len(),
            s.meshes.iter().map(|m| m.base.len()).sum::<usize>(),
            started.elapsed().as_secs_f32(),
            if unbound > 0 { format!(" ({unbound} joint refs not on our skeleton)") } else { String::new() }
        );
        editor.dirty = true; // shape the new part (positions and IBMs)
    }
}

/// Move the bound parts for a (level-0) vertex delta and our joints' world deltas.
pub fn hair_shape(
    lib: &HairLib,
    editor: &Editor,
    verts: &[[f32; 3]],
    dworld: Option<&[Vec3]>,
    rig: &Rig,
    meshes: &mut Assets<Mesh>,
    ibms: &mut Assets<SkinnedMeshInverseBindposes>,
) {
    let Some(pm) = editor.pack.meshes.iter().find(|m| m.name == "CC_Base_Body") else { return };
    let body = &verts[pm.offset as usize..(pm.offset + pm.count) as usize];
    let started = std::time::Instant::now();
    let mut n = 0;
    let mut max_d = 0.0f32;
    // expressions only (bone-followed meshes take the shape from their bone)
    let shape_body = &editor.delta.verts[pm.offset as usize..(pm.offset + pm.count) as usize];
    let expr_body: Vec<[f32; 3]> = body.iter().zip(shape_body).map(|(a, b)| (Vec3::from(*a) - Vec3::from(*b)).into()).collect();
    for s in lib.spawned.values().filter(|s| s.ready) {
        for m in &s.meshes {
            let Some(mesh) = meshes.get_mut(&m.handle) else { continue };
            let d = match (&m.bone, &lib.bones) {
                (Some(b), Some(bx)) => match bx.mixed(&b.joint, &editor.pack, &editor.weights) {
                    Some(((r0, s0), (r1, s1))) => {
                        let j = &rig.joints[b.rig];
                        let j0 = Vec3::from(j.base_world.translation);
                        let j1 = j0 + j.pack.map_or(Vec3::ZERO, |p| Vec3::from(editor.delta.joints[p]));
                        // the attach point rides the bone's full transform; the mesh turns with it
                        let local = (r0.inverse() * (b.pivot - j0)) / s0;
                        let pivot1 = j1 + r1 * (s1 * local);
                        let rd = r1 * r0.inverse();
                        let expr = m.conform.apply(&expr_body);
                        b.world.iter().zip(&expr).map(|(&p, e)| (rd * (p - b.pivot) + pivot1 - p + Vec3::from(*e)).into()).collect()
                    }
                    None => m.conform.apply(body),
                },
                _ => m.conform.apply(body),
            };
            max_d = d.iter().fold(max_d, |a, d| a.max(Vec3::from(*d).length()));
            use rayon::prelude::*;
            let pos: Vec<[f32; 3]> = m.base.par_iter().zip(&d).map(|(b, d)| (Vec3::from(*b) + m.to_mesh * Vec3::from(*d)).into()).collect();
            match &m.ribbon {
                Some(r) => r.write(mesh, &pos),
                None => mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos),
            }
        }
        if let Some(dworld) = dworld {
            update_skins(rig, &s.skins, dworld, ibms);
        }
        n += s.meshes.iter().map(|m| m.base.len()).sum::<usize>();
    }
    if n > 0 {
        info!("hair: {n} vertices conformed in {:.0} ms (max {:.1} mm)", started.elapsed().as_secs_f32() * 1000.0, max_d * 1000.0);
    }
}

/// The hair section of the panel.
pub fn hair_ui(ui: &mut egui::Ui, lib: &mut HairLib) {
    egui::CollapsingHeader::new("Hair").default_open(true).show(ui, |ui| {
        if !lib.styles.is_empty() {
            ui.horizontal(|ui| {
                ui.label("Style");
                let mut pick = None;
                egui::ComboBox::from_id_salt("hair_style").selected_text("apply…").show_ui(ui, |ui| {
                    for s in &lib.styles {
                        if ui.selectable_label(false, &s.name).clicked() {
                            pick = Some(s.name.clone());
                        }
                    }
                    if ui.selectable_label(false, "(own hair)").clicked() {
                        pick = Some(String::new());
                    }
                });
                match pick.as_deref() {
                    Some("") => {
                        lib.chosen.clear();
                        lib.bald = false;
                    }
                    Some(name) => {
                        if let Err(e) = lib.apply_style(name) {
                            lib.status = format!("{e:#}");
                        }
                    }
                    None => {}
                }
            });
        }
        for slot in SLOTS {
            let options: Vec<usize> = (0..lib.parts.len()).filter(|&i| lib.parts[i].slot == slot).collect();
            if options.is_empty() {
                continue;
            }
            ui.horizontal(|ui| {
                ui.label(format!("{slot:<10}"));
                let current = lib.chosen.get(slot).copied();
                let text = current.map_or("—".to_string(), |i| lib.parts[i].id.clone());
                egui::ComboBox::from_id_salt(("hair_slot", slot)).selected_text(text).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "—").clicked() {
                        lib.chosen.remove(slot);
                    }
                    for i in options {
                        if ui.selectable_label(current == Some(i), &lib.parts[i].id).clicked() {
                            lib.chosen.insert(slot.to_string(), i);
                        }
                    }
                });
            });
        }
        ui.small(&lib.status);
    });
}
