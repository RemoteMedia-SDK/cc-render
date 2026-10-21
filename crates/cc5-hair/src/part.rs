//! Build one library part from a spec (hair_build.py `build`, "convert" path):
//! strands for the source's hair cards, CC5's own cards for `keep_cards`
//! layers (brows) and the source's scalp layers, materials with `extras.cc5`,
//! the catalog entry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use glam::{Vec2, Vec3};
use serde_json::{json, Map, Value};

use crate::cards::{cards_to_strands, Tex};
use crate::follow;
use crate::head::{Body, Surface};
use crate::settings::{Convert, Spec};
use crate::source::Source;
use crate::{shade, shape, Strand};

/// A built part: meshes, materials and the catalog entry.
pub struct BuiltPart {
    pub id: String,
    pub meshes: Vec<PartMesh>,
    pub materials: Vec<PartMaterial>,
    pub entry: Value,
    /// CC5 map files the part's materials use ({lib-relative path: source file}).
    pub files: Vec<(String, PathBuf)>,
}

pub struct PartMesh {
    pub name: String,
    pub material: usize,
    pub geom: Geometry,
}

pub enum Geometry {
    /// Strands; skin: each strand rides the body's weights at its root.
    Strands { strands: Vec<Strand>, skin: Vec<[(u16, f32); 4]>, taper: f32, root_fade: f32 },
    /// CC5's own cards (kept layers): per-vertex skin from the body.
    Cards { pos: Vec<Vec3>, nrm: Vec<Vec3>, uv: Vec<Vec2>, tris: Vec<[u32; 3]>, skin: Vec<[(u16, f32); 4]> },
}

pub struct PartMaterial {
    pub name: String,
    /// Alpha-blended soft layer on the skin (scalp, brow base), else alpha-tested hair.
    pub blend: bool,
    pub cc5: Option<Value>,
    pub base_factor: [f32; 4],
    /// Encoded base colour image (bytes, MIME type).
    pub base_image: Option<(Vec<u8>, String)>,
}

/// Hair, beard, brow ... materials: CC5 names its hair materials after them.
fn is_hair_material(name: &str) -> bool {
    let n = name.to_lowercase();
    ["hair", "beard", "brow", "mustache", "moustache", "stubble", "goatee", "sideburn", "bang"].iter().any(|w| n.contains(w))
}

/// The material holding the hair cards: the hair-named one with most triangles.
fn main_material(src: &Source, obj: &crate::source::Object) -> Option<usize> {
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for p in &obj.prims {
        if let Some(m) = p.material {
            *counts.entry(m).or_default() += p.tris.len();
        }
    }
    counts
        .into_iter()
        .filter(|(m, _)| {
            let n = src.materials[*m].name.to_lowercase();
            is_hair_material(&n) && !["scalp", "base_", "color_", "clap", "cap"].iter().any(|w| n.contains(w))
        })
        .max_by_key(|(m, c)| (*c, usize::MAX - *m))
        .map(|(m, _)| m)
}

/// Area-weighted vertex normals.
fn smooth_normals(pos: &[Vec3], tris: &[[u32; 3]]) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; pos.len()];
    for t in tris {
        let [a, b, c] = t.map(|i| pos[i as usize]);
        let f = (b - a).cross(c - a);
        for &i in t {
            n[i as usize] += f;
        }
    }
    n.into_iter().map(|v| v.normalize_or(Vec3::Y)).collect()
}

fn base_name(n: &str) -> &str {
    n.split('.').next().unwrap_or(n)
}

/// RLHair maps for strand meshes (UV = (ID, t)): root map = v, ID map = u, a
/// flat flow map along +v. Written once into `lib/atlas/strandmaps/`.
pub fn strand_maps(lib: &Path) -> Result<Value> {
    let d = lib.join("atlas").join("strandmaps");
    std::fs::create_dir_all(&d)?;
    let root = d.join("root.png");
    if !root.exists() {
        image::RgbImage::from_fn(4, 256, |_, y| image::Rgb([y as u8; 3])).save(&root)?;
        image::RgbImage::from_fn(256, 4, |x, _| image::Rgb([x as u8; 3])).save(d.join("id.png"))?;
        image::RgbImage::from_fn(4, 4, |_, _| image::Rgb([128, 0, 128])).save(d.join("flow.png"))?;
    }
    Ok(json!({"Hair Root Map": "atlas/strandmaps/root.png", "Hair ID Map": "atlas/strandmaps/id.png", "Hair Flow Map": "atlas/strandmaps/flow.png"}))
}

