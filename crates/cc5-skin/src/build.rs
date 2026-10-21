//! `cc5-skin build`: a [`SkinPack`] from a CC5 capture (`cc5_skin_capture.py` with
//! `"export": true`: per variant `tex/<material>_<Channel>.png` as CC5 exported
//! them, and `character.json`).
//!
//! Layers: the layer on two bases, variant `X` on the character's own skin
//! (`base`) and `X@dark` on the character saved with a contrasting skin (`alt`;
//! CC5's SkinGen bake composites onto its own base skin, so a flat diffuse set
//! in the material never reaches it). Per texel CC5 gives L = blend(B, C, α) on
//! both bases; each [`Blend`] is solved by least squares and the mode with the
//! least error over both bases is kept. Normal: (1−α) = (L1−L2)/(B1−B2) over the
//! channels and C = (L1 − B1·(1−α)) / α; multiply and screen: a per-channel
//! effect (B·M, 1−(1−B)(1−S)), split into the least opacity and a colour.
use crate::{Blend, Layer, LayerMap, Skin, SkinPack};
use anyhow::{Context, Result};
use image::RgbaImage;
use rayon::prelude::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct BuildOptions {
    pub capture: PathBuf,
    pub out: PathBuf,
    /// the two clean skin bases layers are solved on (variants), and the suffixes
    /// of their layer variants (`<layer><suffix>`)
    pub base: String,
    pub alt: String,
    pub base_suffix: String,
    pub alt_suffix: String,
    /// held-out check: the layer laid over this image against CC5's `<layer><suffix>`
    pub check: Option<(PathBuf, String)>,
    /// the second base as an image (the flat diffuse CC5 got in place of every
    /// material's) instead of a variant
    pub alt_image: Option<PathBuf>,
    /// the character's own variant (layers on it validate the solve)
    pub character: String,
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
    pub blend: Blend,
    /// how well the solve explains both bases: mean |L − pred| (0..255)
    pub err_alt: f32,
    /// mean |L1 − B1| over the layer's texels (0..255): how visible it is
    pub size: f32,
}

fn rgb(path: &Path) -> Result<(u32, u32, Vec<[f32; 3]>)> {
    let img = image::open(path).with_context(|| format!("{}", path.display()))?.to_rgb8();
    let (w, h) = img.dimensions();
    Ok((w, h, img.pixels().map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect()))
}

/// The diffuse map CC5 baked (`<variant>/<material>_Diffuse.png`) or exported
/// (`<variant>/tex/`) for `material` in a variant.
pub fn diffuse_path(capture: &Path, variant: &str, material: &str) -> Option<PathBuf> {
    let d = capture.join(variant);
    [d.join(format!("{material}_Diffuse.png")), d.join("tex").join(format!("{material}_Diffuse.png"))].into_iter().find(|p| p.is_file())
}

/// Colour and opacity of one texel of a layer blended by `blend`, from the layer
/// on two bases; None where the layer leaves the texel as it is.
fn solve_texel(blend: Blend, b1: [f32; 3], l1: [f32; 3], b2: [f32; 3], l2: [f32; 3]) -> Option<([f32; 3], f32)> {
    let d1 = (0..3).map(|c| (l1[c] - b1[c]).abs()).fold(0.0f32, f32::max);
    if d1 < 1.5 / 255.0 {
        return None;
    }
    // least squares per channel of an effect `e` linear in a known function of
    // the base: L = B + e·g(B) for multiply (g = B, e = M − 1) and screen
    // (g = 1 − B, e = S), fitted over the two bases
    let fit = |g: fn(f32) -> f32| {
        [0, 1, 2].map(|c| {
            let (g1, g2) = (g(b1[c]), g(b2[c]));
            ((l1[c] - b1[c]) * g1 + (l2[c] - b2[c]) * g2) / (g1 * g1 + g2 * g2 + 1e-6)
        })
    };
    let (col, a) = match blend {
        Blend::Normal => {
            // 1 - alpha, regularized towards "fully covered" where the bases agree
            // (then only an opaque layer explains a change)
            let (mut num, mut den) = (0.0, 0.0);
            for c in 0..3 {
                let db = b1[c] - b2[c];
                num += (l1[c] - l2[c]) * db;
                den += db * db;
            }
            let keep = (num / (den + 1e-4)).clamp(0.0, 1.0);
            let a = 1.0 - keep;
            (a >= 1e-3).then(|| ([0, 1, 2].map(|c| ((l1[c] - b1[c] * keep) / a).clamp(0.0, 1.0)), a))?
        }
        Blend::Multiply => {
            // B·(1 − α + α·C): 1 − α + α·C = 1 + e; least opacity that explains it
            let e = fit(|b| b).map(|e| e.clamp(-1.0, 0.0));
            let a = -e.iter().copied().fold(0.0f32, f32::min);
            (a >= 1e-3).then(|| (e.map(|e| (1.0 + e / a).clamp(0.0, 1.0)), a))?
        }
        Blend::Screen => {
            // 1 − (1−B)(1 − α·C): α·C = e
            let e = fit(|b| 1.0 - b).map(|e| e.clamp(0.0, 1.0));
            let a = e.iter().copied().fold(0.0f32, f32::max);
            (a >= 1e-3).then(|| (e.map(|e| (e / a).clamp(0.0, 1.0)), a))?
        }
    };
    Some((col, a))
}

