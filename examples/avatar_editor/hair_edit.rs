//! `--hair-src dir --hair-head glb [--hair-config smart_hair.json] [--hair-edit settings.json]`:
//! live hair editing. Pick a CC5 Smart Hair export (`<dir>/<group>/character.glb` or `.fbx`),
//! change its look / shape / colour settings, and the part is rebuilt with
//! `cc5-hair` in the background (about a second) and swapped into its slot.
//! Colour changes only rebuild materials. "Save" bakes the part into the hair
//! library (GLB, catalog, style) and its settings into the config's `groups`.

use std::sync::Arc;
use std::time::Instant;

use bevy::gltf::{GltfMaterialExtras, GltfMaterialName};
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use cc5_hair::head::Body;
use cc5_hair::part::{self, BuiltPart, Geometry};
use cc5_hair::settings::Spec;
use cc5_hair::source::Source;
use cc5_hair::glb_out;
use cc5_shape::Conform;
use cc_render_plugin::cc_render::cc5_materials::{Cc5HairMaterial, Cc5MapBase};
use serde_json::{json, Map, Value};

use super::hair::{bone_follow, conform_for, HairLib, HairPartRoot, PartMesh, Regions};
use super::*;

/// Wait this long after the last change before rebuilding (slider drags).
const SETTLE_S: f32 = 0.25;

#[derive(Resource)]
pub struct HairEdit {
    src: PathBuf,
    head_path: PathBuf,
    config_path: Option<PathBuf>,
    config: Value,
    pub groups: Vec<String>,
    group: Option<String>,
    /// Conversion settings being edited (kind defaults + group overrides + edits).
    conv: Value,
    /// RLHair variables over the source materials' (colour).
    vars: Map<String, Value>,
    head: Option<Arc<(Source, Body)>>,
    sources: HashMap<String, Arc<Source>>,
    task: Option<Task<anyhow::Result<Built>>>,
    changed_at: Option<Instant>,
    colours_changed: bool,
    /// The part on screen: its build and mesh entities (entity, material index).
    last: Option<Arc<BuiltPart>>,
    live: Option<(usize, Vec<(Entity, usize)>)>,
    pub status: String,
}

/// A part built in the background: meshes ready to upload and bound to the body.
struct Built {
    part: Arc<BuiltPart>,
    meshes: Vec<MeshData>,
    secs: f32,
}

struct MeshData {
    name: String,
    material: usize,
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    col: Option<Vec<[f32; 4]>>,
    /// (head skin joint, weight)
    skin: Vec<[(u16, f32); 4]>,
    idx: Vec<u32>,
    conform: Conform,
    follow: Option<String>,
}

fn kind_of(config: &Value, group: &str) -> &'static str {
    let beards: Vec<String> = serde_json::from_value(config["beards"].clone()).unwrap_or_default();
    if beards.iter().any(|b| b == group) {
        "beard"
    } else if group.starts_with("Male_") || group.starts_with("Female_") {
        "brows"
    } else {
        "hair"
    }
}

fn part_id(group: &str) -> String {
    let mut slug = String::new();
    for ch in group.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    format!("sh_{}", slug.trim_matches('_'))
}

impl HairEdit {
    pub fn new(src: PathBuf, head_path: PathBuf, config_path: Option<PathBuf>) -> anyhow::Result<Self> {
        let config = match &config_path {
            Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
            None => json!({"seed": 101, "beards": [], "kinds": {"hair": {"slot": "Base", "kind": "scalp", "convert": {}},
                           "beard": {"slot": "Beard", "kind": "beard", "convert": {}}, "brows": {"slot": "Brows", "kind": "brow", "convert": {"keep_cards": true, "include": "brow"}}}, "groups": {}}),
        };
        let mut groups: Vec<String> = std::fs::read_dir(&src)?
            .filter_map(|e| e.ok())
            .filter(|e| cc5_hair::group_source(&e.path()).is_some())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        groups.sort();
        Ok(Self {
            src,
            head_path,
            config_path,
            config,
            groups,
            group: None,
            conv: Value::Null,
            vars: Map::new(),
            head: None,
            sources: HashMap::new(),
            task: None,
            changed_at: None,
            colours_changed: false,
            last: None,
            live: None,
            status: String::new(),
        })
    }

