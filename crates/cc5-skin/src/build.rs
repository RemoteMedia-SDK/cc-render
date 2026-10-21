//! `cc5-skin build`: a [`SkinPack`] from a CC5 capture (`cc5_skin_capture.py` with
//! `"export": true`: per variant `tex/<material>_<Channel>.png` as CC5 exported
//! them, and `character.json`).
//!
//! Layers: variant `X` is the layer on the character's own skin (`base`), `X@male`
//! the same layer on a second skin base (`skinbase_male_clean`). Per texel and
//! channel CC5 gives L = B·(1−α) + C·α on both bases, so
//! (1−α) = (L1−L2)/(B1−B2) (least squares over the channels, weighted by how much
//! the bases differ) and C = (L1 − B1·(1−α)) / α.
use crate::{Layer, LayerMap, Skin, SkinPack};
use anyhow::{Context, Result};
use image::RgbaImage;
use rayon::prelude::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct BuildOptions {
    pub capture: PathBuf,
    pub out: PathBuf,
    pub base: String,
    pub alt: String,
    pub materials: Vec<String>,
    /// texels of opacity below this (0..1) are dropped
    pub min_alpha: f32,
}

/// Solve result for one layer on one material.
pub struct Solved {
    pub w: u32,
    pub h: u32,
    /// sRGB colour 0..1 and opacity per texel
    pub c: Vec<[f32; 3]>,
    pub a: Vec<f32>,
    /// how well the solve predicts the second base: mean |L2 − pred| (0..255)
    pub err_alt: f32,
    /// mean |L1 − B1| over the layer's texels (0..255): how visible it is
    pub size: f32,
}

fn rgb(path: &Path) -> Result<(u32, u32, Vec<[f32; 3]>)> {
    let img = image::open(path).with_context(|| format!("{}", path.display()))?.to_rgb8();
    let (w, h) = img.dimensions();
    Ok((w, h, img.pixels().map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect()))
}

/// The diffuse map CC5 exported for `material` in a variant.
pub fn diffuse_path(capture: &Path, variant: &str, material: &str) -> Option<PathBuf> {
    let dir = capture.join(variant).join("tex");
    ["Diffuse", "diffuse", "BaseColor"]
        .iter()
        .flat_map(|c| ["png", "jpg"].map(|e| dir.join(format!("{material}_{c}.{e}"))))
        .find(|p| p.is_file())
}

pub fn solve(b1: &[[f32; 3]], l1: &[[f32; 3]], b2: &[[f32; 3]], l2: &[[f32; 3]], w: u32, h: u32) -> Solved {
    let n = b1.len();
    let res: Vec<([f32; 3], f32, f32, f32)> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (b1, l1, b2, l2) = (b1[i], l1[i], b2[i], l2[i]);
            let (mut num, mut den, mut d1) = (0.0, 0.0, 0.0f32);
            for c in 0..3 {
                let db = b1[c] - b2[c];
                num += (l1[c] - l2[c]) * db;
                den += db * db;
                d1 = d1.max((l1[c] - b1[c]).abs());
            }
            if d1 < 1.5 / 255.0 {
                // the layer leaves this texel as it is
                return ([0.0; 3], 0.0, 0.0, 0.0);
            }
            // 1 - alpha, regularized towards "fully covered" where the bases agree
            // (then only an opaque layer explains a change)
            let k = 1e-4;
            let keep = (num / (den + k)).clamp(0.0, 1.0);
            let a = 1.0 - keep;
            if a < 1e-3 {
                return ([0.0; 3], 0.0, 0.0, d1);
            }
            let col = [0, 1, 2].map(|c| ((l1[c] - b1[c] * keep) / a).clamp(0.0, 1.0));
            let pred = [0, 1, 2].map(|c| b2[c] * keep + col[c] * a);
            let err = (0..3).map(|c| (pred[c] - l2[c]).abs()).sum::<f32>() / 3.0;
            (col, a, err, d1)
        })
        .collect();
    let touched: Vec<&([f32; 3], f32, f32, f32)> = res.iter().filter(|r| r.3 > 0.0).collect();
    let m = touched.len().max(1) as f32;
    Solved {
        w,
        h,
        err_alt: touched.iter().map(|r| r.2).sum::<f32>() / m * 255.0,
        size: touched.iter().map(|r| r.3).sum::<f32>() / m * 255.0,
        c: res.iter().map(|r| r.0).collect(),
        a: res.iter().map(|r| r.1).collect(),
    }
}

