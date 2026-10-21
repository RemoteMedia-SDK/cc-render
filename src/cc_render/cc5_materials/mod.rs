//! CC5 shader materials for character GLBs.
//!
//! CC5 renders skin, eyes, hair and teeth with its own shaders (RLHead, RLSkin,
//! RLEye, RLHair, RLTeethGum, ...) and their extra maps. glTF has no slots for those,
//! so `remotemedia-sdk/scripts/avatars/cc5_editor/glb_cc5_materials.py` writes
//! the maps next to the GLB (`<name>.cc5maps/`) and records, per material,
//! `extras.cc5`: `{"shader", "maps": {slot: path relative to the GLB},
//! "srgb": [colour slots], "vars": {...}, "sss": {...}, ...}`. (Maps embedded
//! as glTF textures — `--embed`, slot: texture index — are not usable here:
//! Bevy's glTF loader drops textures no StandardMaterial references.)
//!
//! [`Cc5MaterialsPlugin`] reads that back: every glTF primitive whose material
//! carries a supported CC5 shader gets a [`Cc5Material`] holding its original
//! `StandardMaterial` and a CC5 equivalent; [`Cc5Materials::enabled`] selects
//! which one renders (handle swaps only — material assets are never mutated).
//! GLBs without `extras.cc5` are unaffected.
//!
//! Supported (each an `ExtendedMaterial<StandardMaterial, _>` approximating the
//! CC5 shader from its maps and variables; the variable names and ranges follow
//! CC5's `Program/Assets/Share/Shaders/**/*.shader` declarations):
//!   - RLHead, RLSkin: [`Cc5SkinMaterial`], `skin.wgsl`;
//!   - RLHair: [`Cc5HairMaterial`], `hair.wgsl` (+ `hair_types.wgsl`, `hair_prepass.wgsl`:
//!     dithered strand coverage for the camera's TAA, translucent shadows);
//!   - RLEye (on the cornea, which then draws the whole eye): [`Cc5EyeMaterial`], `eye.wgsl`;
//!   - RLEyeOcclusion(_Plus), RLEyeTearline(_Plus): [`Cc5EyeOverlayMaterial`], `eye_overlay.wgsl`;
//!   - RLTeethGum, RLTongue: [`Cc5MouthMaterial`], `mouth.wgsl`.
use std::collections::HashMap;

use bevy::asset::{load_internal_asset, AssetPath};
use bevy::ecs::system::EntityCommands;
use bevy::gltf::{GltfMaterialExtras, GltfMaterialName};
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef, ShaderType};
use serde_json::Value;

mod mipmaps;
pub use mipmaps::generate_mips;

const SKIN_SHADER: Handle<Shader> = Handle::weak_from_u128(0x6c3a_9e1f_5b2d_4c8e_a7f0_3d91_c5e2_b804);
const HAIR_SHADER: Handle<Shader> = Handle::weak_from_u128(0x6c3a_9e1f_5b2d_4c8e_a7f0_3d91_c5e2_b805);
const EYE_SHADER: Handle<Shader> = Handle::weak_from_u128(0x6c3a_9e1f_5b2d_4c8e_a7f0_3d91_c5e2_b806);
const EYE_OVERLAY_SHADER: Handle<Shader> = Handle::weak_from_u128(0x6c3a_9e1f_5b2d_4c8e_a7f0_3d91_c5e2_b807);
const MOUTH_SHADER: Handle<Shader> = Handle::weak_from_u128(0x6c3a_9e1f_5b2d_4c8e_a7f0_3d91_c5e2_b808);
const COMMON_SHADER: Handle<Shader> = Handle::weak_from_u128(0x6c3a_9e1f_5b2d_4c8e_a7f0_3d91_c5e2_b809);
const HAIR_PREPASS_SHADER: Handle<Shader> = Handle::weak_from_u128(0x6c3a_9e1f_5b2d_4c8e_a7f0_3d91_c5e2_b80a);
const HAIR_TYPES_SHADER: Handle<Shader> = Handle::weak_from_u128(0x6c3a_9e1f_5b2d_4c8e_a7f0_3d91_c5e2_b80b);

/// CC5 RLHead / RLSkin: StandardMaterial plus [`Cc5Skin`].
pub type Cc5SkinMaterial = ExtendedMaterial<StandardMaterial, Cc5Skin>;
/// CC5 RLHair: StandardMaterial plus [`Cc5Hair`].
pub type Cc5HairMaterial = ExtendedMaterial<StandardMaterial, Cc5Hair>;
/// CC5 RLEye: StandardMaterial plus [`Cc5Eye`].
pub type Cc5EyeMaterial = ExtendedMaterial<StandardMaterial, Cc5Eye>;
/// CC5 RLEyeOcclusion / RLEyeTearline: StandardMaterial plus [`Cc5EyeOverlay`].
pub type Cc5EyeOverlayMaterial = ExtendedMaterial<StandardMaterial, Cc5EyeOverlay>;
/// CC5 RLTeethGum / RLTongue: StandardMaterial plus [`Cc5Mouth`].
pub type Cc5MouthMaterial = ExtendedMaterial<StandardMaterial, Cc5Mouth>;

/// Which materials CC5 characters render with.
#[derive(Resource, Debug, Clone)]
pub struct Cc5Materials {
    /// `true`: CC5 shader materials; `false`: the GLB's plain StandardMaterials.
    pub enabled: bool,
    /// Scales the subsurface term (1 = default calibration); read when a
    /// character's materials are built.
    pub sss_strength: f32,
}

impl Default for Cc5Materials {
    fn default() -> Self {
        Self { enabled: true, sss_strength: 1.0 }
    }
}

