//! Anchor regions for hair conforming, as CC_Base_Body vertex indices (the
//! library's `regions.json`; was hair_regions.py).
//!
//! CC characters share the CC_Base_Body topology, so the regions, picked once on
//! the reference character in bind pose, hold for every character shaped from
//! the same pack. Positions in Blender's frame (metres, Z up, face to -Y):
//!   scalp     the hairline region widened by 10 degrees, without the ears;
//!   face      the head below the hairline, ears and front of the neck: anchors
//!             for beards, moustaches and brows;
//!   head_box  the head without ears and nose: its bounding box is what CC5
//!             fits rigid hair layers to (conform "head_box");
//!   head      the whole head: scalp hair's conform anchors.
//! Indices are mesh-local (control-point order: the pack's CC_Base_Body slice).

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use glam::DVec3;
use serde_json::{json, Value};

const CENTER: DVec3 = DVec3::new(0.0, -0.005, 1.64);
/// (azimuth from the front, elevation) of the hairline, degrees.
const HAIRLINE: [(f64, f64); 8] = [(0.0, 33.0), (30.0, 30.0), (55.0, 20.0), (75.0, 9.0), (100.0, 6.0), (125.0, -8.0), (150.0, -30.0), (180.0, -42.0)];

fn hairline(az: f64) -> f64 {
    if az <= HAIRLINE[0].0 {
        return HAIRLINE[0].1;
    }
    for w in HAIRLINE.windows(2) {
        let ((a0, e0), (a1, e1)) = (w[0], w[1]);
        if az <= a1 {
            return e0 + (e1 - e0) * (az - a0) / (a1 - a0);
        }
    }
    HAIRLINE[HAIRLINE.len() - 1].1
}

pub struct Regions {
    pub scalp: Vec<u32>,
    pub face: Vec<u32>,
    pub head_box: Vec<u32>,
    pub head: Vec<u32>,
}

/// The regions of CC_Base_Body points `p` (Blender frame).
pub fn regions(p: &[DVec3]) -> Regions {
    let margin = 10.0;
    let mut r = Regions { scalp: vec![], face: vec![], head_box: vec![], head: vec![] };
    for (i, &q) in p.iter().enumerate() {
        let d = q - CENTER;
        let dist = d.length();
        let el = (d.z / dist).clamp(-1.0, 1.0).asin().to_degrees();
        let az = d.x.atan2(-d.y).abs().to_degrees();
        let thr = hairline(az);
        let ear = q.x.abs() > 0.071 && q.z < 1.675 && q.y.abs() < 0.03;
        let i = i as u32;
        if el > thr - margin && dist < 0.16 && !ear {
            r.scalp.push(i);
        }
        if dist < 0.17 && q.z > 1.47 && el <= thr + 2.0 && !(q.y > 0.03 && q.z < 1.60) {
            r.face.push(i);
        }
        let nose = q.y < -0.09 && q.z < 1.64;
        let ears = q.x.abs() > 0.065 && q.z < 1.68;
        if dist < 0.11 && !nose && !ears {
            r.head_box.push(i);
        }
        if dist < 0.17 {
            r.head.push(i);
        }
    }
    r
}

impl Regions {
    pub fn to_json(&self) -> Value {
        json!({"mesh": "CC_Base_Body", "scalp": self.scalp, "face": self.face, "head_box": self.head_box, "head": self.head})
    }
}

/// CC_Base_Body's bind positions (Blender frame) from the reference character:
/// a CC5 FBX export, or its `cc5-fbx --dump` JSON (with `.verts.f32`).
pub fn body_points(path: &Path) -> Result<Vec<DVec3>> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fbx")) {
        let c = cc5_fbx::Character::load(path)?;
        let m = c.mesh("CC_Base_Body").ok_or_else(|| anyhow!("{}: no CC_Base_Body", path.display()))?;
        // glTF (Y up) -> Blender (Z up)
        return Ok(m.points.iter().map(|p| DVec3::new(p.x as f64, -p.z as f64, p.y as f64)).collect());
    }
    let d: Value = serde_json::from_slice(&std::fs::read(path).with_context(|| format!("reading {}", path.display()))?)?;
    let v = std::fs::read(format!("{}.verts.f32", path.display()))?;
    let f: Vec<f32> = v.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let mut off = 0;
    for n in d["mesh_order"].as_array().ok_or_else(|| anyhow!("{}: no mesh_order", path.display()))? {
        let n = n.as_str().unwrap_or("");
        let count = d["meshes"][n].as_u64().unwrap_or(0) as usize;
        if n == "CC_Base_Body" {
            return Ok(f[3 * off..3 * (off + count)].chunks_exact(3).map(|c| DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64)).collect());
        }
        off += count;
    }
    Err(anyhow!("{}: no CC_Base_Body", path.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn hairline_interpolates() {
        assert_eq!(super::hairline(0.0), 33.0);
        assert_eq!(super::hairline(15.0), 31.5);
        assert_eq!(super::hairline(180.0), -42.0);
    }
}
