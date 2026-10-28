//! The avatar editor's expression pack (expressions.json + expressions.bin) from
//! the reference FBX's facial morphs, for CPU morphing (`cc5_shape::ExpressionPack`);
//! was build_expression_pack.py (Blender).
//!
//! CC5 exports ~400 facial morphs per face mesh; Bevy loads at most 64 morph
//! targets per mesh, so the editor applies them on the CPU instead.
//!
//! - expressions.json: `{"version": 1, "vertex_count", "meshes" (as pack.json),
//!   "expressions": [{name, meshes: [mesh names it moves], verts: [byte_offset, count]}]}`,
//!   one entry per morph name merged over every mesh that has it (body first:
//!   its morphs are the canonical list);
//! - expressions.bin: per block u32 index[count] then f32 delta[count * 3]:
//!   morph at weight 1 minus the rest shape, glTF frame (metres, Y up), global
//!   (pack) vertex index.

use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::Character;

/// Deltas smaller than this (metres) are dropped.
const EPS: f32 = 1e-6;

pub fn write(c: &Character, pack_dir: &Path, out_dir: &Path) -> Result<()> {
    let pack: Value = serde_json::from_slice(&std::fs::read(pack_dir.join("pack.json")).with_context(|| format!("reading {}", pack_dir.join("pack.json").display()))?)?;
    let offsets: std::collections::HashMap<&str, (u64, u64)> = pack["meshes"]
        .as_array()
        .context("pack.json: no meshes")?
        .iter()
        .filter_map(|m| Some((m["name"].as_str()?, (m["offset"].as_u64()?, m["count"].as_u64()?))))
        .collect();
    // body first, then by name (as the Blender script)
    let mut meshes: Vec<_> = c.meshes.iter().filter(|m| !m.shapes.is_empty()).collect();
    meshes.sort_by_key(|m| m.name != "CC_Base_Body");
    // {name: [(global index, delta), mesh]} in first-seen order
    let mut order: Vec<String> = Vec::new();
    let mut found: std::collections::HashMap<String, Vec<(Vec<(u32, [f32; 3])>, String)>> = Default::default();
    for m in &meshes {
        let Some(&(off, count)) = offsets.get(m.name.as_str()) else {
            eprintln!("skipping {}: not in the slider pack", m.name);
            continue;
        };
        if m.points.len() as u64 != count {
            bail!("{}: {} vertices, pack has {count}", m.name, m.points.len());
        }
        for s in &m.shapes {
            let d: Vec<(u32, [f32; 3])> = s.deltas.iter().filter(|(_, d)| d.abs().max_element() >= EPS).map(|(i, d)| (*i + off as u32, d.to_array())).collect();
            if !found.contains_key(&s.name) {
                order.push(s.name.clone());
            }
            found.entry(s.name.clone()).or_default().push((d, m.name.clone()));
        }
    }
    std::fs::create_dir_all(out_dir)?;
    let mut bin = std::io::BufWriter::new(std::fs::File::create(out_dir.join("expressions.bin"))?);
    let (mut pos, mut total) = (0u64, 0usize);
    let mut exprs = Vec::new();
    for name in &order {
        let parts = &found[name];
        let mut all: Vec<(u32, [f32; 3])> = parts.iter().flat_map(|(d, _)| d.iter().copied()).collect();
        all.sort_by_key(|x| x.0); // stable: as numpy's argsort(kind="stable")
        for (i, _) in &all {
            bin.write_all(&i.to_le_bytes())?;
        }
        for (_, d) in &all {
            for x in d {
                bin.write_all(&x.to_le_bytes())?;
            }
        }
        let meshes: Vec<&str> = parts.iter().filter(|(d, _)| !d.is_empty()).map(|(_, m)| m.as_str()).collect();
        exprs.push(json!({"name": name, "meshes": meshes, "verts": [pos, all.len()]}));
        pos += all.len() as u64 * 16;
        total += all.len();
    }
    bin.flush()?;
    let out = json!({"version": 1, "units": "m", "frame": "glTF Y-up (cc5-fbx)",
                     "source": c.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
                     "vertex_count": pack["vertex_count"], "meshes": pack["meshes"], "expressions": exprs});
    std::fs::write(out_dir.join("expressions.json"), serde_json::to_string_pretty(&out)?)?;
    let empty = exprs.iter().filter(|e| e["verts"][1] == 0).count();
    println!("expressions: {} ({} move nothing) over {} meshes, {} vertex deltas, {:.1} MB", exprs.len(), empty, meshes.len(), total, pos as f64 / 1e6);
    Ok(())
}