/// The CC5 version of a primitive's material.
#[derive(Debug, Clone)]
pub enum Cc5Handle {
    Skin(Handle<Cc5SkinMaterial>),
    Hair(Handle<Cc5HairMaterial>),
    Eye(Handle<Cc5EyeMaterial>),
    EyeOverlay(Handle<Cc5EyeOverlayMaterial>),
    Mouth(Handle<Cc5MouthMaterial>),
}

impl Cc5Handle {
    fn insert(&self, ec: &mut EntityCommands) {
        match self {
            Cc5Handle::Skin(h) => ec.insert(MeshMaterial3d(h.clone())),
            Cc5Handle::Hair(h) => ec.insert(MeshMaterial3d(h.clone())),
            Cc5Handle::Eye(h) => ec.insert(MeshMaterial3d(h.clone())),
            Cc5Handle::EyeOverlay(h) => ec.insert(MeshMaterial3d(h.clone())),
            Cc5Handle::Mouth(h) => ec.insert(MeshMaterial3d(h.clone())),
        };
    }

    fn remove_all(ec: &mut EntityCommands) {
        ec.remove::<(
            MeshMaterial3d<Cc5SkinMaterial>,
            MeshMaterial3d<Cc5HairMaterial>,
            MeshMaterial3d<Cc5EyeMaterial>,
            MeshMaterial3d<Cc5EyeOverlayMaterial>,
            MeshMaterial3d<Cc5MouthMaterial>,
        )>();
    }
}

/// On a primitive with a CC5 shader material: both versions of its material.
#[derive(Component, Debug, Clone)]
pub struct Cc5Material {
    pub shader: String,
    pub standard: Handle<StandardMaterial>,
    pub cc5: Cc5Handle,
    /// Primitives the CC5 material replaces, hidden while it renders (RLEye: the
    /// eyeball under the cornea, which then draws the whole eye).
    pub hides: Vec<Entity>,
}

/// The RLHead / RLSkin extension. Texture slots, in `skin.wgsl` bit order:
/// micro normal, micro normal mask, SSS, transmission, region masks A/B/C,
/// mouth cavity AO, specular mask.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct Cc5Skin {
    #[uniform(100)]
    pub params: Cc5SkinParams,
    #[texture(101)]
    #[sampler(102)]
    pub micro_normal: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    pub micro_mask: Option<Handle<Image>>,
    #[texture(105)]
    #[sampler(106)]
    pub sss: Option<Handle<Image>>,
    #[texture(107)]
    #[sampler(108)]
    pub transmission: Option<Handle<Image>>,
    #[texture(109)]
    #[sampler(110)]
    pub mask_a: Option<Handle<Image>>,
    #[texture(111)]
    #[sampler(112)]
    pub mask_b: Option<Handle<Image>>,
    #[texture(113)]
    #[sampler(114)]
    pub mask_c: Option<Handle<Image>>,
    #[texture(115)]
    #[sampler(116)]
    pub mouth_ao: Option<Handle<Image>>,
    #[texture(117)]
    #[sampler(118)]
    pub specular: Option<Handle<Image>>,
}

/// Uniform of [`Cc5Skin`]; field meanings in `skin.wgsl`.
#[derive(ShaderType, Reflect, Debug, Clone, Default)]
pub struct Cc5SkinParams {
    pub rough_a: Vec4,
    pub rough_b: Vec4,
    pub rough_c: Vec4,
    pub scatter_a: Vec4,
    pub scatter_b: Vec4,
    pub scatter_c: Vec4,
    pub falloff: Vec4,
    pub micro: Vec4,
    pub misc: Vec4,
    pub ao: Vec4,
    pub rough: Vec4,
    pub present: u32,
    pub srgb: u32,
    /// Channels of mask C that are regions (bit per channel; RLHead's ear/neck
    /// mask has an opaque alpha that is not a region).
    pub mask_c_channels: u32,
    pub _pad1: u32,
}

impl MaterialExtension for Cc5Skin {
    fn fragment_shader() -> ShaderRef {
        SKIN_SHADER.into()
    }
}

/// The RLHair extension. Texture slots, in `hair.wgsl` bit order: root map,
/// ID map, flow map, specular mask.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct Cc5Hair {
    #[uniform(100)]
    pub params: Cc5HairParams,
    #[texture(101)]
    #[sampler(102)]
    pub root: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    pub id: Option<Handle<Image>>,
    #[texture(105)]
    #[sampler(106)]
    pub flow: Option<Handle<Image>>,
    #[texture(107)]
    #[sampler(108)]
    pub spec_mask: Option<Handle<Image>>,
}

/// Uniform of [`Cc5Hair`]; field meanings in `hair.wgsl`.
#[derive(ShaderType, Reflect, Debug, Clone, Default)]
pub struct Cc5HairParams {
    pub root_color: Vec4,
    pub tip_color: Vec4,
    pub dye1: Vec4,
    pub dye1_range: Vec4,
    pub dye2: Vec4,
    pub dye2_range: Vec4,
    pub color: Vec4,
    pub light: Vec4,
    pub maps: Vec4,
    pub tangent: Vec4,
    pub lobes: Vec4,
    pub alpha: Vec4,
    pub present: u32,
    pub srgb: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

impl MaterialExtension for Cc5Hair {
    fn fragment_shader() -> ShaderRef {
        HAIR_SHADER.into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        HAIR_PREPASS_SHADER.into()
    }
}

/// The RLEye extension, on the cornea. Texture slots, in `eye.wgsl` bit order:
/// sclera colour, iris normal, sclera normal, inner iris mask.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct Cc5Eye {
    #[uniform(100)]
    pub params: Cc5EyeParams,
    #[texture(101)]
    #[sampler(102)]
    pub sclera: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    pub iris_normal: Option<Handle<Image>>,
    #[texture(105)]
    #[sampler(106)]
    pub sclera_normal: Option<Handle<Image>>,
    #[texture(107)]
    #[sampler(108)]
    pub iris_mask: Option<Handle<Image>>,
}