/// Linear RGB a white strand mask is tinted to without CC5's shader (mean of
/// root and tip colour), or None for coloured maps.
fn mask_tint(vars: &Map<String, Value>) -> Option<[f32; 3]> {
    let on = vars.get("UseRootTipColor").map_or(false, |v| v.as_f64().map_or(v.as_bool().unwrap_or(false), |x| x != 0.0));
    let mode = vars.get("RootTipBlendMode").and_then(|v| v.as_f64());
    if !on || !matches!(mode, Some(m) if m == 1.0 || m == 2.0) {
        return None;
    }
    let col = |k: &str| -> [f64; 3] {
        let a = vars.get(k).and_then(|v| v.as_array());
        std::array::from_fn(|i| a.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).unwrap_or(255.0))
    };
    let (rc, tc) = (col("RootColor"), col("TipColor"));
    Some(std::array::from_fn(|i| (crate::srgb_to_linear((rc[i] / 255.0) as f32) + crate::srgb_to_linear((tc[i] / 255.0) as f32)) / 2.0))
}

fn merge_vars(base: Option<&Value>, over: &Map<String, Value>) -> Value {
    let mut v = base.and_then(|v| v.as_object()).cloned().unwrap_or_default();
    for (k, x) in over {
        v.insert(k.clone(), x.clone());
    }
    Value::Object(v)
}

/// Build `spec` with `head` (the reference character: body, skeleton) into a
/// part for library `lib` (its `atlas/<spec.atlas>/atlas.json` gives the
/// default RLHair variables).
pub fn build(spec: &Spec, head: &Source, body: &Body, lib: &Path) -> Result<BuiltPart> {
    build_with(spec, head, body, lib, None)
}