/// Solves a layer blended by `blend`: colour, opacity and how well that explains
/// both bases.
pub fn solve(blend: Blend, b1: &[[f32; 3]], l1: &[[f32; 3]], b2: &[[f32; 3]], l2: &[[f32; 3]], w: u32, h: u32) -> Solved {
    let n = b1.len();
    let res: Vec<([f32; 3], f32, f32, f32)> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (b1, l1, b2, l2) = (b1[i], l1[i], b2[i], l2[i]);
            let d1 = (0..3).map(|c| (l1[c] - b1[c]).abs()).fold(0.0f32, f32::max);
            let Some((col, a)) = solve_texel(blend, b1, l1, b2, l2) else {
                // untouched (d1 0), or a change no opacity explains (counted)
                let err = if d1 < 1.5 / 255.0 { 0.0 } else { (0..3).map(|c| (l1[c] - b1[c]).abs() + (l2[c] - b2[c]).abs()).sum::<f32>() / 6.0 };
                return ([0.0; 3], 0.0, err, if d1 < 1.5 / 255.0 { 0.0 } else { d1 });
            };
            let err = (0..3).map(|c| (blend.apply(b1[c], col[c], a) - l1[c]).abs() + (blend.apply(b2[c], col[c], a) - l2[c]).abs()).sum::<f32>() / 6.0;
            (col, a, err, d1)
        })
        .collect();
    let touched: Vec<&([f32; 3], f32, f32, f32)> = res.iter().filter(|r| r.3 > 0.0).collect();
    let m = touched.len().max(1) as f32;
    Solved {
        w,
        h,
        blend,
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

fn channel_key(c: &str) -> String {
    if c == "roughness" {
        "MetallicRoughness".into()
    } else {
        c.into()
    }
}

/// Held-out check: the layer laid over `opt.check`'s image ([`crate::compose`])
/// against CC5's texture of the layer on that image: (mean abs error, mean
/// change), 0..255, over the layer's rect; None without that capture.
fn validate(opt: &BuildOptions, id: &str, m: &str, img: &RgbaImage, rect: [u32; 4], blend: Blend, w: u32, h: u32) -> Result<Option<(f32, f32)>> {
    let Some((pb, suffix)) = &opt.check else { return Ok(None) };
    let Some(pl) = diffuse_path(&opt.capture, &format!("{id}{suffix}"), m) else { return Ok(None) };
    let base = image::open(&pb)?.to_rgba8();
    let cc5 = image::open(&pl)?.to_rgba8();
    if base.dimensions() != (w, h) || cc5.dimensions() != (w, h) {
        return Ok(None);
    }
    let li = crate::LayerImage { rect, size: w, w: img.width(), h: img.height(), rgba: img.as_raw().clone(), blend };
    let mut ours = base.as_raw().clone();
    crate::compose(&mut ours, w, h, &[crate::Apply { image: &li, strength: 1.0, tint: None, detail: 0.0, xf: crate::Xf::default() }]);
    let (mut err, mut size, mut n) = (0.0f64, 0.0f64, 0u64);
    for y in rect[1]..rect[1] + rect[3] {
        for x in rect[0]..rect[0] + rect[2] {
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                err += (ours[i + c] as f64 - cc5.as_raw()[i + c] as f64).abs();
                size += (base.as_raw()[i + c] as f64 - cc5.as_raw()[i + c] as f64).abs();
            }
            n += 3;
        }
    }
    Ok(Some(((err / n.max(1) as f64) as f32, (size / n.max(1) as f64) as f32)))
}