/// Uniform of [`Cc5Eye`]; field meanings in `eye.wgsl`.
#[derive(ShaderType, Reflect, Debug, Clone, Default)]
pub struct Cc5EyeParams {
    pub iris: Vec4,
    pub iris_color: Vec4,
    pub limbus: Vec4,
    pub sclera: Vec4,
    pub corner: Vec4,
    pub spec: Vec4,
    pub present: u32,
    pub srgb: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

impl MaterialExtension for Cc5Eye {
    fn fragment_shader() -> ShaderRef {
        EYE_SHADER.into()
    }
}

/// The RLEyeOcclusion / RLEyeTearline extension (no maps: both are procedural
/// over the mesh's UV layout).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct Cc5EyeOverlay {
    #[uniform(100)]
    pub params: Cc5EyeOverlayParams,
}

/// Uniform of [`Cc5EyeOverlay`]; field meanings in `eye_overlay.wgsl`.
#[derive(ShaderType, Reflect, Debug, Clone, Default)]
pub struct Cc5EyeOverlayParams {
    /// x: 0 occlusion, 1 tearline
    pub kind: Vec4,
    pub shadow_color: Vec4,
    pub shadow_top: Vec4,
    pub shadow_bottom: Vec4,
    pub shadow_inner: Vec4,
    pub shadow_outer: Vec4,
    pub blur_color: Vec4,
    pub blur_top: Vec4,
    pub blur_bottom: Vec4,
    pub blur_inner: Vec4,
    pub blur_outer: Vec4,
    pub tear: Vec4,
}

impl MaterialExtension for Cc5EyeOverlay {
    fn fragment_shader() -> ShaderRef {
        EYE_OVERLAY_SHADER.into()
    }
}

/// The RLTeethGum / RLTongue extension. Texture slots, in `mouth.wgsl` bit
/// order: micro normal, gradient AO, gums mask.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct Cc5Mouth {
    #[uniform(100)]
    pub params: Cc5MouthParams,
    #[texture(101)]
    #[sampler(102)]
    pub micro_normal: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    pub gradient_ao: Option<Handle<Image>>,
    #[texture(105)]
    #[sampler(106)]
    pub gums_mask: Option<Handle<Image>>,
}

/// Uniform of [`Cc5Mouth`]; field meanings in `mouth.wgsl`.
#[derive(ShaderType, Reflect, Debug, Clone, Default)]
pub struct Cc5MouthParams {
    pub teeth: Vec4,
    pub gums: Vec4,
    pub front: Vec4,
    pub back: Vec4,
    pub micro: Vec4,
    pub edge_color: Vec4,
    pub present: u32,
    pub srgb: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

impl MaterialExtension for Cc5Mouth {
    fn fragment_shader() -> ShaderRef {
        MOUTH_SHADER.into()
    }
}

pub struct Cc5MaterialsPlugin;

impl Plugin for Cc5MaterialsPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, COMMON_SHADER, "common.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, SKIN_SHADER, "skin.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, HAIR_SHADER, "hair.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, HAIR_TYPES_SHADER, "hair_types.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, HAIR_PREPASS_SHADER, "hair_prepass.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, EYE_SHADER, "eye.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, EYE_OVERLAY_SHADER, "eye_overlay.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, MOUTH_SHADER, "mouth.wgsl", Shader::from_wgsl);
        app.add_plugins((
            MaterialPlugin::<Cc5SkinMaterial>::default(),
            MaterialPlugin::<Cc5HairMaterial>::default(),
            MaterialPlugin::<Cc5EyeMaterial>::default(),
            MaterialPlugin::<Cc5EyeOverlayMaterial>::default(),
            MaterialPlugin::<Cc5MouthMaterial>::default(),
        ))
        .init_resource::<Cc5Materials>()
        .add_systems(Update, (build_cc5_materials, sync_cc5_materials, mipmaps::hair_mipmaps).chain().in_set(Cc5MaterialsSet));
    }
}

/// Systems that build and swap CC5 materials (run in `Update`). Order handle swaps
/// of your own after this set.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cc5MaterialsSet;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn var(vars: &Value, key: &str) -> f32 {
    var_or(vars, key, 0.0)
}

fn var_or(vars: &Value, key: &str, default: f32) -> f32 {
    vars.get(key).and_then(Value::as_f64).map_or(default, |v| v as f32)
}

fn vec4_of(vars: &Value, keys: [&str; 4]) -> Vec4 {
    Vec4::new(var(vars, keys[0]), var(vars, keys[1]), var(vars, keys[2]), var(vars, keys[3]))
}

/// Element `i` of an array variable (CC5 colours are 0..255 sRGB triples,
/// ranges 2- or 3-element arrays).
fn var_at(vars: &Value, key: &str, i: usize, default: f32) -> f32 {
    vars.get(key).and_then(Value::as_array).and_then(|a| a.get(i)).and_then(Value::as_f64).map_or(default, |v| v as f32)
}

/// A CC5 colour variable (0..255 sRGB) as linear RGB.
fn color_var(vars: &Value, key: &str, default: [f32; 3]) -> Vec3 {
    let c = |i: usize| srgb_to_linear(var_at(vars, key, i, default[i] * 255.0) / 255.0);
    Vec3::new(c(0), c(1), c(2))
}

/// A CC5 range variable `[min, max]` as (min, max).
fn range_var(vars: &Value, key: &str) -> Vec2 {
    Vec2::new(var_at(vars, key, 0, 0.0), var_at(vars, key, 1, 1.0))
}

/// Parsed `extras.cc5` with texture slots resolved to image handles.
struct Cc5Info {
    shader: String,
    /// The glTF material name (lowercase).
    name: String,
    vars: Value,
    sss: Value,
    maps: HashMap<String, Handle<Image>>,
}

