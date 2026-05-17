//! GLB material-name parsing and ARKit→CC5 mapping types.
//!
//! Bevy's glTF loader labels each material as `Material{N}` (where N
//! is the index in `gltf.materials`); we use a side table indexed by
//! that label to recover the authoritative material name (e.g.
//! `Std_Skin_Head`, `Hair_Transparency`) so the runtime override
//! systems can target meshes by their CC5-side name.

use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Resource, Clone, Default)]
pub(crate) struct MaterialNames(pub Vec<String>);

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ArkitMap {
    pub mapping: HashMap<String, Vec<MorphRef>>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct MorphRef {
    pub morph: String,
    pub weight: f32,
    pub meshes: Vec<String>,
}

#[derive(Resource, Clone)]
pub(crate) struct ArkitMapping {
    pub map: Arc<ArkitMap>,
}

/// Parse the GLB's JSON chunk and pull out the `materials[].name`
/// list in index order. Returns an empty Vec on any I/O or parse
/// error — the runtime override systems treat empty as "name lookup
/// disabled" and fall back to the entity-chain heuristic.
pub(crate) fn parse_material_names_from_glb(glb_path: &std::path::Path) -> Vec<String> {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(glb_path) else {
        return Vec::new();
    };
    // Skip 12-byte GLB header.
    let mut hdr = [0u8; 12];
    if f.read_exact(&mut hdr).is_err() {
        return Vec::new();
    }
    let mut clen_typ = [0u8; 8];
    if f.read_exact(&mut clen_typ).is_err() {
        return Vec::new();
    }
    let clen = u32::from_le_bytes(clen_typ[..4].try_into().unwrap()) as usize;
    if &clen_typ[4..] != b"JSON" {
        return Vec::new();
    }
    let mut buf = vec![0u8; clen];
    if f.read_exact(&mut buf).is_err() {
        return Vec::new();
    }
    let s = String::from_utf8_lossy(&buf);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return Vec::new();
    };
    let Some(mats) = v.get("materials").and_then(|m| m.as_array()) else {
        return Vec::new();
    };
    mats.iter()
        .map(|m| {
            m.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

pub(crate) fn load_arkit_map(path: &std::path::Path) -> anyhow::Result<ArkitMap> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read mapping {:?}: {e}", path))?;
    let map: ArkitMap = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("parse mapping {:?}: {e}", path))?;
    Ok(map)
}