/// The layer's roughness: CC5's baked roughness of the layer on the first base
/// against that base's, with the opacity solved from the diffuse:
/// R = (L - B·(1-α)) / α. None when the layer leaves roughness alone.
fn layer_roughness(opt: &BuildOptions, id: &str, m: &str, s: &Solved, rect: [u32; 4]) -> Result<Option<String>> {
    let path = |v: &str| opt.capture.join(v).join(format!("{m}_Roughness.png"));
    let (pb, pl) = (path(&opt.base), path(&format!("{id}{}", opt.base_suffix)));
    if !pb.is_file() || !pl.is_file() {
        return Ok(None);
    }
    let b = image::open(&pb)?.to_luma8();
    let l = image::open(&pl)?.to_luma8();
    if b.dimensions() != (s.w, s.h) || l.dimensions() != (s.w, s.h) {
        return Ok(None);
    }
    let mut changed = 0usize;
    let img = RgbaImage::from_fn(rect[2], rect[3], |x, y| {
        let (x, y) = (x + rect[0], y + rect[1]);
        let i = (y * s.w + x) as usize;
        let a = s.a[i];
        let (bv, lv) = (b.get_pixel(x, y)[0] as f32, l.get_pixel(x, y)[0] as f32);
        if (lv - bv).abs() > 3.0 {
            changed += 1;
        }
        if a < opt.min_alpha {
            return image::Rgba([0, 0, 0, 0]);
        }
        let r = ((lv - bv * (1.0 - a)) / a).clamp(0.0, 255.0).round() as u8;
        image::Rgba([r, r, r, (a * 255.0).round() as u8])
    });
    if changed < 64 {
        return Ok(None);
    }
    let file = format!("layers/{id}.{m}.rough.png");
    img.save(opt.out.join(&file))?;
    Ok(Some(file))
}