/// The textures of `slots` (absent slots: `None`), with bit masks of which are
/// present and which the loader decoded as sRGB; `None` while any is loading.
/// A map that failed to load counts as absent.
fn resolve_maps<const N: usize>(
    info: &Cc5Info,
    images: &Assets<Image>,
    assets: &AssetServer,
    slots: [&str; N],
) -> Option<([Option<Handle<Image>>; N], u32, u32)> {
    let mut tex: [Option<Handle<Image>>; N] = std::array::from_fn(|_| None);
    let (mut present, mut srgb) = (0u32, 0u32);
    for (i, slot) in slots.iter().enumerate() {
        let Some(h) = info.maps.get(*slot) else { continue };
        // Wait for the image: its format says whether the loader decoded it as sRGB.
        let Some(img) = images.get(h) else {
            if assets.load_state(h).is_failed() {
                continue;
            }
            return None;
        };
        present |= 1 << i;
        if img.texture_descriptor.format.is_srgb() {
            srgb |= 1 << i;
        }
        tex[i] = Some(h.clone());
    }
    Some((tex, present, srgb))
}

fn skin_extension(info: &Cc5Info, images: &Assets<Image>, assets: &AssetServer, sss_strength: f32) -> Option<Cc5Skin> {
    let v = &info.vars;
    let head = info.shader == "RLHead";
    let (slot_a, slot_b, slot_c) = if head {
        ("Cheek Fore UpperLip Chin Mask", "Nose Mouth UpperInnerLid Mask", "Ear Neck Mask")
    } else {
        ("RGBA Area Mask", "", "")
    };
    let slots = [
        "MicroNormal",
        "MicroNormalMask",
        "SSS Map",
        "Transmission Map",
        slot_a,
        slot_b,
        slot_c,
        "Mouth Cavity Mask and AO",
        "Specular Mask",
    ];
    let (tex, present, srgb) = resolve_maps(info, images, assets, slots)?;
    let rough = |keys: [&str; 4]| vec4_of(v, keys);
    let (rough_a, rough_b, rough_c, scatter_a, scatter_b, scatter_c) = if head {
        let r = |n: &str| format!("{n} Roughness Scale");
        let s = |n: &str| format!("{n} Scatter Scale");
        let four = |f: &dyn Fn(&str) -> String, a: [&str; 4]| {
            let k = a.map(|n| if n.is_empty() { String::new() } else { f(n) });
            vec4_of(v, [&k[0], &k[1], &k[2], &k[3]])
        };
        // Ear Neck mask: the neck/jaw region is red, the ears green (the map's
        // layout, whatever the slot name's order says).
        (
            four(&r, ["Cheek", "Forehead", "UpperLip", "Chin"]),
            four(&r, ["Nose", "Mouth", "UpperLid", "InnerLid"]),
            four(&r, ["Neck", "Ear", "", ""]),
            four(&s, ["Cheek", "Forehead", "UpperLip", "Chin"]),
            four(&s, ["Nose", "Mouth", "UpperLid", "InnerLid"]),
            four(&s, ["Neck", "Ear", "", ""]),
        )
    } else {
        (
            rough([
                "R Channel Roughness Scale",
                "G Channel Roughness Scale",
                "B Channel Roughness Scale",
                "A Channel Roughness Scale",
            ]),
            Vec4::ZERO,
            Vec4::ZERO,
            rough(["R Channel Scatter Scale", "G Channel Scatter Scale", "B Channel Scatter Scale", "A Channel Scatter Scale"]),
            Vec4::ZERO,
            Vec4::ZERO,
        )
    };
    let falloff = info
        .sss
        .get("Falloff")
        .and_then(Value::as_array)
        .map(|a| {
            let c = |i: usize| srgb_to_linear(a.get(i).and_then(Value::as_f64).unwrap_or(255.0) as f32 / 255.0);
            Vec3::new(c(0), c(1), c(2))
        })
        .unwrap_or(Vec3::new(1.0, 0.11, 0.07));
    // CC5's SSS radius (mm-ish, ~2 for skin) widens the wrap of light past the terminator.
    let radius = info.sss.get("Radius").and_then(Value::as_f64).unwrap_or(2.0) as f32;
    let [micro_normal, micro_mask, sss, transmission, mask_a, mask_b, mask_c, mouth_ao, specular] = tex;
    let keep_rough = var(v, "Keep Original Roughness") > 0.5;
    Some(Cc5Skin {
        params: Cc5SkinParams {
            rough_a,
            rough_b,
            rough_c,
            scatter_a,
            scatter_b,
            scatter_c,
            falloff: falloff.extend(0.35 * sss_strength),
            micro: Vec4::new(
                var(v, "MicroNormal Tiling").max(1.0),
                var(v, "MicroNormal Strength"),
                var(v, "Flip MicroNormal Y"),
                if keep_rough { 0.0 } else { var(v, "Micro Roughness Scale") },
            ),
            misc: Vec4::new(
                if v.get("_Specular").is_some() { var(v, "_Specular") } else { 0.5 },
                if keep_rough { 0.0 } else { var(v, "Unmasked Roughness Scale") },
                var_or(v, "Unmasked Scatter Scale", 1.0),
                0.4,
            ),
            // the mask's channels: R mouth cavity, G mouth / lips gap, B nostrils
            ao: Vec4::new(var(v, "Inner Mouth Ao"), var(v, "Lips Gap Ao"), var(v, "Nostril Ao"), 0.25 + 0.15 * radius),
            rough: Vec4::new(
                if keep_rough { 1.0 } else { var_or(v, "Original Roughness Strength", 0.85) },
                var(v, "Edge Roughness Multiplier"),
                if keep_rough { 0.0 } else { 1.0 },
                0.0,
            ),
            present,
            srgb,
            mask_c_channels: if head { 0b0011 } else { 0b1111 },
            _pad1: 0,
        },
        micro_normal,
        micro_mask,
        sss,
        transmission,
        mask_a,
        mask_b,
        mask_c,
        mouth_ao,
        specular,
    })
}