    /// Building (or about to): `--snapshot` waits.
    pub fn busy(&self) -> bool {
        self.task.is_some() || self.changed_at.is_some()
    }

    /// Edit `group` from its saved settings (`overrides`: on top of them).
    pub fn select(&mut self, group: &str, overrides: Option<&Value>) {
        let kind = kind_of(&self.config, group);
        let mut conv = self.config["kinds"][kind]["convert"].clone();
        let mut vars = Map::new();
        let mut apply = |o: &Value, conv: &mut Value| {
            if let Some(o) = o.as_object() {
                for (k, v) in o {
                    if k == "vars" {
                        vars.extend(v.as_object().cloned().unwrap_or_default());
                    } else {
                        conv[k] = v.clone();
                    }
                }
            }
        };
        apply(&self.config["groups"][group].clone(), &mut conv);
        if let Some(o) = overrides {
            apply(o, &mut conv);
        }
        let dir = self.src.join(group);
        conv["source_glb"] = json!(cc5_hair::group_source(&dir).unwrap_or_else(|| dir.join("character.glb")));
        if dir.join("shape_base.obj").exists() {
            conv["shape_dir"] = json!(dir);
        }
        self.group = Some(group.to_string());
        self.conv = conv;
        self.vars = vars;
        self.changed_at = Some(Instant::now());
    }

    fn spec(&self) -> Option<Spec> {
        let g = self.group.as_ref()?;
        let k = &self.config["kinds"][kind_of(&self.config, g)];
        Some(Spec {
            id: part_id(g),
            slot: k["slot"].as_str().unwrap_or("Base").into(),
            kind: k["kind"].as_str().unwrap_or("scalp").into(),
            atlas: "strands".into(),
            seed: self.config["seed"].as_u64().unwrap_or(101),
            vars: self.vars.clone(),
            convert: self.conv.clone(),
        })
    }

