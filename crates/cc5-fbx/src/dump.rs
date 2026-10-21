//! `fbx_to_glb.py`'s dump, for the pack builders (build_editor_pack.py,
//! build_hd_map.py, hair_regions.py). In Blender's frame (Z up, metres):
//!
//! ```json
//! {"joints": {name: [x, y, z]},          // bind-pose joint positions
//!  "meshes": {name: vertex count},       // control points
//!  "mesh_order": [name, ...],
//!  "parents": {name: parent or null},
//!  "weights": {mesh: [[[joint, w], ...] per vertex]}}
//! ```
//! plus `<dump>.verts.f32`: every mesh's rest positions (f32 xyz), in `mesh_order`.

use std::path::Path;

use anyhow::{Context, Result};
use glam::Vec3;
use serde_json::{json, Map, Value};

use crate::Character;

/// Y-up -> Blender's Z-up.
fn z_up(p: Vec3) -> [f32; 3] {
    [p.x, -p.z, p.y]
}

fn num(x: f32) -> Value {
    json!(x as f64)
}

pub fn write(c: &Character, path: &Path) -> Result<()> {
    let mut joints = Map::new();
    let mut parents = Map::new();
    for j in &c.joints {
        joints.insert(j.name.clone(), json!(z_up(j.world.w_axis.truncate()).map(num)));
        parents.insert(j.name.clone(), j.parent.map_or(Value::Null, |p| json!(c.joints[p].name)));
    }
    let mut meshes = Map::new();
    let mut weights = Map::new();
    let mut flat: Vec<u8> = Vec::new();
    for m in &c.meshes {
        meshes.insert(m.name.clone(), json!(m.points.len()));
        weights.insert(
            m.name.clone(),
            Value::Array(
                m.weights
                    .iter()
                    .map(|ws| json!(ws.iter().map(|&(j, w)| json!([c.joints[j as usize].name, (w as f64 * 1e6).round() / 1e6])).collect::<Vec<_>>()))
                    .collect(),
            ),
        );
        for p in &m.points {
            for x in z_up(*p) {
                flat.extend_from_slice(&x.to_le_bytes());
            }
        }
    }
    let order: Vec<&str> = c.meshes.iter().map(|m| m.name.as_str()).collect();
    let out = json!({"joints": joints, "meshes": meshes, "mesh_order": order, "parents": parents, "weights": weights});
    let mut text = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut text, serde_json::ser::PrettyFormatter::with_indent(b" "));
    serde::Serialize::serialize(&out, &mut ser)?;
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    let vp = format!("{}.verts.f32", path.display());
    std::fs::write(&vp, flat).with_context(|| format!("writing {vp}"))?;
    Ok(())
}