/// Brows and lashes: thin strands (sharper alpha edges, faint highlights).
fn fine_hair(info: &Cc5Info) -> bool {
    info.name.contains("lash") || info.name.contains("brow")
}

fn hair_extension(info: &Cc5Info, images: &Assets<Image>, assets: &AssetServer) -> Option<Cc5Hair> {
    let v = &info.vars;
    let slots = ["Hair Root Map", "Hair ID Map", "Hair Flow Map", "Hair Specular Mask Map"];
    let (tex, present, srgb) = resolve_maps(info, images, assets, slots)?;
    let active = var_or(v, "ActiveChangeHairColor", 1.0) * var_or(v, "UseRootTipColor", 1.0);
    // Dye distribution: three ID-map levels (0..255) where the dye starts, peaks and ends.
    let dye = |n: &str| {
        let color = color_var(v, &format!("_{n} Dye Color"), [1.0; 3]);
        let range = Vec4::new(
            var_at(v, &format!("_{n} Dye Distribution from Grayscale"), 0, 0.0) / 255.0,
            var_at(v, &format!("_{n} Dye Distribution from Grayscale"), 1, 127.5) / 255.0,
            var_at(v, &format!("_{n} Dye Distribution from Grayscale"), 2, 255.0) / 255.0,
            // 0: everywhere, 1: masked by the root map (tips), 2: by its inverse (roots)
            if var(v, &format!("Mask {n} Dye by RootMap")) > 0.5 {
                if var(v, &format!("Invert {n} Dye RootMap Mask")) > 0.5 { 2.0 } else { 1.0 }
            } else {
                0.0
            },
        );
        let strength = var(v, &format!("_{n} Dye Strength")) * var_or(v, "ActiveChangeHairColor", 1.0);
        (color.extend(strength), range, var(v, &format!("_{n} BlendMode")))
    };
    let (dye1, dye1_range, dye1_mode) = dye("1st");
    let (dye2, dye2_range, dye2_mode) = dye("2nd");
    let [root, id, flow, spec_mask] = tex;
    Some(Cc5Hair {
        params: Cc5HairParams {
            root_color: color_var(v, "RootColor", [1.0; 3]).extend(var_or(v, "RootColorStrength", 1.0) * active),
            tip_color: color_var(v, "TipColor", [1.0; 3]).extend(var_or(v, "TipColorStrength", 1.0) * active),
            dye1,
            dye1_range,
            dye2,
            dye2_range,
            color: Vec4::new(
                var_or(v, "RootTipBlendMode", 1.0),
                var(v, "InvertRootTip"),
                var_or(v, "BaseColorMapStrength", 1.0),
                dye1_mode + 8.0 * dye2_mode,
            ),
            light: Vec4::new(
                var_or(v, "Diffuse Strength", 0.8),
                var_or(v, "Specular Strength", 1.0),
                var_or(v, "Secondary Specular Strength", 1.0),
                var_or(v, "Transmission Strength", 1.0),
            ),
            maps: Vec4::new(
                var_or(v, "Hair Specular Map Strength", 1.0),
                var_or(v, "Hair Roughness Map Strength", 1.0),
                var_or(v, "TangentMapFlipGreen", 1.0),
                var(v, "AO Map Occlude All Lighting"),
            ),
            tangent: Vec3::new(
                var_at(v, "TangentVectorColor", 0, 255.0) / 255.0,
                var_at(v, "TangentVectorColor", 1, 0.0) / 255.0,
                var_at(v, "TangentVectorColor", 2, 0.0) / 255.0,
            )
            // w: highlight scale (lashes / brows: faint, no sub-pixel glitter)
            .extend(if fine_hair(info) { 0.15 } else { 1.0 }),
            // primary shift, secondary shift, per-strand shift jitter, ID brightness jitter
            lobes: Vec4::new(-0.06, 0.08, 0.08, 0.12),
            // cutoff, soft coverage ramp, opacity mip bias, dithered (hair_types.wgsl):
            // scalp hair blends into CC5's soft mass (the negative bias keeps single
            // strands resolved; TAA smooths them); lashes / brows stay crisp clumps
            alpha: if fine_hair(info) { Vec4::new(0.3, 0.1, 0.0, 0.0) } else { Vec4::new(0.38, 0.3, -1.0, 1.0) },
            present,
            srgb,
            _pad0: 0,
            _pad1: 0,
        },
        root,
        id,
        flow,
        spec_mask,
    })
}