    /// Settings that differ from the kind's defaults (what "Save" stores).
    fn overrides(&self) -> Value {
        let Some(g) = &self.group else { return Value::Null };
        let defaults = &self.config["kinds"][kind_of(&self.config, g)]["convert"];
        let mut out = Map::new();
        if let Some(c) = self.conv.as_object() {
            for (k, v) in c {
                if !["source_glb", "shape_dir"].contains(&k.as_str()) && defaults.get(k) != Some(v) {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        if !self.vars.is_empty() {
            out.insert("vars".into(), Value::Object(self.vars.clone()));
        }
        Value::Object(out)
    }
}

/// Build `spec` in the background: the part, its meshes and their conform binds.
fn build_task(head: Arc<(Source, Body)>, group: Arc<Source>, spec: Spec, lib: PathBuf, body: Arc<Vec<[f32; 3]>>, regions: Regions) -> anyhow::Result<Built> {
    let t = Instant::now();
    let built = part::build_with(&spec, &head.0, &head.1, &lib, Some(&group))?;
    let follow = |name: &str| built.entry.get("follow").and_then(|f| f.get(name)).and_then(|v| v.as_str()).map(str::to_string);
    let mut meshes = Vec::new();
    for pm in &built.meshes {
        let (pos, nrm, uv, col, skin, tris): (Vec<bevy::math::Vec3>, Vec<bevy::math::Vec3>, Vec<bevy::math::Vec2>, Option<Vec<[f32; 4]>>, Vec<[(u16, f32); 4]>, Vec<[u32; 3]>) = match &pm.geom {
            Geometry::Strands { strands, skin, taper, root_fade } => {
                let r = glb_out::ribbons(strands, skin, *taper, *root_fade);
                (r.pos, r.nrm, r.uv, Some(r.col), r.skin, r.tris)
            }
            Geometry::Cards { pos, nrm, uv, tris, skin } => (pos.clone(), nrm.clone(), uv.clone(), None, skin.clone(), tris.clone()),
        };
        let pos: Vec<[f32; 3]> = pos.iter().map(|p| p.to_array()).collect();
        let f = follow(&pm.name);
        let conform = conform_for(&regions, &spec.kind, f.as_deref(), &body, &pos)?;
        meshes.push(MeshData {
            name: pm.name.clone(),
            material: pm.material,
            nrm: nrm.iter().map(|n| n.to_array()).collect(),
            uv: uv.iter().map(|u| u.to_array()).collect(),
            col,
            skin,
            idx: tris.into_iter().flatten().collect(),
            conform,
            follow: f,
            pos,
        });
    }
    Ok(Built { part: Arc::new(built), meshes, secs: t.elapsed().as_secs_f32() })
}

/// `extras.cc5` of material `m` with the edited colour variables over its own.
fn material_extras(part: &BuiltPart, m: usize, vars: &Map<String, Value>) -> Option<String> {
    let mut cc5 = part.materials[m].cc5.clone()?;
    if let Some(v) = cc5.get_mut("vars").and_then(|v| v.as_object_mut()) {
        for (k, x) in vars {
            v.insert(k.clone(), x.clone());
        }
    }
    Some(json!({"cc5": cc5}).to_string())
}

fn standard_material(part: &BuiltPart, m: usize, images: &mut Assets<Image>) -> StandardMaterial {
    let pm = &part.materials[m];
    let f = pm.base_factor;
    let texture = pm.base_image.as_ref().and_then(|(bytes, mime)| {
        Image::from_buffer(bytes, ImageType::MimeType(mime), CompressedImageFormats::NONE, true, ImageSampler::Default, RenderAssetUsages::default())
            .ok()
            .map(|i| images.add(i))
    });
    StandardMaterial {
        base_color: Color::linear_rgba(f[0], f[1], f[2], f[3]),
        base_color_texture: texture,
        alpha_mode: if pm.blend { AlphaMode::Blend } else { AlphaMode::Mask(0.35) },
        double_sided: true,
        cull_mode: None,
        metallic: 0.0,
        perceptual_roughness: 0.5,
        ..default()
    }
}

/// Start rebuilds once changes settle; install finished parts; swap materials
/// on colour changes.
#[allow(clippy::too_many_arguments)]
pub fn hair_edit_run(
    mut commands: Commands,
    mut edit: ResMut<HairEdit>,
    mut lib: ResMut<HairLib>,
    rig: Res<Rig>,
    mut editor: ResMut<Editor>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut ibms: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    let ed = &mut *edit;
    // colour only: new materials on the live meshes (the CC5 material is rebuilt from the extras)
    if std::mem::take(&mut ed.colours_changed) {
        if let (Some(part), Some((_, ents))) = (&ed.last, &ed.live) {
            for &(e, m) in ents {
                let mut ec = commands.entity(e);
                ec.insert(MeshMaterial3d(materials.add(standard_material(part, m, &mut images))));
                if let Some(x) = material_extras(part, m, &ed.vars) {
                    ec.insert(GltfMaterialExtras { value: x });
                }
                ec.remove::<(cc_render_plugin::cc_render::cc5_materials::Cc5Material, MeshMaterial3d<Cc5HairMaterial>)>();
            }
        }
    }
    // a finished build
    if let Some(task) = ed.task.as_mut() {
        if let Some(res) = block_on(future::poll_once(task)) {
            ed.task = None;
            match res {
                Ok(built) => {
                    if let Err(e) = install(ed, built, &mut commands, &mut lib, &rig, &mut meshes, &mut materials, &mut images, &mut ibms) {
                        ed.status = format!("{e:#}");
                    }
                    editor.dirty = true; // shape the new part
                }
                Err(e) => ed.status = format!("build failed: {e:#}"),
            }
        }
    }
    // start a rebuild once the settings stop changing
    if ed.task.is_none() && ed.changed_at.is_some_and(|t| t.elapsed().as_secs_f32() >= SETTLE_S) {
        ed.changed_at = None;
        let (Some(spec), Some(body)) = (ed.spec(), lib.body.clone()) else {
            ed.status = "hair edit: waiting for the character's body".into();
            ed.changed_at = Some(Instant::now());
            return;
        };
        if ed.head.is_none() {
            match Source::load(&ed.head_path).and_then(|s| Body::from_source(&s).map(|b| (s, b))) {
                Ok(h) => ed.head = Some(Arc::new(h)),
                Err(e) => {
                    ed.status = format!("--hair-head: {e:#}");
                    return;
                }
            }
        }
        let g = ed.group.clone().unwrap();
        let group = match ed.sources.get(&g) {
            Some(s) => s.clone(),
            None => match Source::load(&cc5_hair::group_source(&ed.src.join(&g)).unwrap_or_default()) {
                Ok(s) => {
                    let s = Arc::new(s);
                    ed.sources.insert(g.clone(), s.clone());
                    s
                }
                Err(e) => {
                    ed.status = format!("{g}: {e:#}");
                    return;
                }
            },
        };
        let (head, regions, dir) = (ed.head.clone().unwrap(), lib.regions.clone(), lib.dir.clone());
        let body = Arc::new(body);
        ed.task = Some(AsyncComputeTaskPool::get().spawn(async move { build_task(head, group, spec, dir, body, regions) }));
        ed.status = format!("building {g} …");
    }
}

/// Spawn a built part's meshes on our rig and put it in its slot.
#[allow(clippy::too_many_arguments)]
fn install(
    ed: &mut HairEdit,
    built: Built,
    commands: &mut Commands,
    lib: &mut HairLib,
    rig: &Rig,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    ibms: &mut Assets<SkinnedMeshInverseBindposes>,
) -> anyhow::Result<()> {
    let t = Instant::now();
    let part = built.part.clone();
    let i = lib.part_index(&part.entry)?;
    let slot = lib.parts[i].slot.clone();
    lib.chosen.insert(slot, i);
    // beards / brows sit on the character's converted own hair, as the CC5 styles do
    if lib.parts[i].kind != "scalp" && !lib.chosen.contains_key("Base") {
        if let Some(own) = lib.parts.iter().position(|p| p.id == "cc5_converted") {
            lib.chosen.insert("Base".into(), own);
        }
    }
    // skin on all our joints: the part's joints map by name
    let head = ed.head.as_ref().unwrap();
    let names = head.0.skin_joints();
    let to_rig: Vec<u16> = names.iter().map(|n| rig.joints.iter().position(|j| &j.name == n).unwrap_or(0) as u16).collect();
    let ibm: Vec<Mat4> = rig.joints.iter().map(|j| Mat4::from(j.base_world).inverse()).collect();
    let ibm_handle = ibms.add(SkinnedMeshInverseBindposes::from(ibm.clone()));
    let joints: Vec<Entity> = rig.joints.iter().map(|j| j.entity).collect();
    let root = commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            HairPartRoot(i),
            Cc5MapBase(bevy::asset::AssetPath::from_path(&lib.dir.join(format!("{}.glb", part.id))).into_owned()),
            Name::new(part.id.clone()),
        ))
        .id();
    let mut part_meshes = Vec::new();
    let mut ents = Vec::new();
    for md in built.meshes {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, md.pos.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, md.nrm);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, md.uv);
        if let Some(c) = md.col {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(md.skin.iter().map(|s| s.map(|e| to_rig[e.0 as usize])).collect()));
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, md.skin.iter().map(|s| s.map(|e| e.1)).collect::<Vec<[f32; 4]>>());
        mesh.insert_indices(Indices::U32(md.idx));
        let handle = meshes.add(mesh);
        let mut ec = commands.spawn((
            Mesh3d(handle.clone()),
            MeshMaterial3d(materials.add(standard_material(&part, md.material, images))),
            SkinnedMesh { inverse_bindposes: ibm_handle.clone(), joints: joints.clone() },
            GltfMaterialName(part.materials[md.material].name.clone()),
            Name::new(md.name.clone()),
        ));
        if let Some(x) = material_extras(&part, md.material, &ed.vars) {
            ec.insert(GltfMaterialExtras { value: x });
        }
        ec.set_parent(root);
        ents.push((ec.id(), md.material));
        let bone = bone_follow(md.follow.as_deref(), rig, lib.bones.as_ref(), &md.pos);
        part_meshes.push(PartMesh { handle, base: md.pos, to_mesh: Mat3::IDENTITY, conform: md.conform, bone });
    }
    let skins = vec![SkinRec { handle: ibm_handle, joints: (0..rig.joints.len()).collect(), base: ibm }];
    if let Some(old) = lib.put_built(i, root, part_meshes, skins) {
        commands.entity(old).despawn_recursive();
    }
    ed.status = format!(
        "{}: {} strands/cards, {} vertices: built {:.1} s, shown {:.2} s",
        part.id,
        part.entry["cards"],
        part.entry["verts"],
        built.secs,
        t.elapsed().as_secs_f32()
    );
    ed.last = Some(part);
    ed.live = Some((i, ents));
    Ok(())
}

