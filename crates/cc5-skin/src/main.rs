//! cc5-skin build <capture_dir> <out_pack_dir> <spec.json>
//!
//! spec.json: {"base": "base", "base_suffix": "", "alt_image": "blue2048.png", "alt_suffix": "@blue",
//!             "character": "base",            (or "alt": "<variant>" instead of alt_image)
//!             "alt_for": {"<layer>": ["<alt variant>", "<alt suffix>"]},   (per-layer second base)
//!             "materials": ["Std_Skin_Head", ...],
//!             "layers": [["lip_rouge", "Rouge", "Lip Makeup"], ...],
//!             "skins": [["full_default_male", "Default Male"], ...]}
//! cc5-skin generate <masks_dir> <out_dir>   (each generator at its default look)
//! See the crate docs (`cc5_skin`).
use anyhow::{Context, Result};
use cc5_skin::build::{build, BuildOptions};
use cc5_skin::generate::{generate, Kind, Masks, Pattern};
use serde_json::Value;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("build") if args.len() == 5 => {
            let spec: Value = serde_json::from_slice(&std::fs::read(&args[4]).context("spec")?)?;
            let s = |k: &str, d: &str| spec[k].as_str().unwrap_or(d).to_string();
            let list = |k: &str| -> Vec<Vec<String>> {
                spec[k].as_array().into_iter().flatten().map(|r| r.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect()).collect()
            };
            let opt = BuildOptions {
                capture: PathBuf::from(&args[2]),
                out: PathBuf::from(&args[3]),
                base: s("base", "skinbase_female_clean"),
                alt: s("alt", "skinbase_male_clean"),
                base_suffix: s("base_suffix", "@female"),
                alt_suffix: s("alt_suffix", "@male"),
                character: s("character", "base"),
                alt_image: spec["alt_image"].as_str().map(PathBuf::from),
                check: spec["check_image"].as_str().map(|p| (PathBuf::from(p), s("check_suffix", "@gray"))),
                materials: spec["materials"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
                min_alpha: spec["min_alpha"].as_f64().unwrap_or(0.02) as f32,
                alt_for: spec["alt_for"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(k, v)| Some((k.clone(), (v[0].as_str()?.to_owned(), v[1].as_str()?.to_owned()))))
                    .collect(),
            };
            let layers: Vec<(String, String, String)> =
                list("layers").into_iter().map(|r| (r[0].clone(), r.get(1).cloned().unwrap_or_else(|| r[0].clone()), r.get(2).cloned().unwrap_or_default())).collect();
            let skins: Vec<(String, String)> = list("skins").into_iter().map(|r| (r[0].clone(), r.get(1).cloned().unwrap_or_else(|| r[0].clone()))).collect();
            let t = std::time::Instant::now();
            let pack = build(&opt, &layers, &skins)?;
            println!("{} layers, {} skins in {:.1} s -> {}", pack.layers.len(), pack.skins.len(), t.elapsed().as_secs_f32(), opt.out.display());
            Ok(())
        }
        Some("generate") if args.len() == 4 => {
            // every generator at its default look, from a head's region masks
            // (<dir>/Std_Skin_Head_{CFULC,NMUIL,EN}Mask.png): <out_dir>/<kind>.png
            // (RGBA: colour, opacity at strength 1) and kinds.json (blend, colour, strength)
            let (dir, out) = (PathBuf::from(&args[2]), PathBuf::from(&args[3]));
            let mask = |n: &str| -> Result<image::RgbaImage> { Ok(image::open(dir.join(format!("Std_Skin_Head_{n}Mask.png"))).context(n.to_string())?.to_rgba8()) };
            let (a, b, c) = (mask("CFULC")?, mask("NMUIL")?, mask("EN")?);
            let m = Masks { w: a.width(), h: a.height(), cfulc: a.into_raw(), nmuil: b.into_raw(), en: c.into_raw() };
            std::fs::create_dir_all(&out)?;
            let mut kinds = serde_json::Map::new();
            for k in Kind::ALL {
                let (rgb, strength) = k.default_look();
                let g = generate(&m, k, rgb, 1, Pattern::default());
                image::RgbaImage::from_raw(m.w, m.h, g.color.rgba).context("size")?.save(out.join(format!("{k:?}.png")))?;
                kinds.insert(format!("{k:?}"), serde_json::json!({"blend": g.color.blend, "color": rgb, "strength": strength}));
            }
            std::fs::write(out.join("kinds.json"), serde_json::to_vec_pretty(&kinds)?)?;
            Ok(())
        }
        _ => anyhow::bail!("usage: cc5-skin build <capture_dir> <out_pack_dir> <spec.json> | generate <masks_dir> <out_dir>"),
    }
}
