//! cc5-skin build <capture_dir> <out_pack_dir> <spec.json>
//!
//! spec.json: {"base": "skinbase_female_clean", "alt": "skinbase_male_clean",
//!             "base_suffix": "@female", "alt_suffix": "@male", "character": "base",
//!             "materials": ["Std_Skin_Head", ...],
//!             "layers": [["lip_rouge", "Rouge", "Lip Makeup"], ...],
//!             "skins": [["full_default_male", "Default Male"], ...]}
//! See the crate docs (`cc5_skin`).
use anyhow::{Context, Result};
use cc5_skin::build::{build, BuildOptions};
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
                materials: spec["materials"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
                min_alpha: spec["min_alpha"].as_f64().unwrap_or(0.02) as f32,
            };
            let layers: Vec<(String, String, String)> =
                list("layers").into_iter().map(|r| (r[0].clone(), r.get(1).cloned().unwrap_or_else(|| r[0].clone()), r.get(2).cloned().unwrap_or_default())).collect();
            let skins: Vec<(String, String)> = list("skins").into_iter().map(|r| (r[0].clone(), r.get(1).cloned().unwrap_or_else(|| r[0].clone()))).collect();
            let t = std::time::Instant::now();
            let pack = build(&opt, &layers, &skins)?;
            println!("{} layers, {} skins in {:.1} s -> {}", pack.layers.len(), pack.skins.len(), t.elapsed().as_secs_f32(), opt.out.display());
            Ok(())
        }
        _ => anyhow::bail!("usage: cc5-skin build <capture_dir> <out_pack_dir> <spec.json>"),
    }
}
