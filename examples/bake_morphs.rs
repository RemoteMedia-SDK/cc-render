//! Bake CC5 facial expressions into a character GLB as glTF morph targets.
//! Usage: bake_morphs character.glb expressions_dir arkit_map.json out.glb
//!
//! The avatar editor mixes the expression pack (`cc5-fbx expressions`) on the
//! CPU; the runtime renderer drives morph targets from an ARKit map instead. This
//! writes the morphs `arkit_map.json` uses (each on every mesh it moves; see
//! `cc5_shape::bake_morphs`) into `out.glb`, and next to it
//! `<out>.arkit_map.json`: the same mapping with each morph's `meshes` set to the
//! meshes it was baked on (the renderer only drives a morph on the meshes listed).
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use cc5_shape::{bake_morphs, ExpressionPack};
use serde_json::{json, Value};

/// Bevy's per-mesh morph target limit upstream (cc-render's patched Bevy: 128).
const UPSTREAM_LIMIT: usize = 64;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 5, "character.glb expressions_dir arkit_map.json out.glb required");
    let pack = ExpressionPack::load(Path::new(&args[2]))?;
    let map: Value = serde_json::from_slice(&std::fs::read(&args[3]).with_context(|| format!("reading {}", args[3]))?)?;
    let mapping = map["mapping"].as_object().context("ARKit map has no \"mapping\"")?;
    let mut names: Vec<&str> = Vec::new();
    for targets in mapping.values() {
        for t in targets.as_array().context("mapping entries are lists")? {
            let m = t["morph"].as_str().context("mapping entry without \"morph\"")?;
            if !names.contains(&m) {
                names.push(m);
            }
        }
    }
    let (glb, baked) = bake_morphs(&std::fs::read(&args[1])?, &pack, &names)?;
    let out = Path::new(&args[4]);
    std::fs::write(out, &glb)?;

    let on: BTreeMap<&str, &Vec<String>> = baked.iter().map(|b| (b.name.as_str(), &b.meshes)).collect();
    let mut resolved = serde_json::Map::new();
    for (arkit, targets) in mapping {
        let list: Vec<Value> = targets
            .as_array()
            .unwrap()
            .iter()
            .map(|t| json!({"morph": t["morph"], "weight": t.get("weight").cloned().unwrap_or(json!(1.0)), "meshes": on[t["morph"].as_str().unwrap()]}))
            .collect();
        resolved.insert(arkit.clone(), Value::Array(list));
    }
    let map_path = out.with_extension("arkit_map.json");
    let doc = json!({
        "_comment": "ARKit-52 -> CC5 morph targets baked into the GLB by bake_morphs; each morph is driven on the meshes listed.",
        "_source": args[3],
        "_glb_path": out.file_name().unwrap().to_string_lossy(),
        "mapping": resolved,
    });
    std::fs::write(&map_path, serde_json::to_vec_pretty(&doc)?)?;

    let mut per_mesh: BTreeMap<&str, usize> = BTreeMap::new();
    for b in &baked {
        for m in &b.meshes {
            *per_mesh.entry(m).or_default() += 1;
        }
    }
    println!("wrote {} ({} morphs, {:.1} MB) and {}", out.display(), names.len(), glb.len() as f64 / 1e6, map_path.display());
    for (m, n) in &per_mesh {
        let note = if *n > UPSTREAM_LIMIT { " (over upstream Bevy's 64)" } else { "" };
        println!("  {m}: {n}{note}");
    }
    Ok(())
}