/// Crop to where the layer shows; RGBA8 + rect.
fn crop(s: &Solved, min_alpha: f32) -> Option<(RgbaImage, [u32; 4], [f32; 3])> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    let mut mean = [0.0f64; 3];
    let mut aw = 0.0f64;
    for y in 0..s.h {
        for x in 0..s.w {
            let i = (y * s.w + x) as usize;
            let a = s.a[i];
            if a >= min_alpha {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
                for c in 0..3 {
                    mean[c] += (s.c[i][c] * a) as f64;
                }
                aw += a as f64;
            }
        }
    }
    if x1 <= x0 {
        return None;
    }
    // one texel of margin for bilinear sampling
    let (x0, y0, x1, y1) = (x0.saturating_sub(1), y0.saturating_sub(1), (x1 + 1).min(s.w), (y1 + 1).min(s.h));
    let img = RgbaImage::from_fn(x1 - x0, y1 - y0, |x, y| {
        let i = ((y + y0) * s.w + x + x0) as usize;
        let a = if s.a[i] >= min_alpha { s.a[i] } else { 0.0 };
        let c = s.c[i];
        image::Rgba([(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8, (a * 255.0).round() as u8])
    });
    let mean = [0, 1, 2].map(|c| (mean[c] / aw.max(1e-9) * 255.0) as f32);
    Some((img, [x0, y0, x1 - x0, y1 - y0], mean))
}

/// CC5's shader variables per material from an export's JSON.
fn export_vars(json: &Path, materials: &[String]) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Ok(v) = std::fs::read(json).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice::<Value>(&b)?)) else { return out };
    fn walk(v: &Value, materials: &[String], out: &mut BTreeMap<String, Value>) {
        if let Some(o) = v.as_object() {
            for (k, x) in o {
                if materials.contains(k) {
                    if let Some(vars) = x.pointer("/Custom Shader/Variable") {
                        out.insert(k.clone(), vars.clone());
                    }
                } else {
                    walk(x, materials, out);
                }
            }
        }
    }
    walk(&v, materials, &mut out);
    out
}

/// The exported channels a skin swaps (others stay the character's).
const SKIN_CHANNELS: [&str; 8] = ["Diffuse", "Normal", "roughness", "ao", "SSSMap", "TransMap", "MicroNMask", "SpecMask"];

