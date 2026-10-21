//! Skin section of the avatar editor: CC5's skin material settings, edited live.
//!
//! The settings are CC5's own (RLHead / RLSkin, names, ranges and groups as in
//! CC5's material panel and `Program/Assets/Share/Shaders/Human/RLHead.shader` /
//! `RLSkin.shader`), stored per skin material in `extras.cc5.vars` / `.sss`;
//! the editor changes them on the CC5 skin materials ([`Cc5Skin::set_vars`]).
//!
//! With a skin pack (`--skin-pack dir`, default `<character dir>/skin`, built by
//! `cc5-skin build` from CC5 captures), also CC5's SkinGen: whole skins (their
//! textures and settings) and layers (makeup, freckles, stubble, scars, ...)
//! composited over the skin's diffuse in the background ([`cc5_skin::compose`]),
//! each at a strength and optionally recoloured.
//!
//! A skin recipe (`--skin recipe.json`, the panel's Save / Load, snapshot jobs'
//! `skin`) keeps what differs from the character:
//! `{"format": "cc5-skin", "version": 1, "skin": "full_default_male" | null,
//!   "layers": [{"id": "lip_rouge", "strength": 0.8, "tint": [200, 30, 60]}],
//!   "materials": {"Std_Skin_Head": {"vars": {...}, "sss": {...}}}}`.
use bevy::gltf::GltfMaterialName;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use bevy_egui::egui;
use cc5_skin::{compose, Apply, LayerImage, SkinPack};
use cc_render_plugin::cc_render::cc5_materials::{Cc5Handle, Cc5Material, Cc5SkinMaterial};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// How a setting is edited.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// slider over [min, max] (typing beyond: CC5's spin range)
    Float(f32, f32),
    Toggle,
    /// 0..255 sRGB triple
    Color,
}

/// Where the setting lives: `vars` or the SSS settings.
#[derive(Clone, Copy, PartialEq)]
enum Store {
    Var,
    Sss,
}

struct Setting {
    key: &'static str,
    group: &'static str,
    kind: Kind,
    store: Store,
}

const fn f(key: &'static str, group: &'static str, min: f32, max: f32) -> Setting {
    Setting { key, group, kind: Kind::Float(min, max), store: Store::Var }
}
const fn t(key: &'static str, group: &'static str) -> Setting {
    Setting { key, group, kind: Kind::Toggle, store: Store::Var }
}
const fn c(key: &'static str, group: &'static str) -> Setting {
    Setting { key, group, kind: Kind::Color, store: Store::Var }
}