fn eye_extension(info: &Cc5Info, images: &Assets<Image>, assets: &AssetServer) -> Option<Cc5Eye> {
    let v = &info.vars;
    let slots = ["Sclera", "Iris Normal", "Sclera Normal", "Inner Iris Mask"];
    let (tex, present, srgb) = resolve_maps(info, images, assets, slots)?;
    // CC's eye textures paint the iris at radius 0.125 (cornea UV); CC5 draws it
    // scaled by Iris UV Radius against its default 0.16 (0.22: 1.375x larger).
    const TEXTURE_IRIS_RADIUS: f32 = 0.125;
    let iris_radius = TEXTURE_IRIS_RADIUS * var_or(v, "Iris UV Radius", 0.16) / 0.16;
    let ior = var_or(v, "_IoR", 1.4);
    let [sclera, iris_normal, sclera_normal, iris_mask] = tex;
    Some(Cc5Eye {
        params: Cc5EyeParams {
            // x: iris radius (UV), y: depth scale, z: pupil scale, w: 1 / IoR
            iris: Vec4::new(iris_radius, var_or(v, "Iris Depth Scale", 1.0), var_or(v, "Pupil Scale", 1.0), 1.0 / ior.max(1.0)),
            iris_color: (color_var(v, "Iris Color", [1.0; 3]) * var_or(v, "Iris Color Brightness", 1.0))
                .extend(var_or(v, "Iris Inner Scale", 0.0)),
            limbus: Vec4::new(
                var_or(v, "Limbus UV Width Color", 0.055) * 0.5,
                var_or(v, "Limbus Dark Scale", 5.0),
                var_or(v, "Iris UV Scale", 1.0),
                TEXTURE_IRIS_RADIUS,
            ),
            sclera: Vec4::new(
                var_or(v, "ScleraBrightness", 0.65),
                var_or(v, "Sclera Flatten Normal", 1.0),
                var_or(v, "Sclera Normal UV Scale", 2.0),
                var_or(v, "Sclera UV Radius", 0.93),
            ),
            // a multiplier: CC5's default (0.5273, ...) is the 0..255 value / 255, not decoded
            corner: Vec3::new(
                var_at(v, "Eye Corner Darkness Color", 0, 134.5) / 255.0,
                var_at(v, "Eye Corner Darkness Color", 1, 121.5) / 255.0,
                var_at(v, "Eye Corner Darkness Color", 2, 109.5) / 255.0,
            )
            .extend(var_or(v, "Shadow Radius", 0.57)),
            spec: Vec4::new(
                var_or(v, "Specular Scale", 0.8),
                var_or(v, "_Iris Roughness", 0.05),
                var_or(v, "Sclera Roughness", 0.04),
                var_or(v, "Shadow Hardness", 0.3),
            ),
            present,
            srgb,
            _pad0: 0,
            _pad1: 0,
        },
        sclera,
        iris_normal,
        sclera_normal,
        iris_mask,
    })
}

fn eye_overlay_extension(info: &Cc5Info) -> Cc5EyeOverlay {
    let v = &info.vars;
    // A gradient band: (range min, range max, contrast, edge fadeout contrast).
    let band = |name: &str, range: &str| {
        let r = range_var(v, &format!("{name} {range}"));
        Vec4::new(r.x, r.y, var(v, &format!("{name} Contrast")), var(v, &format!("{name} Edge Fadeout Contrast")))
    };
    let tear = info.shader.starts_with("RLEyeTearline");
    Cc5EyeOverlay {
        params: Cc5EyeOverlayParams {
            kind: Vec4::new(if tear { 1.0 } else { 0.0 }, var_or(v, "OpacityForBlink", 1.0), 0.0, 0.0),
            shadow_color: color_var(v, "Shadow Color", [0.5; 3]).extend(var_or(v, "Shadow Strength", 0.312)),
            shadow_top: band("Shadow Top", "Range"),
            shadow_bottom: band("Shadow Bottom", "Range"),
            shadow_inner: band("Shadow Inner", "Range"),
            shadow_outer: band("Shadow Outer", "Range"),
            blur_color: color_var(v, "Blur Color", [1.0, 0.9, 0.9]).extend(var_or(v, "Blur Strength", 0.219)),
            blur_top: Vec4::new(
                range_var(v, "Top Blur Range").x,
                range_var(v, "Top Blur Range").y,
                var(v, "Top Blur Contrast"),
                var(v, "Top Blur Edge Fadeout Contrast"),
            ),
            blur_bottom: Vec4::new(
                range_var(v, "Bottom Blur Range").x,
                range_var(v, "Bottom Blur Range").y,
                var(v, "Bottom Blur Contrast"),
                var(v, "Bottom Blur Edge Fadeout Contrast"),
            ),
            blur_inner: Vec4::new(
                range_var(v, "Inner Blur Duct Range").x,
                range_var(v, "Inner Blur Duct Range").y,
                var(v, "Inner Blur Contrast"),
                var(v, "Inner Blur Edge Fadeout Contrast"),
            ),
            blur_outer: Vec4::new(
                range_var(v, "Outer Blur Range").x,
                range_var(v, "Outer Blur Range").y,
                var(v, "Outer Blur Contrast"),
                var(v, "Outer Blur Edge Fadeout Contrast"),
            ),
            // x: roughness, y: opacity, z: edge fadeout, w: detail amount
            tear: Vec4::new(
                var_or(v, "_Roughness", 0.25),
                var_or(v, "_Opacity", 0.04),
                var_or(v, "Edge Fadeout", 1.25),
                var_or(v, "Detail Amount", 0.25),
            ),
        },
    }
}