/// Save the part on screen into the library and its settings into the config.
fn save(ed: &mut HairEdit, lib: &mut HairLib) -> anyhow::Result<String> {
    let (Some(part), Some(g), Some(head)) = (ed.last.clone(), ed.group.clone(), ed.head.clone()) else {
        anyhow::bail!("nothing built yet");
    };
    // the colour edits are part of the saved materials
    let mut part_owned = BuiltPart { id: part.id.clone(), meshes: Vec::new(), materials: Vec::new(), entry: part.entry.clone(), files: part.files.clone() };
    for (m, pm) in part.materials.iter().enumerate() {
        let mut cc5 = pm.cc5.clone();
        if let (Some(c), Some(x)) = (cc5.as_mut(), material_extras(&part, m, &ed.vars)) {
            *c = serde_json::from_str::<Value>(&x)?["cc5"].clone();
        }
        part_owned.materials.push(part::PartMaterial { name: pm.name.clone(), blend: pm.blend, cc5, base_factor: pm.base_factor, base_image: pm.base_image.clone() });
    }
    // geometry: the same build (meshes are borrowed from the live part)
    let path = lib.dir.join(format!("{}.glb", part.id));
    glb_out::write_with(&part, &part_owned.materials, &head.0, &path)?;
    glb_out::copy_files(&part, &lib.dir)?;
    // catalog + style
    let read = |f: &str| -> anyhow::Result<Value> { Ok(serde_json::from_str(&std::fs::read_to_string(lib.dir.join(f))?)?) };
    let mut cat = read("catalog.json")?;
    let parts = cat["parts"].as_array_mut().unwrap();
    parts.retain(|p| p["id"] != part.entry["id"]);
    parts.push(part.entry.clone());
    std::fs::write(lib.dir.join("catalog.json"), serde_json::to_string_pretty(&cat)?)?;
    let mut sty = read("styles.json")?;
    let name = format!("CC5 {g}");
    let style_parts = if part.entry["kind"] == "scalp" { vec![part.id.clone()] } else { vec!["cc5_converted".to_string(), part.id.clone()] };
    let styles = sty["styles"].as_array_mut().unwrap();
    styles.retain(|s| s["name"] != json!(name));
    styles.push(json!({"name": name, "parts": style_parts}));
    std::fs::write(lib.dir.join("styles.json"), serde_json::to_string_pretty(&sty)?)?;
    // settings
    if let Some(cp) = &ed.config_path {
        ed.config["groups"][&g] = ed.overrides();
        std::fs::write(cp, serde_json::to_string_pretty(&ed.config)?)?;
    }
    lib.part_index(&part.entry)?;
    Ok(format!("saved {} ({:.0} MB)", path.display(), std::fs::metadata(&path)?.len() as f64 / 1048576.0))
}