/// CC5's skin settings (RLHead and RLSkin; a material shows those it has).
const SETTINGS: &[Setting] = &[
    f("_BaseColorMap Brightness", "Color", -1.0, 1.0),
    f("_BaseColorMap Saturation", "Color", -1.0, 1.0),
    t("_Active Human Color", "Human color"),
    t("_Active", "Human color"),
    c("_Skin Color Range", "Human color"),
    f("_Range", "Human color", 0.0, 1.0),
    f("_Red", "Human color", 0.0, 100.0),
    f("_Yellow", "Human color", 0.0, 100.0),
    f("_Black", "Human color", 0.0, 100.0),
    t("_Active Blend Map", "Human color"),
    c("_Blend Map Color", "Human color"),
    f("_Blend Map Strength", "Human color", 0.0, 1.0),
    f("BaseColor Blend2 Strength", "Color", 0.0, 1.0),
    f("NormalMap Blend Strength", "Micro normal", 0.0, 1.0),
    f("MicroNormal Strength", "Micro normal", 0.0, 3.0),
    f("MicroNormal Tiling", "Micro normal", 0.0, 200.0),
    t("Flip MicroNormal Y", "Micro normal"),
    f("_Specular", "Specular & roughness", 0.0, 5.0),
    t("Keep Original Roughness", "Specular & roughness"),
    f("Original Roughness Strength", "Specular & roughness", 0.0, 1.0),
    f("Micro Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Edge Roughness Multiplier", "Specular & roughness", -1.5, 1.5),
    f("Unmasked Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Cheek Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Forehead Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("UpperLip Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Chin Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Nose Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Mouth Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("UpperLid Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("InnerLid Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Ear Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Neck Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("R Channel Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("G Channel Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("B Channel Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("A Channel Roughness Scale", "Specular & roughness", -1.5, 1.5),
    f("Inner Mouth Ao", "Specular & roughness", 0.0, 5.0),
    f("Nostril Ao", "Specular & roughness", 0.0, 5.0),
    f("Lips Gap Ao", "Specular & roughness", 0.0, 5.0),
    Setting { key: "Falloff", group: "Subsurface", kind: Kind::Color, store: Store::Sss },
    Setting { key: "Radius", group: "Subsurface", kind: Kind::Float(0.1, 3.0), store: Store::Sss },
    f("Unmasked Scatter Scale", "Subsurface", 0.0, 2.0),
    f("Cheek Scatter Scale", "Subsurface", 0.0, 2.0),
    f("Forehead Scatter Scale", "Subsurface", 0.0, 2.0),
    f("UpperLip Scatter Scale", "Subsurface", 0.0, 2.0),
    f("Chin Scatter Scale", "Subsurface", 0.0, 2.0),
    f("Nose Scatter Scale", "Subsurface", 0.0, 2.0),
    f("Mouth Scatter Scale", "Subsurface", 0.0, 2.0),
    f("UpperLid Scatter Scale", "Subsurface", 0.0, 2.0),
    f("InnerLid Scatter Scale", "Subsurface", 0.0, 2.0),
    f("Ear Scatter Scale", "Subsurface", 0.0, 2.0),
    f("Neck Scatter Scale", "Subsurface", 0.0, 2.0),
    f("R Channel Scatter Scale", "Subsurface", 0.0, 2.0),
    f("G Channel Scatter Scale", "Subsurface", 0.0, 2.0),
    f("B Channel Scatter Scale", "Subsurface", 0.0, 2.0),
    f("A Channel Scatter Scale", "Subsurface", 0.0, 2.0),
];

const GROUPS: [&str; 5] = ["Color", "Human color", "Specular & roughness", "Subsurface", "Micro normal"];

/// One CC5 skin material (shared by every primitive that uses it).
pub struct SkinMat {
    pub name: String,
    pub handle: Handle<Cc5SkinMaterial>,
    /// as the character has it
    orig_vars: Value,
    orig_sss: Value,
    pub vars: Value,
    pub sss: Value,
}

impl SkinMat {
    fn store(&self, s: Store) -> &Value {
        match s {
            Store::Var => &self.vars,
            Store::Sss => &self.sss,
        }
    }
    fn store_mut(&mut self, s: Store) -> &mut Value {
        match s {
            Store::Var => &mut self.vars,
            Store::Sss => &mut self.sss,
        }
    }
}

/// A SkinGen layer laid over the skin (in the order they were switched on).
#[derive(Clone, Debug)]
pub struct LayerState {
    pub id: String,
    /// CC5's layer strength: opacity scale
    pub strength: f32,
    /// recolour (sRGB)
    pub tint: Option<[u8; 3]>,
}

/// The character's own textures of a skin material (restored when a skin preset
/// or the last layer is removed).
#[derive(Clone)]
struct Originals {
    base_color: Color,
    diffuse: Option<Handle<Image>>,
    normal: Option<Handle<Image>>,
    mr: Option<Handle<Image>>,
    occlusion: Option<Handle<Image>>,
    sss: Option<Handle<Image>>,
    transmission: Option<Handle<Image>>,
    micro_mask: Option<Handle<Image>>,
    specular: Option<Handle<Image>>,
    present: u32,
    srgb: u32,
}

type Composed = Vec<(usize, u32, u32, Vec<u8>)>;

#[derive(Resource, Default)]
pub struct SkinEdit {
    pub mats: Vec<SkinMat>,
    /// index into `mats` edited by the panel; None = all of them
    selected: Option<usize>,
    /// recipe to apply once the materials exist
    pending: Option<Value>,
    dirty: bool,
    pub path: String,
    pub status: String,
    /// `--skin-pack` (default `<character dir>/skin`): SkinGen layers and skins captured from CC5
    pub pack: Option<Arc<SkinPack>>,
    pub layers: Vec<LayerState>,
    /// skin preset id (None: the character's own skin)
    pub skin: Option<String>,
    /// recipe `"albedo"`: a flat diffuse colour instead of the maps (sRGB; for
    /// calibrating lights against CC5 renders of a flat grey skin)
    pub albedo: Option<[u8; 3]>,
    /// layers / skin changed: recomposite and swap textures
    layers_dirty: bool,
    skin_dirty: bool,
    originals: Vec<Option<Originals>>,
    /// per material: the character's diffuse texels (RGBA8 sRGB) and size
    bases: Vec<Option<Arc<(u32, u32, Vec<u8>)>>>,
    /// per material: the composited diffuse
    composed: Vec<Option<Handle<Image>>>,
    cache: Arc<Mutex<HashMap<(String, String), Option<Arc<LayerImage>>>>>,
    task: Option<Task<anyhow::Result<Composed>>>,
    /// what the running task composites (to notice newer edits)
    task_key: String,
}

impl SkinEdit {
    pub fn new(recipe: Option<PathBuf>, pack: Option<PathBuf>) -> anyhow::Result<Self> {
        let pending = recipe.as_ref().map(|p| -> anyhow::Result<Value> { Ok(serde_json::from_slice(&std::fs::read(p)?)?) }).transpose()?;
        let pack = match pack {
            Some(p) if p.join("skin.json").is_file() => {
                let pk = SkinPack::load(&p)?;
                info!("skin pack {}: {} layers, {} skins", p.display(), pk.layers.len(), pk.skins.len());
                Some(Arc::new(pk))
            }
            _ => None,
        };
        Ok(Self {
            pending,
            pack,
            path: recipe.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "skin.json".into()),
            ..default()
        })
    }

    /// No recipe waiting for the materials and no edit waiting to be shown.
    pub fn ready(&self) -> bool {
        self.pending.is_none() && !self.dirty && !self.layers_dirty && !self.skin_dirty && self.task.is_none()
    }

    /// Settings that differ from the character's (per material), the skin preset and layers.
    pub fn recipe(&self) -> Value {
        let mut mats = Map::new();
        for m in &self.mats {
            let diff = |cur: &Value, orig: &Value| -> Map<String, Value> {
                cur.as_object()
                    .map(|o| o.iter().filter(|(k, v)| orig.get(k.as_str()) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default()
            };
            let (v, s) = (diff(&m.vars, &m.orig_vars), diff(&m.sss, &m.orig_sss));
            if !v.is_empty() || !s.is_empty() {
                mats.insert(m.name.clone(), json!({"vars": v, "sss": s}));
            }
        }
        let layers: Vec<Value> = self
            .layers
            .iter()
            .map(|l| {
                let mut o = json!({"id": l.id, "strength": l.strength});
                if let Some(t) = l.tint {
                    o["tint"] = json!(t);
                }
                o
            })
            .collect();
        let mut r = json!({"format": "cc5-skin", "version": 1, "skin": self.skin, "layers": layers, "materials": mats});
        if let Some(a) = self.albedo {
            r["albedo"] = json!(a);
        }
        r
    }

    /// Back to the character's skin, then `recipe`'s on top.
    pub fn apply(&mut self, recipe: &Value) -> anyhow::Result<()> {
        if self.mats.is_empty() {
            self.pending = Some(recipe.clone());
            return Ok(());
        }
        anyhow::ensure!(recipe.get("format").and_then(Value::as_str) == Some("cc5-skin"), "not a cc5-skin recipe");
        self.reset();
        if let Some(id) = recipe.get("skin").and_then(Value::as_str) {
            self.set_skin(Some(id.to_string()));
        }
        self.albedo = recipe.get("albedo").and_then(color);
        for l in recipe.get("layers").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = l.get("id").and_then(Value::as_str) else { continue };
            if self.pack.as_ref().and_then(|p| p.layer(id)).is_none() {
                warn!("skin recipe: no layer {id} in the skin pack");
                continue;
            }
            let tint = l.get("tint").and_then(color);
            self.layers.push(LayerState { id: id.into(), strength: l.get("strength").and_then(number).unwrap_or(1.0), tint });
        }
        if let Some(ms) = recipe.get("materials").and_then(Value::as_object) {
            for (name, r) in ms {
                let Some(m) = self.mats.iter_mut().find(|m| &m.name == name) else {
                    warn!("skin recipe: no material {name}");
                    continue;
                };
                for (part, store) in [("vars", Store::Var), ("sss", Store::Sss)] {
                    if let Some(o) = r.get(part).and_then(Value::as_object) {
                        for (k, v) in o {
                            m.store_mut(store)[k] = v.clone();
                        }
                    }
                }
            }
        }
        self.dirty = true;
        Ok(())
    }

    pub fn reset(&mut self) {
        for m in &mut self.mats {
            m.vars = m.orig_vars.clone();
            m.sss = m.orig_sss.clone();
        }
        self.layers.clear();
        self.albedo = None;
        if self.skin.is_some() {
            self.skin = None;
            self.skin_dirty = true;
        }
        self.layers_dirty = true;
        self.dirty = true;
    }

    /// A skin preset (or the character's own): its textures, and its shader
    /// settings over the character's.
    pub fn set_skin(&mut self, id: Option<String>) {
        let skin = id.as_ref().and_then(|id| self.pack.as_ref()?.skin(id).cloned());
        for m in &mut self.mats {
            m.vars = m.orig_vars.clone();
            if let Some(v) = skin.as_ref().and_then(|s| s.vars.get(&m.name)).and_then(Value::as_object) {
                for (k, x) in v {
                    if !k.to_lowercase().starts_with("padding") {
                        m.vars[k] = x.clone();
                    }
                }
            }
        }
        self.skin = id;
        self.skin_dirty = true;
        self.layers_dirty = true;
        self.dirty = true;
    }

    fn targets(&self) -> Vec<usize> {
        match self.selected {
            Some(i) => vec![i],
            None => (0..self.mats.len()).collect(),
        }
    }

    fn key(&self) -> String {
        format!("{:?}|{:?}", self.skin, self.layers)
    }
}

/// Finds the character's CC5 skin materials and pushes edited settings into them.
pub fn skin_sync(
    mut skin: ResMut<SkinEdit>,
    prims: Query<(&Cc5Material, Option<&GltfMaterialName>)>,
    mut assets: ResMut<Assets<Cc5SkinMaterial>>,
) {
    if skin.mats.is_empty() {
        let mut found: Vec<SkinMat> = Vec::new();
        for (m, name) in &prims {
            let Cc5Handle::Skin(h) = &m.cc5 else { continue };
            if found.iter().any(|f| f.handle == *h) {
                continue;
            }
            let Some(mat) = assets.get(h) else { continue };
            let src = &mat.extension.source;
            found.push(SkinMat {
                name: name.map(|n| n.0.clone()).unwrap_or_else(|| format!("skin {}", found.len())),
                handle: h.clone(),
                orig_vars: src.vars.clone(),
                orig_sss: src.sss.clone(),
                vars: src.vars.clone(),
                sss: src.sss.clone(),
            });
        }
        if found.is_empty() {
            return;
        }
        // head first, then by name
        found.sort_by_key(|m| (!m.name.contains("Head"), m.name.clone()));
        info!("skin: {} CC5 skin materials ({})", found.len(), found.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "));
        let n = found.len();
        skin.mats = found;
        skin.originals = vec![None; n];
        skin.bases = vec![None; n];
        skin.composed = vec![None; n];
        if let Some(r) = skin.pending.take() {
            match skin.apply(&r) {
                Ok(()) => skin.status = "skin recipe applied".into(),
                Err(e) => skin.status = format!("skin recipe: {e:#}"),
            }
        }
    }
    if !std::mem::take(&mut skin.dirty) {
        return;
    }
    for m in &skin.mats {
        if let Some(mat) = assets.get_mut(&m.handle) {
            mat.extension.set_vars(m.vars.clone(), m.sss.clone());
        }
    }
}

/// Texture swaps for the skin preset and the composited SkinGen layers.
pub fn skin_layers(
    mut skin: ResMut<SkinEdit>,
    mut mats: ResMut<Assets<Cc5SkinMaterial>>,
    mut images: ResMut<Assets<Image>>,
    server: Res<AssetServer>,
) {
    let skin = &mut *skin;
    if skin.mats.is_empty() {
        return;
    }
    // without a pack: only the flat albedo (no skins, no layers)
    let pack = skin.pack.clone().unwrap_or_else(|| Arc::new(SkinPack::default()));
    // the character's own textures, once
    for (i, m) in skin.mats.iter().enumerate() {
        if skin.originals[i].is_some() {
            continue;
        }
        let Some(mat) = mats.get(&m.handle) else { continue };
        let (b, e) = (&mat.base, &mat.extension);
        skin.originals[i] = Some(Originals {
            base_color: b.base_color,
            diffuse: b.base_color_texture.clone(),
            normal: b.normal_map_texture.clone(),
            mr: b.metallic_roughness_texture.clone(),
            occlusion: b.occlusion_texture.clone(),
            sss: e.sss.clone(),
            transmission: e.transmission.clone(),
            micro_mask: e.micro_mask.clone(),
            specular: e.specular.clone(),
            present: e.params.present,
            srgb: e.params.srgb,
        });
    }
    if std::mem::take(&mut skin.skin_dirty) {
        let preset = skin.skin.as_ref().and_then(|id| pack.skin(id));
        for (i, m) in skin.mats.iter().enumerate() {
            let (Some(o), Some(mat)) = (skin.originals[i].clone(), mats.get_mut(&m.handle)) else { continue };
            let maps = preset.and_then(|s| s.maps.get(&m.name));
            let load = |ch: &str| -> Option<Handle<Image>> {
                let rel = maps?.get(ch)?;
                let path = pack.dir.join(rel);
                Some(server.load_with_settings(path, |s: &mut ImageLoaderSettings| {
                    s.is_srgb = false;
                    s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        ..ImageSamplerDescriptor::linear()
                    });
                }))
            };
            mat.base.normal_map_texture = load("Normal").or(o.normal.clone());
            mat.base.metallic_roughness_texture = load("MetallicRoughness").or(o.mr.clone());
            mat.base.occlusion_texture = load("ao").or(o.occlusion.clone());
            // CC5 shader maps (skin.wgsl bits: 1 micro mask, 2 SSS, 3 transmission, 8 specular)
            let e = &mut mat.extension;
            let (mut present, mut srgb) = (o.present, o.srgb);
            for (bit, ch, slot, orig) in [
                (1u32, "MicroNMask", &mut e.micro_mask, &o.micro_mask),
                (2, "SSSMap", &mut e.sss, &o.sss),
                (3, "TransMap", &mut e.transmission, &o.transmission),
                (8, "SpecMask", &mut e.specular, &o.specular),
            ] {
                match load(ch) {
                    Some(h) => {
                        *slot = Some(h);
                        present |= 1 << bit;
                        srgb &= !(1 << bit);
                    }
                    None => *slot = orig.clone(),
                }
            }
            e.params.present = present;
            e.params.srgb = srgb;
        }
        skin.layers_dirty = true;
    }
    // finished composite: into the materials' diffuse
    if let Some(task) = skin.task.as_mut() {
        if let Some(result) = block_on(future::poll_once(task)) {
            skin.task = None;
            match result {
                Ok(out) => {
                    for (i, w, h, data) in out {
                        let handle = match skin.composed[i].clone() {
                            Some(hd) if images.get(&hd).is_some_and(|im| im.width() == w && im.height() == h) => {
                                images.get_mut(&hd).unwrap().data = data;
                                hd
                            }
                            _ => {
                                let mut img = Image::new(
                                    Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                                    TextureDimension::D2,
                                    data,
                                    TextureFormat::Rgba8UnormSrgb,
                                    RenderAssetUsages::default(),
                                );
                                img.sampler = ImageSampler::linear();
                                let hd = images.add(img);
                                skin.composed[i] = Some(hd.clone());
                                hd
                            }
                        };
                        if let Some(mat) = mats.get_mut(&skin.mats[i].handle) {
                            mat.base.base_color_texture = Some(handle);
                        }
                    }
                    skin.status = format!("skin: {} layer(s)", skin.layers.len());
                }
                Err(e) => skin.status = format!("skin layers: {e:#}"),
            }
        }
    }
    if !skin.layers_dirty || skin.task.is_some() {
        return;
    }
    if let Some(a) = skin.albedo {
        for (i, m) in skin.mats.iter().enumerate() {
            if let (Some(_), Some(mat)) = (&skin.originals[i], mats.get_mut(&m.handle)) {
                mat.base.base_color_texture = None;
                mat.base.base_color = Color::srgb_u8(a[0], a[1], a[2]);
            }
        }
        skin.layers_dirty = false;
        return;
    }
    for (i, m) in skin.mats.iter().enumerate() {
        if let (Some(o), Some(mat)) = (&skin.originals[i], mats.get_mut(&m.handle)) {
            if mat.base.base_color != o.base_color {
                mat.base.base_color = o.base_color;
            }
        }
    }
    // nothing over the character's own skin: its own texture
    if skin.layers.is_empty() && skin.skin.is_none() {
        for (i, m) in skin.mats.iter().enumerate() {
            if let (Some(o), Some(mat)) = (&skin.originals[i], mats.get_mut(&m.handle)) {
                mat.base.base_color_texture = o.diffuse.clone();
            }
        }
        skin.layers_dirty = false;
        return;
    }
    // the character's diffuse texels, once they have loaded
    for i in 0..skin.mats.len() {
        if skin.bases[i].is_some() {
            continue;
        }
        let Some(h) = skin.originals[i].as_ref().and_then(|o| o.diffuse.clone()) else { continue };
        let Some(img) = images.get(&h) else { return };
        if img.texture_descriptor.format != TextureFormat::Rgba8UnormSrgb {
            skin.status = format!("skin: {} diffuse is {:?}, not RGBA8 sRGB", skin.mats[i].name, img.texture_descriptor.format);
            skin.layers_dirty = false;
            return;
        }
        skin.bases[i] = Some(Arc::new((img.width(), img.height(), img.data.clone())));
    }
    skin.layers_dirty = false;
    skin.task_key = skin.key();
    let jobs: Vec<(usize, String, Option<Arc<(u32, u32, Vec<u8>)>>, Option<PathBuf>)> = skin
        .mats
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let preset = skin.skin.as_ref().and_then(|id| pack.skin(id)).and_then(|s| s.maps.get(&m.name)).and_then(|mm| mm.get("Diffuse")).map(|r| pack.dir.join(r));
            (i, m.name.clone(), skin.bases[i].clone(), preset)
        })
        .collect();
    let layers = skin.layers.clone();
    let cache = skin.cache.clone();
    let pack2 = pack.clone();
    skin.status = "skin: compositing...".into();
    skin.task = Some(AsyncComputeTaskPool::get().spawn(async move {
        let t0 = std::time::Instant::now();
        let mut out = Vec::new();
        for (i, name, base, preset) in jobs {
            let (w, h, mut data) = match preset {
                Some(p) => {
                    let img = image::open(&p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?.to_rgba8();
                    (img.width(), img.height(), img.into_raw())
                }
                None => match base {
                    Some(b) => (b.0, b.1, b.2.clone()),
                    None => continue,
                },
            };
            let mut imgs = Vec::new();
            for l in &layers {
                let Some(layer) = pack2.layer(&l.id) else { continue };
                let key = (l.id.clone(), name.clone());
                let cached = cache.lock().unwrap().get(&key).cloned();
                let li = match cached {
                    Some(c) => c,
                    None => {
                        let li = pack2.layer_image(layer, &name)?.map(Arc::new);
                        cache.lock().unwrap().insert(key, li.clone());
                        li
                    }
                };
                if let Some(li) = li {
                    imgs.push((li, l.strength, l.tint.map(|t| (t.map(|x| x as f32), layer.color))));
                }
            }
            let applies: Vec<Apply> = imgs.iter().map(|(li, s, t)| Apply { image: li, strength: *s, tint: *t }).collect();
            compose(&mut data, w, h, &applies);
            out.push((i, w, h, data));
        }
        info!("skin: composited {} layer(s) in {:.0} ms", layers.len(), t0.elapsed().as_secs_f32() * 1000.0);
        Ok(out)
    }));
}

fn number(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

fn color(v: &Value) -> Option<[u8; 3]> {
    let a = v.as_array()?;
    let c = |i: usize| a.get(i).and_then(Value::as_f64).map(|x| x.round().clamp(0.0, 255.0) as u8);
    Some([c(0)?, c(1)?, c(2)?])
}

pub fn skin_ui(ui: &mut egui::Ui, skin: &mut SkinEdit) {
    egui::CollapsingHeader::new("Skin").default_open(false).show(ui, |ui| {
        if skin.mats.is_empty() {
            ui.label("waiting for the character's CC5 skin materials...");
            return;
        }
        ui.horizontal(|ui| {
            ui.label("Recipe");
            ui.text_edit_singleline(&mut skin.path);
            if ui.button("Load").clicked() {
                let r = std::fs::read(&skin.path)
                    .map_err(anyhow::Error::from)
                    .and_then(|b| Ok(serde_json::from_slice::<Value>(&b)?))
                    .and_then(|v| skin.apply(&v));
                skin.status = match r {
                    Ok(()) => format!("loaded {}", skin.path),
                    Err(e) => format!("load failed: {e:#}"),
                };
            }
            if ui.button("Save").clicked() {
                let json = serde_json::to_vec_pretty(&skin.recipe()).unwrap();
                skin.status = match std::fs::write(&skin.path, json) {
                    Ok(()) => format!("saved {}", skin.path),
                    Err(e) => format!("save failed: {e}"),
                };
            }
            if ui.button("Reset").clicked() {
                skin.reset();
            }
        });
        if !skin.status.is_empty() {
            ui.small(&skin.status);
        }
        if let Some(pack) = skin.pack.clone() {
            ui.horizontal(|ui| {
                ui.label("Skin");
                let label = skin.skin.as_ref().and_then(|id| pack.skin(id)).map_or("Character's own".to_string(), |s| s.name.clone());
                let mut choice = skin.skin.clone();
                egui::ComboBox::from_id_salt("skin_preset").selected_text(label).show_ui(ui, |ui| {
                    ui.selectable_value(&mut choice, None, "Character's own");
                    for s in &pack.skins {
                        ui.selectable_value(&mut choice, Some(s.id.clone()), &s.name);
                    }
                });
                if choice != skin.skin {
                    skin.set_skin(choice);
                }
            });
            egui::CollapsingHeader::new(format!("SkinGen layers ({} on)", skin.layers.len())).id_salt("skin_layers").show(ui, |ui| {
                let mut groups: Vec<&str> = pack.layers.iter().map(|l| l.group.as_str()).collect();
                groups.dedup();
                for g in groups {
                    ui.label(egui::RichText::new(g).strong());
                    for l in pack.layers.iter().filter(|l| l.group == g) {
                        let pos = skin.layers.iter().position(|s| s.id == l.id);
                        ui.horizontal(|ui| {
                            let mut on = pos.is_some();
                            if ui.checkbox(&mut on, &l.name).changed() {
                                match (on, pos) {
                                    (true, None) => skin.layers.push(LayerState { id: l.id.clone(), strength: 1.0, tint: None }),
                                    (false, Some(p)) => {
                                        skin.layers.remove(p);
                                    }
                                    _ => {}
                                }
                                skin.layers_dirty = true;
                            }
                            if let Some(p) = pos {
                                let st = &mut skin.layers[p];
                                if ui.add(egui::Slider::new(&mut st.strength, 0.0..=1.0).text("strength")).changed() {
                                    skin.layers_dirty = true;
                                }
                                let st = &mut skin.layers[p];
                                let mut c = st.tint.unwrap_or(l.color.map(|x| x.round().clamp(0.0, 255.0) as u8));
                                if ui.color_edit_button_srgb(&mut c).changed() {
                                    st.tint = Some(c);
                                    skin.layers_dirty = true;
                                }
                                if skin.layers[p].tint.is_some() && ui.small_button("↺").on_hover_text("CC5's colour").clicked() {
                                    skin.layers[p].tint = None;
                                    skin.layers_dirty = true;
                                }
                            }
                        });
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("Material");
            let label = skin.selected.map_or("All skin".to_string(), |i| skin.mats[i].name.clone());
            egui::ComboBox::from_id_salt("skin_material").selected_text(label).show_ui(ui, |ui| {
                ui.selectable_value(&mut skin.selected, None, "All skin");
                for i in 0..skin.mats.len() {
                    let n = skin.mats[i].name.clone();
                    ui.selectable_value(&mut skin.selected, Some(i), n);
                }
            });
        });
        let targets = skin.targets();
        for group in GROUPS {
            // settings the targeted materials have
            let shown: Vec<&Setting> = SETTINGS
                .iter()
                .filter(|s| s.group == group && targets.iter().any(|&i| skin.mats[i].store(s.store).get(s.key).is_some()))
                .collect();
            if shown.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(group).id_salt(("skin", group)).show(ui, |ui| {
                for s in shown {
                    let first = targets.iter().copied().find(|&i| skin.mats[i].store(s.store).get(s.key).is_some()).unwrap();
                    let cur = skin.mats[first].store(s.store)[s.key].clone();
                    let new = setting_widget(ui, s, &cur);
                    if let Some(v) = new {
                        for &i in &targets {
                            let st = skin.mats[i].store_mut(s.store);
                            if st.get(s.key).is_some() {
                                st[s.key] = v.clone();
                            }
                        }
                        skin.dirty = true;
                    }
                }
            });
        }
    });
}

/// The widget for one setting; Some(new value) when changed.
fn setting_widget(ui: &mut egui::Ui, s: &Setting, cur: &Value) -> Option<Value> {
    let label = s.key.trim_start_matches('_');
    match s.kind {
        Kind::Float(min, max) => {
            let mut x = number(cur).unwrap_or(0.0);
            let r = ui.add(egui::Slider::new(&mut x, min..=max).text(label).clamping(egui::SliderClamping::Never));
            r.changed().then(|| json!(x))
        }
        Kind::Toggle => {
            let mut on = number(cur).unwrap_or(0.0) > 0.5;
            ui.checkbox(&mut on, label).changed().then(|| json!(if on { 1.0 } else { 0.0 }))
        }
        Kind::Color => {
            let mut rgb = color(cur).unwrap_or([255, 255, 255]);
            ui.horizontal(|ui| {
                let r = ui.color_edit_button_srgb(&mut rgb);
                ui.label(label);
                r.changed().then(|| json!(rgb.map(|x| x as f64)))
            })
            .inner
        }
    }
}