/// [`build`] with the group's source already loaded (`group`: the spec's
/// `source_glb`; its decoded images are reused across rebuilds).
pub fn build_with(spec: &Spec, head: &Source, body: &Body, lib: &Path, group: Option<&Source>) -> Result<BuiltPart> {
    let base: Convert = Convert::for_source(&spec.convert, "")?;
    // the source: another export of the character (a Smart Hair group), or the head itself
    let owned: Option<Source> = match (&base.source_glb, group) {
        (Some(sg), None) => Some(Source::load(sg)?),
        _ => None,
    };
    let src: &Source = match (&base.source_glb, group) {
        (Some(_), Some(g)) => g,
        (Some(_), None) => owned.as_ref().unwrap(),
        (None, _) => head,
    };
    let sources: Vec<String> = if base.source_glb.is_some() {
        let mut names: Vec<String> = src
            .objects
            .iter()
            .filter(|o| !o.name.starts_with("CC_Base"))
            .filter(|o| o.prims.iter().any(|p| p.material.is_some_and(|m| is_hair_material(&src.materials[m].name))))
            .filter(|o| !base.exclude.iter().any(|x| x == base_name(&o.name)))
            .filter(|o| base.include.as_ref().map_or(true, |inc| o.name.to_lowercase().contains(&inc.to_lowercase())))
            .map(|o| o.name.clone())
            .collect();
        names.sort();
        names
    } else {
        base.sources.clone()
    };
    let from_group = base.source_glb.is_some();
    // the CC5 material settings: the source GLB's own, or the character's
    // (the head's: `../character.glb`, else `<head>.cc5mat.glb` beside it)
    let mat_glb: Option<PathBuf> = base.material_glb.clone().or_else(|| base.source_glb.clone()).or_else(|| {
        let char_glb = head.path.parent()?.join("..").join("character.glb");
        let cc5mat = head.path.with_extension("cc5mat.glb");
        [char_glb, cc5mat].into_iter().find(|p| p.exists())
    });
    let mat_src_owned;
    let mat_src: Option<&Source> = match &mat_glb {
        Some(p) if *p == src.path => Some(src),
        Some(p) => {
            mat_src_owned = Source::load(p)?;
            Some(&mat_src_owned)
        }
        None => None,
    };
    let cc5: HashMap<String, Value> = mat_src
        .map(|m| m.materials.iter().filter_map(|x| x.cc5.clone().map(|c| (base_name(&x.name).to_string(), c))).collect())
        .unwrap_or_default();
    let base_dir = mat_glb.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let atlas: Value = serde_json::from_str(&std::fs::read_to_string(lib.join("atlas").join(&spec.atlas).join("atlas.json")).with_context(|| format!("{}: atlas {}", lib.display(), spec.atlas))?)?;
    let maps = strand_maps(lib)?;
    let default_vars = merge_vars(atlas.get("vars"), &spec.vars);
    let default_cc5 = json!({"shader": "RLHair", "srgb": [], "vars": default_vars, "maps": maps});
    let tint = default_vars.as_object().and_then(mask_tint);
    let factor = |tint: Option<[f32; 3]>| tint.map_or([1.0; 4], |t| [t[0], t[1], t[2], 1.0]);

    let mut meshes = Vec::new();
    let mut materials: Vec<PartMaterial> = Vec::new();
    let mut files = Vec::new();
    let (mut cards, mut verts) = (0usize, 0usize);
    let mut seed = spec.seed;
    for name in &sources {
        let c = Convert::for_source(&spec.convert, name)?;
        let obj = src.object(name).ok_or_else(|| anyhow!("{}: no object {name}", src.path.display()))?;
        let main = main_material(src, obj);
        let sname = base_name(name);
        seed = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(sname.bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64)));
        if !c.keep_cards {
            let Some(main) = main else { bail!("{name}: no hair material") };
            let mname = base_name(&src.materials[main].name).to_string();
            // the hair material's triangles (one or more primitives)
            let (mut pos, mut uv, mut tris) = (Vec::new(), Vec::new(), Vec::new());
            let (mut spos, mut stris) = (Vec::new(), Vec::new());
            for p in &obj.prims {
                if p.material == Some(main) {
                    let o = pos.len() as u32;
                    pos.extend(&p.pos);
                    uv.extend(&p.uv);
                    tris.extend(p.tris.iter().map(|t| t.map(|i| i + o)));
                } else {
                    let o = spos.len() as u32;
                    spos.extend(&p.pos);
                    stris.extend(p.tris.iter().map(|t| t.map(|i| i + o)));
                }
            }
            let img = match src.materials[main].base_image {
                Some(i) => Some(Tex::new(src.image_rgba(i)?)),
                None => None,
            };
            let flow = cc5
                .get(&mname)
                .and_then(|v| v.get("maps")?.get("Hair Flow Map")?.as_str().map(|f| base_dir.join(f)))
                .filter(|p| p.exists())
                .map(|p| src.file_rgba(&p).map(Tex::new))
                .transpose()?;
            let scalp = Surface::new(spos, stris);
            let mut strands = cards_to_strands(&pos, &uv, &tris, img.as_ref(), flow.as_ref(), &c, body, seed);
            shape::finish(&mut strands, &c.shape, body, scalp.as_ref(), seed);
            if let Some([ao_min, gamma]) = c.depth_shade {
                shade::depth_shade(&mut strands, body, ao_min as f32, gamma as f32);
            }
            let skin = strands.iter().map(|s| body.skin_at(s.pts[0])).collect();
            let mesh_name = format!("{}_{}", spec.id, sname);
            let cc5_mat = match cc5.get(&mname) {
                Some(v) => {
                    let mut v = v.clone();
                    let vars = merge_vars(v.get("vars"), &spec.vars);
                    v["maps"] = maps.clone();
                    v["srgb"] = json!([]);
                    v["vars"] = vars;
                    v
                }
                None => default_cc5.clone(),
            };
            materials.push(PartMaterial { name: format!("{mesh_name}_Hair"), blend: false, cc5: Some(cc5_mat), base_factor: factor(tint), base_image: None });
            cards += strands.len();
            verts += strands.iter().map(|s| 2 * s.pts.len()).sum::<usize>();
            meshes.push(PartMesh { name: mesh_name, material: materials.len() - 1, geom: Geometry::Strands { strands, skin, taper: c.taper as f32, root_fade: c.root_fade as f32 } });
        }
        // kept layers: every layer (keep_cards: brows), or the source's other layers (scalp, caps)
        if c.keep_cards || c.source_scalp {
            for (li, p) in obj.prims.iter().enumerate() {
                let Some(m) = p.material else { continue };
                let mname = base_name(&src.materials[m].name).to_string();
                let is_main = Some(m) == main;
                if !c.keep_cards && (is_main || (!from_group && !mname.to_lowercase().contains("scalp"))) {
                    continue;
                }
                let lname = format!("{}_{}_layer{}", spec.id, sname, li);
                let skin = p.pos.iter().map(|q| body.skin_at(*q)).collect();
                let image = match src.materials[m].base_image {
                    Some(i) => Some(src.image_bytes(i).map(|(b, t)| (b.to_vec(), t))?),
                    None => None,
                };
                let mat = if c.keep_cards && is_main && cc5.contains_key(&mname) {
                    // the hair cards keep CC5's RLHair shader and their own maps
                    let mut v = cc5[&mname].clone();
                    let mut own = Map::new();
                    if let Some(ms) = v.get("maps").and_then(|m| m.as_object()) {
                        for (slot, f) in ms {
                            let Some(f) = f.as_str() else { continue };
                            let file = base_dir.join(f);
                            if file.exists() {
                                let rel = format!("cc5maps/{}/{}", spec.id, Path::new(f).file_name().unwrap().to_string_lossy());
                                files.push((rel.clone(), file));
                                own.insert(slot.clone(), Value::String(rel));
                            }
                        }
                    }
                    v["maps"] = Value::Object(own);
                    v["vars"] = merge_vars(v.get("vars"), &spec.vars);
                    PartMaterial { name: format!("{lname}_Hair"), blend: false, cc5: Some(v), base_factor: factor(tint), base_image: image }
                } else {
                    PartMaterial { name: format!("{lname}_Scalp"), blend: true, cc5: None, base_factor: src.materials[m].base_factor, base_image: image }
                };
                materials.push(mat);
                if c.keep_cards {
                    cards += p.tris.len(); // (strand parts count strands only, as hair_build.py)
                }
                verts += p.pos.len();
                meshes.push(PartMesh {
                    name: lname,
                    material: materials.len() - 1,
                    // smooth normals from the cards' own faces, as hair_build.py's rebuilt
                    // layers had (the hair shader's diffuse reads them)
                    geom: Geometry::Cards { pos: p.pos.clone(), nrm: smooth_normals(&p.pos, &p.tris), uv: p.uv.clone(), tris: p.tris.clone(), skin },
                });
            }
        }
    }
    let mut entry = json!({"id": spec.id, "slot": spec.slot, "kind": spec.kind, "file": format!("{}.glb", spec.id),
                           "atlas": spec.atlas, "cards": cards, "verts": verts});
    if !base.follow.is_empty() || base.shape_dir.is_some() {
        let mut modes = HashMap::new();
        if let Some(d) = &base.shape_dir {
            modes.extend(follow::layer_modes(d, 0.05)?);
            modes.extend(follow::bone_modes(d, 0.1)?);
        }
        modes.extend(base.follow.iter().map(|(k, v)| (follow::key(k), v.clone())));
        let names: Vec<String> = meshes.iter().map(|m| m.name.clone()).collect();
        let f = follow::part_follow(&names, &spec.id, &modes);
        if !f.is_empty() {
            entry["follow"] = serde_json::to_value(f)?;
        }
    }
    if base.keep_cards && spec.kind == "brow" {
        entry["painted_layers"] = json!(true);
    }
    Ok(BuiltPart { id: spec.id.clone(), meshes, materials, entry, files })
}