fn num(ui: &mut egui::Ui, v: &mut Value, key: &str, range: std::ops::RangeInclusive<f64>, label: &str, log: bool) -> bool {
    let mut x = v.get(key).and_then(Value::as_f64).unwrap_or(*range.start());
    let changed = ui.add(egui::Slider::new(&mut x, range).logarithmic(log).text(label)).changed();
    if changed {
        v[key] = json!(x);
    }
    changed
}

fn pair(ui: &mut egui::Ui, v: &mut Value, key: &str, range: std::ops::RangeInclusive<f64>, label: &str, default: [f64; 2]) -> bool {
    let mut a: [f64; 2] = serde_json::from_value(v.get(key).cloned().unwrap_or(Value::Null)).unwrap_or(default);
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.add(egui::DragValue::new(&mut a[0]).range(range.clone()).speed((range.end() - range.start()) / 300.0)).changed();
        changed |= ui.add(egui::DragValue::new(&mut a[1]).range(range.clone()).speed((range.end() - range.start()) / 300.0)).changed();
        ui.label(label);
    });
    if changed {
        v[key] = json!(a);
    }
    changed
}

fn colour(ui: &mut egui::Ui, vars: &mut Map<String, Value>, base: &Map<String, Value>, key: &str, label: &str) -> bool {
    let cur = vars.get(key).or_else(|| base.get(key)).and_then(|v| v.as_array()).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect::<Vec<_>>()).unwrap_or(vec![128.0; 3]);
    let mut c = [cur[0] as u8, cur[1] as u8, cur[2] as u8];
    let changed = ui.horizontal(|ui| {
        let r = ui.color_edit_button_srgb(&mut c).changed();
        ui.label(label);
        r
    }).inner;
    if changed {
        vars.insert(key.into(), json!([c[0] as f64, c[1] as f64, c[2] as f64]));
    }
    changed
}

