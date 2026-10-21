//! Part spec and conversion settings (the JSON keys of hair_build.py's specs:
//! `specs/cc5_converted.json`, convert_smart_hair.py), with the same defaults.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A library part to build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub id: String,
    pub slot: String,
    #[serde(default = "scalp")]
    pub kind: String,
    #[serde(default = "strands")]
    pub atlas: String,
    #[serde(default = "one")]
    pub seed: u64,
    /// RLHair variables over the source material's (root/tip colour, dyes, ...).
    #[serde(default)]
    pub vars: serde_json::Map<String, Value>,
    /// Conversion settings, as JSON (merged with `per_source` per source layer).
    pub convert: Value,
}

fn scalp() -> String {
    "scalp".into()
}
fn strands() -> String {
    "strands".into()
}
fn one() -> u64 {
    1
}

/// Conversion settings for one source layer (`convert` + its `per_source`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Convert {
    /// Hair meshes from another export of the character (a Smart Hair group).
    pub source_glb: Option<PathBuf>,
    /// GLB with the CC5 material settings (default: the source GLB).
    pub material_glb: Option<PathBuf>,
    /// Source objects (without `source_glb`: objects of the head GLB).
    pub sources: Vec<String>,
    /// Case-insensitive substring an object name must contain.
    pub include: Option<String>,
    pub exclude: Vec<String>,
    pub per_source: HashMap<String, Value>,
    /// Points per strand.
    pub points: usize,
    pub strands_per_cm2: f64,
    /// Spread of strands off the card plane (m).
    pub thickness: f64,
    /// Strand ribbon width range (m).
    pub width: [f64; 2],
    pub taper: f64,
    /// Per-strand brightness range.
    pub shade: [f64; 2],
    /// A texture column becomes a strand when its alpha (max or mean) clears a
    /// random threshold in this range.
    pub accept: [f64; 2],
    pub accept_on: String,
    pub opacity_power: f64,
    pub clumps_per_cm: f64,
    pub clump_spread: f64,
    pub clump_shade: [f64; 2],
    /// [darkest factor, curve] for hair deep in the volume.
    pub depth_shade: Option<[f64; 2]>,
    pub root_fade: f64,
    pub start_at_hair: bool,
    pub min_tries: usize,
    /// Cards shorter than this × their width become short patch strands.
    pub patch_aspect: f64,
    pub patch_density: f64,
    pub patch_length: f64,
    /// Keep CC5's cards and textures (brows), every material layer.
    pub keep_cards: bool,
    /// Keep the source's other material layers (scalp, caps) as CC5 made them.
    pub source_scalp: bool,
    /// {source layer: "head_box" | "bone:..."} over what `shape_dir` detects.
    pub follow: HashMap<String, String>,
    /// The group's CC5 shape OBJs / bone dumps (follow detection).
    pub shape_dir: Option<PathBuf>,
    /// Shape edits (applied after conversion; all off by default).
    pub shape: ShapeEdits,
}

impl Default for Convert {
    fn default() -> Self {
        Self {
            source_glb: None,
            material_glb: None,
            sources: Vec::new(),
            include: None,
            exclude: Vec::new(),
            per_source: HashMap::new(),
            points: 14,
            strands_per_cm2: 9.0,
            thickness: 0.003,
            width: [0.00008, 0.00013],
            taper: 0.6,
            shade: [0.82, 1.08],
            accept: [0.05, 0.6],
            accept_on: "mean".into(),
            opacity_power: 0.5,
            clumps_per_cm: 0.0,
            clump_spread: 0.22,
            clump_shade: [0.75, 1.15],
            depth_shade: None,
            root_fade: 0.0,
            start_at_hair: false,
            min_tries: 2,
            patch_aspect: 2.5,
            patch_density: 40.0,
            patch_length: 0.018,
            keep_cards: false,
            source_scalp: true,
            follow: HashMap::new(),
            shape_dir: None,
            shape: ShapeEdits::default(),
        }
    }
}

/// Edits on the converted strands (root -> tip, t in 0..1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapeEdits {
    /// Arc length scale (1 = as converted; < 1 trims the tips, > 1 extends them).
    pub length: f64,
    /// Comb direction (world, any length) and strength (0..1, grows towards the tip).
    pub comb: [f64; 3],
    pub comb_strength: f64,
    /// Droop along world down (m at the tip of a 10 cm strand).
    pub gravity: f64,
    /// Helix radius (m) and period (m along the strand).
    pub curl_radius: f64,
    pub curl_period: f64,
    /// Random offsets growing towards the tip (m).
    pub frizz: f64,
}

impl Convert {
    /// The settings for one source layer: `base` with that layer's
    /// `per_source` entry merged over it.
    pub fn for_source(base: &Value, source: &str) -> anyhow::Result<Self> {
        let mut v = base.clone();
        if let (Some(obj), Some(over)) = (v.as_object_mut(), base.get("per_source").and_then(|p| p.get(source)).and_then(|o| o.as_object())) {
            for (k, x) in over {
                obj.insert(k.clone(), x.clone());
            }
        }
        // ShapeEdits defaults are 0; length means "as converted" at 1
        let mut c: Self = serde_json::from_value(v)?;
        if c.shape.length == 0.0 {
            c.shape.length = 1.0;
        }
        Ok(c)
    }
}