fn mouth_extension(info: &Cc5Info, images: &Assets<Image>, assets: &AssetServer) -> Option<Cc5Mouth> {
    let v = &info.vars;
    let slots = ["MicroNormal", "Gradient AO", "Gums Mask"];
    let (tex, present, srgb) = resolve_maps(info, images, assets, slots)?;
    let teeth = info.shader == "RLTeethGum";
    let [micro_normal, gradient_ao, gums_mask] = tex;
    let params = if teeth {
        Cc5MouthParams {
            // x: brightness, y: desaturation, z: scatter, w: micro normal strength
            teeth: Vec4::new(
                var_or(v, "Teeth Brightness", 1.0),
                var_or(v, "Teeth Desaturation", 0.3),
                var_or(v, "Teeth Scatter", 0.5),
                var_or(v, "Teeth MicroNormal Strength", 0.3),
            ),
            gums: Vec4::new(
                var_or(v, "Gums Brightness", 0.9),
                var(v, "Gums Desaturation"),
                var_or(v, "Gums Scatter", 1.0),
                var_or(v, "Gums MicroNormal Strength", 0.3),
            ),
            // x: roughness, y: specular, z: AO
            front: Vec4::new(var_or(v, "Front Roughness", 0.4), var_or(v, "Front Specular", 0.1), var_or(v, "Front AO", 1.0), 0.0),
            back: Vec4::new(var(v, "Back Roughness"), var(v, "Back Specular"), var(v, "Back AO"), 0.0),
            // x: teeth tiling, y: gums tiling, z: flip y
            micro: Vec4::new(
                var_or(v, "Teeth MicroNormal Tiling", 10.0),
                var_or(v, "Gums MicroNormal Tiling", 10.0),
                var_or(v, "Flip MicroNormal Y", 1.0),
                0.0,
            ),
            edge_color: color_var(v, "Teeth Edge Color", [0.3137, 0.1176, 0.0784]).extend(1.0),
            present,
            srgb,
            _pad0: 0,
            _pad1: 0,
        }
    } else {
        let tongue = Vec4::new(
            var_or(v, "_Brightness", 1.0),
            var_or(v, "_Desaturation", 0.05),
            var_or(v, "_Scatter", 1.0),
            var_or(v, "MicroNormal Strength", 0.5),
        );
        Cc5MouthParams {
            teeth: tongue,
            gums: tongue,
            front: Vec4::new(var_or(v, "Front Roughness", 1.0), var_or(v, "Front Specular", 0.259), var_or(v, "Front AO", 1.0), 0.0),
            back: Vec4::new(var_or(v, "Back Roughness", 1.0), var(v, "Back Specular"), var(v, "Back AO"), 0.0),
            micro: Vec4::new(
                var_or(v, "MicroNormal Tiling", 4.0),
                var_or(v, "MicroNormal Tiling", 4.0),
                var_or(v, "Flip MicroNormal Y", 1.0),
                0.0,
            ),
            edge_color: Vec4::ZERO,
            present,
            srgb,
            _pad0: 0,
            _pad1: 0,
        }
    };
    Some(Cc5Mouth { params, micro_normal, gradient_ao, gums_mask })
}

/// For meshes built in memory (no glTF scene): the asset path their
/// `extras.cc5` map paths are relative to, e.g. `<hair lib>/<part>.glb`.
#[derive(Component, Clone)]
pub struct Cc5MapBase(pub AssetPath<'static>);

/// The glTF file a scene entity was spawned from (its nearest `SceneRoot`
/// ancestor), or the nearest [`Cc5MapBase`].
fn gltf_path(mut e: Entity, parents: &Query<&Parent>, roots: &Query<&SceneRoot>, bases: &Query<&Cc5MapBase>) -> Option<AssetPath<'static>> {
    loop {
        if let Ok(base) = bases.get(e) {
            return Some(base.0.clone());
        }
        if let Ok(root) = roots.get(e) {
            return root.0.path().map(|p| p.without_label().into_owned());
        }
        e = parents.get(e).ok()?.get();
    }
}

/// Per StandardMaterial: not a supported CC5 material, waiting for its maps
/// (the handles keep them loading), or built.
enum Built {
    No,
    Pending(Cc5Info),
    Done(String, Cc5Handle),
}

const SHADERS: [&str; 10] = [
    "RLHead",
    "RLSkin",
    "RLHair",
    "RLEye",
    "RLEyeOcclusion",
    "RLEyeOcclusion_Plus",
    "RLEyeTearline",
    "RLEyeTearline_Plus",
    "RLTeethGum",
    "RLTongue",
];

fn parse_cc5(
    extras: &GltfMaterialExtras,
    name: Option<&GltfMaterialName>,
    path: &AssetPath<'static>,
    assets: &AssetServer,
) -> Option<Cc5Info> {
    let json = serde_json::from_str::<Value>(&extras.value).ok()?;
    let cc5 = json.get("cc5")?;
    let shader = cc5.get("shader").and_then(Value::as_str).unwrap_or_default().to_string();
    if !SHADERS.contains(&shader.as_str()) {
        return None;
    }
    let srgb: Vec<String> = cc5
        .get("srgb")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    let maps = cc5
        .get("maps")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(slot, file)| {
                    // a path relative to the GLB (texture indices from --embed are skipped)
                    let file = path.resolve_embed(file.as_str()?).ok()?;
                    let is_srgb = srgb.contains(slot);
                    let h = assets.load_with_settings(file, move |s: &mut ImageLoaderSettings| {
                        s.is_srgb = is_srgb;
                        s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                            address_mode_u: ImageAddressMode::Repeat,
                            address_mode_v: ImageAddressMode::Repeat,
                            ..ImageSamplerDescriptor::linear()
                        });
                    });
                    Some((slot.clone(), h))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Cc5Info {
        shader,
        name: name.map(|n| n.0.to_lowercase()).unwrap_or_default(),
        vars: cc5.get("vars").cloned().unwrap_or(Value::Null),
        sss: cc5.get("sss").cloned().unwrap_or(Value::Null),
        maps,
    })
}

/// Material assets of every CC5 material type.
#[derive(bevy::ecs::system::SystemParam)]
struct Cc5Assets<'w> {
    skins: ResMut<'w, Assets<Cc5SkinMaterial>>,
    hair: ResMut<'w, Assets<Cc5HairMaterial>>,
    eyes: ResMut<'w, Assets<Cc5EyeMaterial>>,
    overlays: ResMut<'w, Assets<Cc5EyeOverlayMaterial>>,
    mouths: ResMut<'w, Assets<Cc5MouthMaterial>>,
}