fn strength(ui: &mut egui::Ui, vars: &mut Map<String, Value>, base: &Map<String, Value>, key: &str, label: &str, max: f64) -> bool {
    let mut x = vars.get(key).or_else(|| base.get(key)).and_then(Value::as_f64).unwrap_or(1.0);
    let changed = ui.add(egui::Slider::new(&mut x, 0.0..=max).text(label)).changed();
    if changed {
        vars.insert(key.into(), json!(x));
    }
    changed
}

/// The Edit section of the hair panel.
pub fn hair_edit_ui(ui: &mut egui::Ui, ed: &mut HairEdit, lib: &mut HairLib) {
    egui::CollapsingHeader::new("Hair edit (live)").default_open(true).show(ui, |ui| {
        let mut pick = None;
        egui::ComboBox::from_id_salt("hair_edit_group").selected_text(ed.group.clone().unwrap_or_else(|| "pick a CC5 hair group…".into())).show_ui(ui, |ui| {
            for g in &ed.groups {
                if ui.selectable_label(ed.group.as_ref() == Some(g), g).clicked() {
                    pick = Some(g.clone());
                }
            }
        });
        if let Some(g) = pick {
            ed.select(&g, None);
        }
        if ed.group.is_none() {
            ui.small(&ed.status);
            return;
        }
        let mut changed = false;
        let c = &mut ed.conv;
        ui.label("Look");
        changed |= num(ui, c, "strands_per_cm2", 1.0..=150.0, "density (strands / cm²)", true);
        changed |= pair(ui, c, "width", 0.00002..=0.0005, "strand width (m)", [0.00008, 0.00013]);
        changed |= num(ui, c, "points", 3.0..=24.0, "points per strand", false);
        changed |= num(ui, c, "clumps_per_cm", 0.0..=20.0, "clumps per cm", false);
        changed |= num(ui, c, "clump_spread", 0.02..=0.5, "clump spread", false);
        changed |= pair(ui, c, "clump_shade", 0.2..=1.6, "clump shade", [0.75, 1.15]);
        changed |= pair(ui, c, "shade", 0.2..=1.6, "strand shade", [0.82, 1.08]);
        changed |= pair(ui, c, "depth_shade", 0.0..=1.0, "depth shade (darkest, curve)", [1.0, 0.8]);
        changed |= num(ui, c, "thickness", 0.0..=0.01, "layer thickness (m)", false);
        changed |= num(ui, c, "root_fade", 0.0..=0.5, "root fade", false);
        changed |= pair(ui, c, "accept", 0.0..=1.0, "texture acceptance", [0.05, 0.6]);
        ui.label("Shape");
        if !c.get("shape").is_some_and(Value::is_object) {
            c["shape"] = json!({"length": 1.0});
        }
        let sh = &mut c["shape"];
        changed |= num(ui, sh, "length", 0.2..=2.0, "length", false);
        changed |= num(ui, sh, "gravity", -0.05..=0.05, "gravity", false);
        changed |= num(ui, sh, "curl_radius", 0.0..=0.01, "curl radius (m)", false);
        changed |= num(ui, sh, "curl_period", 0.005..=0.1, "curl period (m)", false);
        changed |= num(ui, sh, "frizz", 0.0..=0.005, "frizz (m)", false);
        changed |= num(ui, sh, "comb_strength", 0.0..=1.0, "comb strength", false);
        ui.horizontal(|ui| {
            let mut d: [f64; 3] = serde_json::from_value(sh.get("comb").cloned().unwrap_or(Value::Null)).unwrap_or([0.0, -1.0, 0.0]);
            let mut ch = false;
            for x in d.iter_mut() {
                ch |= ui.add(egui::DragValue::new(x).range(-1.0..=1.0).speed(0.02)).changed();
            }
            ui.label("comb direction");
            if ch {
                sh["comb"] = json!(d);
                changed = true;
            }
        });
        // colour (material variables only, no rebuild)
        ui.label("Colour");
        let base: Map<String, Value> = ed
            .last
            .as_ref()
            .and_then(|p| p.materials.iter().find_map(|m| m.cc5.as_ref()?.get("vars")?.as_object().cloned()))
            .unwrap_or_default();
        let mut col = false;
        col |= colour(ui, &mut ed.vars, &base, "RootColor", "root");
        col |= colour(ui, &mut ed.vars, &base, "TipColor", "tip");
        col |= strength(ui, &mut ed.vars, &base, "RootColorStrength", "root strength", 1.0);
        col |= strength(ui, &mut ed.vars, &base, "TipColorStrength", "tip strength", 1.0);
        col |= colour(ui, &mut ed.vars, &base, "_1st Dye Color", "1st dye");
        col |= strength(ui, &mut ed.vars, &base, "_1st Dye Strength", "1st dye strength", 1.0);
        col |= colour(ui, &mut ed.vars, &base, "_2nd Dye Color", "2nd dye");
        col |= strength(ui, &mut ed.vars, &base, "_2nd Dye Strength", "2nd dye strength", 1.0);
        col |= strength(ui, &mut ed.vars, &base, "BaseColorMapStrength", "texture colour", 1.0);
        col |= strength(ui, &mut ed.vars, &base, "VertexColorStrength", "strand colour", 1.0);
        col |= strength(ui, &mut ed.vars, &base, "Specular Strength", "specular", 1.0);
        if changed {
            ed.changed_at = Some(Instant::now());
        }
        if col {
            ed.colours_changed = true;
        }
        ui.horizontal(|ui| {
            if ui.button("Reset to saved").clicked() {
                let g = ed.group.clone().unwrap();
                ed.select(&g, None);
                ed.colours_changed = true;
            }
            if ui.add_enabled(ed.last.is_some() && !ed.busy(), egui::Button::new("Save to library")).clicked() {
                ed.status = match save(ed, lib) {
                    Ok(s) => s,
                    Err(e) => format!("save failed: {e:#}"),
                };
            }
        });
        ui.small(&ed.status);
    });
}
