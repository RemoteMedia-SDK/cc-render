//! Bake CC5 facial expressions into a character GLB as glTF morph targets.
//! Usage: bake_morphs character.glb expressions_dir arkit_map.json out.glb
//!
//! The avatar editor mixes the expression pack (`cc5-fbx expressions`) on the
//! CPU; the runtime renderer drives morph targets from an ARKit map instead. This
//! writes the morphs `arkit_map.json` uses, and its `correctives` (each on every
//! mesh it moves; see `cc5_shape::bake_morphs`), into `out.glb`, and next to it
//! `<out>.arkit_map.json`: the same mapping with each morph's `meshes` set to the
//! meshes it was baked on (the renderer only drives a morph on the meshes listed).
//! For CC5 HD characters use `crates/cc5-shape/data/cc5_hd.arkit_map.json`.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use cc5_shape::{bake_morphs, ExpressionPack};
use serde_json::{json, Value};

/// Bevy's per-mesh morph target limit upstream (cc-render's patched Bevy: 256).
const UPSTREAM_LIMIT: usize = 64;
/// Correctives that move no vertex this far (metres) are left out.
const MIN_CORRECTIVE_M: f32 = 0.001;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 5, "character.glb expressions_dir arkit_map.json out.glb required");
    let pack = ExpressionPack::load(Path::new(&args[2]))?;
    let map: Value = serde_json::from_slice(&std::fs::read(&args[3]).with_context(|| format!("reading {}", args[3]))?)?;
    let mapping = map["mapping"].as_object().context("ARKit map has no \"mapping\"")?;
    let correctives = map.get("correctives").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut names: Vec<&str> = Vec::new();
    for targets in mapping.values() {
        for t in targets.as_array().context("mapping entries are lists")? {
            let m = t["morph"].as_str().context("mapping entry without \"morph\"")?;
            if !names.contains(&m) {
                names.push(m);
            }
        }
    }
    for c in &correctives {
        let m = c["morph"].as_str().context("corrective without \"morph\"")?;
        // A corrective that barely moves (CC5's eyelid ones, ~0.3 mm) isn't worth
        // the GPU memory every morph target costs (~4 MB on a face).
        let moves = pack.find(m).is_some_and(|i| {
            pack.deltas(i).1.iter().any(|d| (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() >= MIN_CORRECTIVE_M)
        });
        if moves && !names.contains(&m) {
            names.push(m);
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
    // A corrective needs its own morph and every driver's baked; the pack's
    // `*_JawOpen` ones move nothing, so they land on no mesh and are left out.
    let baked_fixes: Vec<Value> = correctives
        .iter()
        .filter_map(|c| {
            let meshes = on.get(c["morph"].as_str()?).filter(|m| !m.is_empty())?;
            let drivers = c["drivers"].as_array()?;
            drivers
                .iter()
                .all(|d| d.as_str().is_some_and(|d| on.contains_key(d)))
                .then(|| json!({"morph": c["morph"], "drivers": drivers, "meshes": meshes}))
        })
        .collect();
    let map_path = out.with_extension("arkit_map.json");
    let mut doc = json!({
        "_comment": "ARKit-52 -> CC5 morph targets baked into the GLB by bake_morphs; each morph is driven on the meshes listed, each corrective at the product of its drivers' weights.",
        "_source": args[3],
        "_glb_path": out.file_name().unwrap().to_string_lossy(),
        "mapping": resolved,
        "correctives": baked_fixes,
    });
    // The character's own rig: the jaw bone's turn at jawOpen 1 carries over.
    if let Some(deg) = map.get("jaw_open_deg") {
        doc["jaw_open_deg"] = deg.clone();
    }
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
