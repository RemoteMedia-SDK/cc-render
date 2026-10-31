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
    /// CC5's combination shapes (`C_StretchL_StretchR`, ...), each driven at
    /// the product of its drivers' morph weights; absent in older maps.
    #[serde(default)]
    pub correctives: Vec<Corrective>,
    /// Jaw bone rotation (degrees) at `jawOpen` 1 for this avatar: CC5's rig
    /// 28; a rig hinged further back less (the bakers write it). Its
    /// `Jaw_Open` morph only reshapes the skin around that rotation.
    #[serde(default)]
    pub jaw_open_deg: Option<f32>,
    /// Where the jaw hinges, `[up, forward]` metres from the jaw bone along
    /// the character's axes, for rigs whose jaw bone isn't at the hinge (the
    /// Saluki operators' sits level with the lips, so turning it only drops
    /// them); absent: the bone itself.
    #[serde(default)]
    pub jaw_pivot_m: Option<[f32; 2]>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct MorphRef {
    pub morph: String,
    pub weight: f32,
    pub meshes: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct Corrective {
    pub morph: String,
    /// Morph names (not ARKit channels).
    pub drivers: Vec<String>,
    pub meshes: Vec<String>,
}

impl ArkitMap {
    /// Each corrective's weight (in `correctives` order): the product of its
    /// drivers' morph weights, each the sum over the ARKit channels that drive
    /// that morph, times `amp`, clamped to `0..=amp.max(1)` as the morphs
    /// themselves are.
    pub fn corrective_weights(&self, arkit: &HashMap<String, f32>, amp: f32) -> Vec<f32> {
        if self.correctives.is_empty() {
            return Vec::new();
        }
        let upper = amp.max(1.0);
        let mut morph: HashMap<&str, f32> = HashMap::new();
        for (channel, &w) in arkit {
            if !w.is_finite() || w == 0.0 {
                continue;
            }
            for r in self.mapping.get(channel).into_iter().flatten() {
                *morph.entry(r.morph.as_str()).or_default() += w * r.weight * amp;
            }
        }
        self.correctives
            .iter()
            .map(|c| {
                c.drivers
                    .iter()
                    .map(|d| morph.get(d.as_str()).map_or(0.0, |w| w.clamp(0.0, upper)))
                    .product()
            })
            .collect()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> ArkitMap {
        serde_json::from_value(serde_json::json!({
            "mapping": {
                "mouthStretchLeft": [{"morph": "Mouth_Stretch_L", "weight": 1.0, "meshes": ["Body"]}],
                "mouthStretchRight": [{"morph": "Mouth_Stretch_R", "weight": 1.0, "meshes": ["Body"]}],
                "jawOpen": [{"morph": "Jaw_Open", "weight": 1.0, "meshes": ["Body"]}],
                "mouthSmileLeft": [{"morph": "Mouth_Corner_Pull_L", "weight": 0.5, "meshes": ["Body"]}],
                "cheekSquintLeft": [{"morph": "Mouth_Corner_Pull_L", "weight": 0.5, "meshes": ["Body"]}]
            },
            "correctives": [
                {"morph": "C_StretchL_StretchR", "drivers": ["Mouth_Stretch_L", "Mouth_Stretch_R"], "meshes": ["Body"]},
                {"morph": "C_StretchL_JawOpen", "drivers": ["Mouth_Stretch_L", "Jaw_Open"], "meshes": ["Body"]},
                {"morph": "C_CornerPullL_JawOpen", "drivers": ["Mouth_Corner_Pull_L", "Jaw_Open"], "meshes": ["Body"]}
            ]
        }))
        .unwrap()
    }

    fn pose(kv: &[(&str, f32)]) -> HashMap<String, f32> {
        kv.iter().map(|&(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn correctives_follow_the_product_of_their_drivers() {
        let m = map();
        let w = m.corrective_weights(&pose(&[("mouthStretchLeft", 0.5), ("mouthStretchRight", 0.8), ("jawOpen", 0.0)]), 1.0);
        assert_eq!(w.len(), 3);
        assert!((w[0] - 0.4).abs() < 1e-6, "{w:?}");
        assert_eq!(w[1], 0.0, "a driver at rest keeps it off");
        // A morph driven by two channels sums them (clamped) before the product.
        let w = m.corrective_weights(&pose(&[("mouthSmileLeft", 1.6), ("cheekSquintLeft", 1.6), ("jawOpen", 0.5)]), 1.0);
        assert!((w[2] - 0.5).abs() < 1e-6, "{w:?}");
    }

    #[test]
    fn maps_without_correctives_still_load() {
        let m: ArkitMap = serde_json::from_str(r#"{"mapping": {}}"#).unwrap();
        assert!(m.correctives.is_empty());
        assert_eq!(m.jaw_open_deg, None);
        assert_eq!(m.jaw_pivot_m, None);
        let m: ArkitMap = serde_json::from_str(r#"{"mapping": {}, "jaw_open_deg": 25.4, "jaw_pivot_m": [0.025, 0.014]}"#).unwrap();
        assert_eq!((m.jaw_open_deg, m.jaw_pivot_m), (Some(25.4), Some([0.025, 0.014])));
        assert!(m.corrective_weights(&pose(&[("jawOpen", 1.0)]), 1.0).is_empty());
    }
}