/// The layer's normals: CC5's exported normal map of the layer on the character
/// (`<variant>/tex/<material>_Normal.png`, with `"export": true`) where it differs
/// from the character's own, opacity ramping in over 2..10 levels of change.
fn layer_normal(opt: &BuildOptions, id: &str, m: &str) -> Result<Option<(String, [u32; 4])>> {
    let path = |v: &str| opt.capture.join(v).join("tex").join(format!("{m}_Normal.png"));
    let (pb, pl) = (path(&opt.character), path(&format!("{id}{}", opt.base_suffix)));
    if !pb.is_file() || !pl.is_file() {
        return Ok(None);
    }
    let b = image::open(&pb)?.to_rgb8();
    let l = image::open(&pl)?.to_rgb8();
    if b.dimensions() != l.dimensions() {
        return Ok(None);
    }
    let (w, h) = b.dimensions();
    let alpha = |x: u32, y: u32| {
        let (p, q) = (b.get_pixel(x, y), l.get_pixel(x, y));
        let d = (0..3).map(|c| (p[c] as f32 - q[c] as f32).abs()).fold(0.0, f32::max);
        ((d - 2.0) / 8.0).clamp(0.0, 1.0)
    };
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0usize);
    for y in 0..h {
        for x in 0..w {
            if alpha(x, y) > 0.0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
                n += 1;
            }
        }
    }
    if n < 64 {
        return Ok(None);
    }
    let rect = [x0, y0, x1 - x0, y1 - y0];
    let img = RgbaImage::from_fn(rect[2], rect[3], |x, y| {
        let (x, y) = (x + x0, y + y0);
        let q = l.get_pixel(x, y);
        image::Rgba([q[0], q[1], q[2], (alpha(x, y) * 255.0).round() as u8])
    });
    let file = format!("layers/{id}.{m}.normal.png");
    img.save(opt.out.join(&file))?;
    println!("{id:24} {m:14} normals: {n} texels, rect {rect:?}");
    Ok(Some((file, rect)))
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
                diffuse_path(&opt.capture, &format!("{id}{}", opt.base_suffix), m),
                opt.alt_image.clone().or_else(|| diffuse_path(&opt.capture, &opt.alt, m)),
                diffuse_path(&opt.capture, &format!("{id}{}", opt.alt_suffix), m),
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
            // CC5's SkinGen bake composites onto its own base skin: a second base
            // the bake ignored gives the same texture twice and no opacity.
            let same = l1.iter().zip(&l2).all(|(a, b)| (0..3).all(|c| (a[c] - b[c]).abs() < 1.0 / 255.0));
            anyhow::ensure!(!same, "{id} {m}: the bake on the second base equals the one on the character (the base was ignored)");
            // a preset CC5 did not apply on the second base (e.g. over its layer limit)
            let changed = |b: &[[f32; 3]], l: &[[f32; 3]]| b.iter().zip(l).filter(|(b, l)| (0..3).any(|c| (b[c] - l[c]).abs() > 3.0 / 255.0)).count();
            let (n1, n2) = (changed(&b1, &l1), changed(&b2, &l2));
            anyhow::ensure!(n1 < 100 || n2 * 20 > n1, "{id} {m}: changes {n1} texels on the character but {n2} on the second base (not applied there?)");
            pack.size = pack.size.max(w);
            // the blend mode that explains both bases best
            let s = [Blend::Normal, Blend::Multiply, Blend::Screen]
                .map(|b| solve(b, &b1, &l1, &b2, &l2, w, h))
                .into_iter()
                .min_by(|a, b| a.err_alt.total_cmp(&b.err_alt))
                .unwrap();
            let Some((img, rect, mean)) = crop(&s, opt.min_alpha) else { continue };
            let file = format!("layers/{id}.{m}.png");
            img.save(opt.out.join(&file))?;
            let cover = img.pixels().filter(|p| p[3] > 0).count() as f32;
            let check = validate(opt, id, m, &img, rect, s.blend, w, h)?;
            println!(
                "{id:24} {m:14} {:8} rect {rect:?} size {:.1} err {:.2} ({cover} texels){}",
                format!("{:?}", s.blend).to_lowercase(),
                s.size,
                s.err_alt,
                check.map_or(String::new(), |(err, size)| format!("; held-out base: err {err:.2} vs change {size:.2}"))
            );
            if cover > best {
                best = cover;
                color = mean;
            }
            let roughness = layer_roughness(opt, id, m, &s, rect)?;
            let normal = layer_normal(opt, id, m)?;
            maps.insert(m.clone(), LayerMap { diffuse: file, rect, blend: s.blend, roughness, normal });
        }
        if !maps.is_empty() {
            pack.layers.push(Layer { id: id.clone(), name: name.clone(), group: group.clone(), color, maps, source: String::new() });
        }
    }
    for (id, name) in skins {
        let d = opt.capture.join(id);
        let dst = opt.out.join("skins").join(id);
        std::fs::create_dir_all(&dst)?;
        let mut maps: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        // baked channels (GetImage names) first, then the export's; each against the character's
        let ch_dir = opt.capture.join(&opt.character);
        let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (dir, orig_dir) in [(d.clone(), ch_dir.clone()), (d.join("tex"), ch_dir.join("tex"))] {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd {
                let p = e?.path();
                if p.is_file() {
                    let f = p.file_name().unwrap().to_owned();
                    files.push((p, orig_dir.join(f)));
                }
            }
        }
        for (p, orig) in files {
            let f = p.file_name().unwrap().to_string_lossy().to_string();
            let Some(m) = opt.materials.iter().find(|m| f.starts_with(&format!("{m}_"))) else { continue };
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let channel = match &stem[m.len() + 1..] {
                "Roughness" => "roughness".to_string(),
                "AmbientOcclusion" => "ao".to_string(),
                c => c.to_string(),
            };
            if !SKIN_CHANNELS.contains(&channel.as_str()) || maps.get(m).is_some_and(|mm| mm.contains_key(&channel_key(&channel))) {
                continue;
            }
            // the same as the character's: nothing to swap
            if std::fs::read(&orig).ok() == Some(std::fs::read(&p)?) {
                continue;
            }
            if channel == "roughness" {
                // as a glTF metallic-roughness map (roughness in G, metal 0) for StandardMaterial
                let r = image::open(&p)?.to_luma8();
                let mr = image::RgbImage::from_fn(r.width(), r.height(), |x, y| image::Rgb([0, r.get_pixel(x, y)[0], 0]));
                let out = format!("{m}_MR.png");
                mr.save(dst.join(&out))?;
                maps.entry(m.clone()).or_default().insert("MetallicRoughness".into(), format!("skins/{id}/{out}"));
                continue;
            }
            // colour and normal maps as JPEG (q 92), the rest as PNG
            let big = ["Diffuse", "Normal"].contains(&channel.as_str());
            let out = if big { format!("{m}_{channel}.jpg") } else { format!("{m}_{channel}.png") };
            if big {
                let img = image::open(&p)?.to_rgb8();
                let mut w = std::io::BufWriter::new(std::fs::File::create(dst.join(&out))?);
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 92).encode_image(&img)?;
            } else {
                std::fs::copy(&p, dst.join(&out))?;
            }
            maps.entry(m.clone()).or_default().insert(channel, format!("skins/{id}/{out}"));
        }
        let vars = export_vars(&d.join("character.json"), &opt.materials);
        println!("skin {id}: {} materials, vars for {}", maps.len(), vars.len());
        pack.skins.push(Skin { id: id.clone(), name: name.clone(), maps, vars, source: String::new() });
    }
    pack.save()?;
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_recovers_colour_and_opacity() {
        let (c, a) = ([0.8f32, 0.1, 0.2], 0.6f32);
        let b1 = vec![[0.7f32, 0.5, 0.4], [0.3, 0.3, 0.3]];
        let b2 = vec![[0.5f32, 0.35, 0.3], [0.6, 0.5, 0.4]];
        let over = |b: &[f32; 3]| [0, 1, 2].map(|k| b[k] * (1.0 - a) + c[k] * a);
        let l1: Vec<[f32; 3]> = b1.iter().map(over).collect();
        let l2: Vec<[f32; 3]> = b2.iter().map(over).collect();
        let s = solve(Blend::Normal, &b1, &l1, &b2, &l2, 2, 1);
        for i in 0..2 {
            assert!((s.a[i] - a).abs() < 1e-3, "alpha {}", s.a[i]);
            for k in 0..3 {
                assert!((s.c[i][k] - c[k]).abs() < 1e-3, "colour {:?}", s.c[i]);
            }
        }
        assert!(s.err_alt < 0.5, "alt err {} (0..255)", s.err_alt);
    }

    #[test]
    fn solve_tells_blend_modes_apart() {
        // light and dark skin bases, as the capture's character and dark project
        let b1 = vec![[0.75f32, 0.55, 0.45], [0.65, 0.45, 0.35]];
        let b2 = vec![[0.40f32, 0.28, 0.20], [0.30, 0.20, 0.15]];
        for (blend, c, a) in [(Blend::Multiply, [0.2f32, 0.0, 0.1], 0.7f32), (Blend::Screen, [1.0f32, 0.9, 0.8], 0.4), (Blend::Normal, [0.8, 0.1, 0.2], 0.6)] {
            let over = |b: &[f32; 3]| [0, 1, 2].map(|k| blend.apply(b[k], c[k], a));
            let l1: Vec<[f32; 3]> = b1.iter().map(over).collect();
            let l2: Vec<[f32; 3]> = b2.iter().map(over).collect();
            let fits = [Blend::Normal, Blend::Multiply, Blend::Screen].map(|b| solve(b, &b1, &l1, &b2, &l2, 2, 1));
            let best = fits.iter().min_by(|x, y| x.err_alt.total_cmp(&y.err_alt)).unwrap();
            assert_eq!(best.blend, blend, "errors {:?}", fits.iter().map(|f| f.err_alt).collect::<Vec<_>>());
            assert!(best.err_alt < 0.1, "{blend:?} err {}", best.err_alt);
            // the recovered layer reproduces both bases
            for i in 0..2 {
                for k in 0..3 {
                    let p = blend.apply(b2[i][k], best.c[i][k], best.a[i]);
                    assert!((p - l2[i][k]).abs() < 1e-3, "{blend:?} texel {i}: {p} vs {}", l2[i][k]);
                }
            }
        }
    }
}