/// Builds the pack: `layers` (variant names; `X@male` = X on the alt base) and
/// `skins` (variant names with whole skins), with display names and groups.
pub fn build(opt: &BuildOptions, layers: &[(String, String, String)], skins: &[(String, String)]) -> Result<SkinPack> {
    std::fs::create_dir_all(opt.out.join("layers"))?;
    let mut pack = SkinPack { format: "cc5-skin-pack".into(), version: 1, size: 0, layers: Vec::new(), skins: Vec::new(), dir: opt.out.clone() };
    for (id, name, group) in layers {
        let mut maps = BTreeMap::new();
        let mut color = [0.0f32; 3];
        let mut best = 0.0f32;
        for m in &opt.materials {
            let paths = [
                diffuse_path(&opt.capture, &opt.base, m),
                diffuse_path(&opt.capture, id, m),
                diffuse_path(&opt.capture, &opt.alt, m),
                diffuse_path(&opt.capture, &format!("{id}@male"), m),
            ];
            let [Some(pb1), Some(pl1), Some(pb2), Some(pl2)] = paths else {
                eprintln!("{id} {m}: missing exports {:?}", paths.iter().map(Option::is_some).collect::<Vec<_>>());
                continue;
            };
            let (w, h, b1) = rgb(&pb1)?;
            let (_, _, l1) = rgb(&pl1)?;
            let (_, _, b2) = rgb(&pb2)?;
            let (_, _, l2) = rgb(&pl2)?;
            anyhow::ensure!([l1.len(), b2.len(), l2.len()].iter().all(|&n| n == b1.len()), "{id} {m}: texture sizes differ");
            pack.size = pack.size.max(w);
            let s = solve(&b1, &l1, &b2, &l2, w, h);
            let Some((img, rect, mean)) = crop(&s, opt.min_alpha) else { continue };
            let file = format!("layers/{id}.{m}.png");
            img.save(opt.out.join(&file))?;
            let cover = img.pixels().filter(|p| p[3] > 0).count() as f32;
            println!("{id:24} {m:14} rect {rect:?} size {:.1} alt err {:.2} ({cover} texels)", s.size, s.err_alt);
            if cover > best {
                best = cover;
                color = mean;
            }
            maps.insert(m.clone(), LayerMap { diffuse: file, rect });
        }
        if !maps.is_empty() {
            pack.layers.push(Layer { id: id.clone(), name: name.clone(), group: group.clone(), color, maps, source: String::new() });
        }
    }
    for (id, name) in skins {
        let src = opt.capture.join(id).join("tex");
        let dst = opt.out.join("skins").join(id);
        std::fs::create_dir_all(&dst)?;
        let mut maps: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for e in std::fs::read_dir(&src).with_context(|| format!("{}", src.display()))? {
            let p = e?.path();
            let f = p.file_name().unwrap().to_string_lossy().to_string();
            let Some(m) = opt.materials.iter().find(|m| f.starts_with(&format!("{m}_"))) else { continue };
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let channel = stem[m.len() + 1..].to_string();
            if !SKIN_CHANNELS.contains(&channel.as_str()) {
                continue;
            }
            // the same as the character's: nothing to swap
            let same = std::fs::read(opt.capture.join(&opt.base).join("tex").join(&f)).ok() == Some(std::fs::read(&p)?);
            if same {
                continue;
            }
            if channel == "roughness" {
                // as a glTF metallic-roughness map (roughness in G, metal 0) for StandardMaterial
                let r = image::open(&p)?.to_luma8();
                let mr = image::RgbImage::from_fn(r.width(), r.height(), |x, y| image::Rgb([0, r.get_pixel(x, y)[0], 0]));
                let out = format!("{stem}_MR.png");
                mr.save(dst.join(&out))?;
                maps.entry(m.clone()).or_default().insert("MetallicRoughness".into(), format!("skins/{id}/{out}"));
                continue;
            }
            // colour and normal maps as JPEG (q 92), the rest as they are
            let big = ["Diffuse", "Normal"].contains(&channel.as_str());
            let out = if big { format!("{stem}.jpg") } else { f.clone() };
            if big {
                let img = image::open(&p)?.to_rgb8();
                let mut w = std::io::BufWriter::new(std::fs::File::create(dst.join(&out))?);
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 92).encode_image(&img)?;
            } else {
                std::fs::copy(&p, dst.join(&out))?;
            }
            maps.entry(m.clone()).or_default().insert(channel, format!("skins/{id}/{out}"));
        }
        let vars = export_vars(&opt.capture.join(id).join("character.json"), &opt.materials);
        println!("skin {id}: {} materials, vars for {}", maps.len(), vars.len());
        pack.skins.push(Skin { id: id.clone(), name: name.clone(), maps, vars, source: String::new() });
    }
    pack.save()?;
    Ok(pack)
}