/// The CC5 material for `info` on top of a copy of its StandardMaterial, or
/// `None` while its maps are loading.
fn build_material(
    info: &Cc5Info,
    base: &StandardMaterial,
    images: &Assets<Image>,
    assets: &AssetServer,
    settings: &Cc5Materials,
    out: &mut Cc5Assets,
) -> Option<Cc5Handle> {
    let mut base = base.clone();
    let handle = match info.shader.as_str() {
        "RLHead" | "RLSkin" => {
            let ext = skin_extension(info, images, assets, settings.sss_strength)?;
            info!("cc5 materials: {}: maps {:#b}, srgb-decoded {:#b}", info.shader, ext.params.present, ext.params.srgb);
            Cc5Handle::Skin(out.skins.add(ExtendedMaterial { base, extension: ext }))
        }
        "RLHair" => {
            let ext = hair_extension(info, images, assets)?;
            // Opaque pass, alpha-tested at 0.5: hair.wgsl / hair_prepass.wgsl turn the
            // opacity into keep / drop themselves (dithered for scalp hair).
            if matches!(base.alpha_mode, AlphaMode::Mask(_) | AlphaMode::Blend | AlphaMode::AlphaToCoverage) {
                base.alpha_mode = AlphaMode::Mask(0.5);
            }
            // brows / lashes use no diffuse map: hair.wgsl colours them from the root/tip colours
            base.base_color = Color::WHITE;
            base.reflectance = 0.0;
            info!("cc5 materials: {}: maps {:#b}", info.shader, ext.params.present);
            Cc5Handle::Hair(out.hair.add(ExtendedMaterial { base, extension: ext }))
        }
        "RLEye" => {
            let ext = eye_extension(info, images, assets)?;
            // The cornea draws the whole eye (sclera, refracted iris, wet highlight)
            // opaquely; the eyeball it sits on (coincident at the sclera) is hidden.
            base.base_color = Color::WHITE;
            base.alpha_mode = AlphaMode::Opaque;
            info!("cc5 materials: {}: maps {:#b}, srgb-decoded {:#b}", info.shader, ext.params.present, ext.params.srgb);
            Cc5Handle::Eye(out.eyes.add(ExtendedMaterial { base, extension: ext }))
        }
        "RLEyeOcclusion" | "RLEyeOcclusion_Plus" | "RLEyeTearline" | "RLEyeTearline_Plus" => {
            let ext = eye_overlay_extension(info);
            // eye_overlay.wgsl writes premultiplied colour: darkening, tint and highlights
            base.alpha_mode = AlphaMode::Premultiplied;
            base.base_color = Color::WHITE;
            base.unlit = false;
            Cc5Handle::EyeOverlay(out.overlays.add(ExtendedMaterial { base, extension: ext }))
        }
        "RLTeethGum" | "RLTongue" => {
            let ext = mouth_extension(info, images, assets)?;
            Cc5Handle::Mouth(out.mouths.add(ExtendedMaterial { base, extension: ext }))
        }
        _ => unreachable!("parse_cc5 filters shaders"),
    };
    Some(handle)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn build_cc5_materials(
    mut commands: Commands,
    prims: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>, &GltfMaterialExtras, Option<&GltfMaterialName>),
        Without<Cc5Material>,
    >,
    parents: Query<&Parent>,
    children: Query<&Children>,
    names: Query<&GltfMaterialName>,
    roots: Query<&SceneRoot>,
    bases: Query<&Cc5MapBase>,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    standard: Res<Assets<StandardMaterial>>,
    mut out: Cc5Assets,
    settings: Res<Cc5Materials>,
    mut cache: Local<HashMap<AssetId<StandardMaterial>, Built>>,
) {
    for (e, mat, extras, name) in &prims {
        let id = mat.0.id();
        if !cache.contains_key(&id) {
            let Some(path) = gltf_path(e, &parents, &roots, &bases) else { continue };
            let built = parse_cc5(extras, name, &path, &assets).map_or(Built::No, Built::Pending);
            cache.insert(id, built);
        }
        let entry = cache.get_mut(&id).unwrap();
        if let Built::Pending(info) = entry {
            let Some(base) = standard.get(id) else { continue };
            let Some(handle) = build_material(info, base, &images, &assets, &settings, &mut out) else {
                continue; // maps still loading
            };
            *entry = Built::Done(info.shader.clone(), handle);
        }
        if let Built::Done(shader, cc5) = entry {
            // RLEye: the eyeball primitives next to the cornea (CC's Std_Eye_L / Std_Eye_R)
            let hides = match (shader.as_str(), parents.get(e)) {
                ("RLEye", Ok(parent)) => children
                    .get(parent.get())
                    .map(|c| {
                        c.iter()
                            .copied()
                            .filter(|&s| {
                                names.get(s).is_ok_and(|n| {
                                    let n = n.0.to_lowercase();
                                    n.starts_with("std_eye_") && !n.contains("occlusion")
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            commands.entity(e).insert(Cc5Material {
                shader: shader.clone(),
                standard: mat.0.clone(),
                cc5: cc5.clone(),
                hides,
            });
        }
    }
}

fn sync_cc5_materials(
    mut commands: Commands,
    settings: Res<Cc5Materials>,
    prims: Query<(Entity, &Cc5Material, Has<MeshMaterial3d<StandardMaterial>>)>,
    added: Query<(), Added<Cc5Material>>,
) {
    if !settings.is_changed() && added.is_empty() {
        return;
    }
    for (e, m, has_standard) in &prims {
        let hidden = if settings.enabled { Visibility::Hidden } else { Visibility::Inherited };
        for &h in &m.hides {
            if let Some(mut ec) = commands.get_entity(h) {
                ec.insert(hidden);
            }
        }
        let mut ec = commands.entity(e);
        if settings.enabled {
            ec.remove::<MeshMaterial3d<StandardMaterial>>();
            m.cc5.insert(&mut ec);
        } else {
            Cc5Handle::remove_all(&mut ec);
            if !has_standard {
                ec.insert(MeshMaterial3d(m.standard.clone()));
            }
        }
    }
}
