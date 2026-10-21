//! Which hair layers CC5 moves rigidly with the head box or turns with a bone
//! (port of cc5_layer_follow.py), from cc5_export_smart_hair.py's "shapes" OBJs
//! (`shape_base.obj`, `shape_<recipe>.obj`) and "bones" dumps
//! (`bones_<recipe>.json`) in a group's folder.
//!
//! The catalog's `follow` gets `{mesh: "head_box" | "bone:<CC joint>@x,y,z"}`
//! for the part meshes made from those layers; avatar_editor conforms them so.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use glam::{DMat3, DMat4, DQuat, DVec3, DVec4};
use serde_json::Value;

/// Layers that are not hair (body, eyes, teeth, accessories).
pub fn not_hair(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("cc_base_") || ["eyelash", "earring", "heels", "shoe", "cloth", "teeth", "tongue"].iter().any(|w| n.contains(w))
}

/// Layer names as glTF importers / OBJ writers spell them.
pub fn key(name: &str) -> String {
    let base = name.split('.').next().unwrap_or("").to_lowercase();
    let mut out = String::new();
    let mut gap = false;
    for ch in base.chars() {
        if ch.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            out.push(ch);
            gap = false;
        } else {
            gap = true;
        }
    }
    out
}

/// {group name: vertices (m, CC5 frame: Z up, face -Y)} of a CC5 bind OBJ (cm).
/// A group's vertices are the ones its faces use (CC5 writes them before `g`).
pub fn read_obj(path: &Path) -> Result<BTreeMap<String, Vec<DVec3>>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut verts = Vec::new();
    let mut used: BTreeMap<String, std::collections::BTreeSet<usize>> = BTreeMap::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        if let Some(r) = line.strip_prefix("v ") {
            let v: Vec<f64> = r.split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
            if v.len() == 3 {
                verts.push(DVec3::new(v[0], v[1], v[2]) / 100.0);
            }
        } else if let Some(r) = line.strip_prefix("g ") {
            let g = r.trim().to_string();
            used.entry(g.clone()).or_default();
            cur = Some(g);
        } else if let (Some(r), Some(g)) = (line.strip_prefix("f "), cur.as_ref()) {
            let set = used.get_mut(g).unwrap();
            for t in r.split_whitespace() {
                if let Some(i) = t.split('/').next().and_then(|i| i.parse::<usize>().ok()) {
                    set.insert(i - 1);
                }
            }
        }
    }
    Ok(used.into_iter().filter(|(_, ix)| !ix.is_empty()).map(|(k, ix)| (k, ix.into_iter().map(|i| verts[i]).collect())).collect())
}

fn shapes(dir: &Path) -> Result<(BTreeMap<String, Vec<DVec3>>, Vec<(String, BTreeMap<String, Vec<DVec3>>)>)> {
    let base = read_obj(&dir.join("shape_base.obj"))?;
    let mut out = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    for n in names {
        if let Some(r) = n.strip_prefix("shape_").and_then(|r| r.strip_suffix(".obj")) {
            // slider recipes only: the group on another base character
            // (cc5_export_on_avatar.py, `../_avatars/<name>.obj`) is not a shape change
            let avatar = dir.parent().is_some_and(|p| p.join("_avatars").join(format!("{r}.obj")).exists());
            if r != "base" && !avatar {
                out.push((r.to_string(), read_obj(&dir.join(&n))?));
            }
        }
    }
    Ok((base, out))
}

/// Least-squares affine fit `B ≈ [A 1] M` (M: 4×3 as rows of a DMat4's xyz).
fn affine_fit(a: &[DVec3], b: &[DVec3]) -> [[f64; 3]; 4] {
    let mut ata = [[0.0f64; 4]; 4];
    let mut atb = [[0.0f64; 3]; 4];
    for (p, q) in a.iter().zip(b) {
        let x = [p.x, p.y, p.z, 1.0];
        for i in 0..4 {
            for j in 0..4 {
                ata[i][j] += x[i] * x[j];
            }
            for k in 0..3 {
                atb[i][k] += x[i] * q[k];
            }
        }
    }
    let inv = DMat4::from_cols_array_2d(&ata).inverse();
    let mut m = [[0.0f64; 3]; 4];
    for k in 0..3 {
        let col = inv * DVec4::new(atb[0][k], atb[1][k], atb[2][k], atb[3][k]);
        for i in 0..4 {
            m[i][k] = col[i];
        }
    }
    m
}

/// {layer key: "head_box" | "surface"}: a layer CC5 moved by an exact per-axis
/// scale + translation in every recipe is rigid with the head box.
pub fn layer_modes(dir: &Path, tol_mm: f64) -> Result<HashMap<String, String>> {
    let (base, recs) = shapes(dir)?;
    let mut out = HashMap::new();
    for (k, a) in &base {
        if not_hair(k) || recs.is_empty() {
            continue;
        }
        let (mut rigid, mut moved) = (true, false);
        for (_, sh) in &recs {
            let Some(b) = sh.get(k).filter(|b| b.len() == a.len()) else {
                rigid = false;
                break;
            };
            let m = affine_fit(a, b);
            let diag_only = (0..3).all(|i| (0..3).all(|j| i == j || m[i][j].abs() < 1e-4));
            let resid = a.iter().zip(b).map(|(p, q)| {
                let f = DVec3::new(
                    p.x * m[0][0] + p.y * m[1][0] + p.z * m[2][0] + m[3][0],
                    p.x * m[0][1] + p.y * m[1][1] + p.z * m[2][1] + m[3][1],
                    p.x * m[0][2] + p.y * m[1][2] + p.z * m[2][2] + m[3][2],
                );
                f.distance(*q)
            }).fold(0.0, f64::max) * 1000.0;
            moved |= a.iter().zip(b).map(|(p, q)| p.distance(*q)).fold(0.0, f64::max) > 0.001;
            rigid &= diag_only && resid < tol_mm;
        }
        out.insert(key(k), if rigid && moved { "head_box" } else { "surface" }.to_string());
    }
    Ok(out)
}

/// A bone dump entry -> (rotation, scale, position in m), CC5 frame.
fn xf(b: &Value) -> Option<(DMat3, DVec3, DVec3)> {
    let f = |k: &str| -> Option<Vec<f64>> { b.get(k)?.as_array()?.iter().map(|v| v.as_f64()).collect() };
    let (r, s, t) = (f("r")?, f("s")?, f("t")?);
    Some((DMat3::from_quat(DQuat::from_xyzw(r[0], r[1], r[2], r[3])), DVec3::new(s[0], s[1], s[2]), DVec3::new(t[0], t[1], t[2]) / 100.0))
}

fn angle(r: DMat3) -> f64 {
    (((r.x_axis.x + r.y_axis.y + r.z_axis.z) - 1.0) / 2.0).clamp(-1.0, 1.0).acos().to_degrees()
}

/// {layer key: "bone:<CC joint>@x,y,z"} (glTF frame) for layers CC5 turns with a
/// bone: the bone whose transform change reproduces the layer in every recipe
/// gives the attach point; the CC_Base_ joint turning the same way is the one
/// our rig has.
pub fn bone_modes(dir: &Path, tol_mm: f64) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    let base_bones = dir.join("bones_base.json");
    if !base_bones.exists() {
        return Ok(out);
    }
    let (base, recs) = shapes(dir)?;
    let load = |p: &Path| -> Result<serde_json::Map<String, Value>> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(p)?)?;
        Ok(v.get("bones").and_then(|b| b.as_object()).cloned().unwrap_or_default())
    };
    let b0 = load(&base_bones)?;
    let mut rb = Vec::new();
    for (r, sh) in &recs {
        let p = dir.join(format!("bones_{r}.json"));
        if p.exists() {
            rb.push((sh, load(&p)?));
        }
    }
    if rb.is_empty() {
        return Ok(out);
    }
    for (k, a) in &base {
        if not_hair(k) {
            continue;
        }
        // the bone reproducing the layer's move exactly, in every recipe
        let attach = b0.iter().find_map(|(name, x0)| {
            let (r0, s0, t0) = xf(x0)?;
            for (sh, b1) in &rb {
                let b = sh.get(k).filter(|b| b.len() == a.len())?;
                let (r1, s1, t1) = xf(b1.get(name)?)?;
                let ratio = s1 / s0;
                if (ratio - DVec3::ONE).abs().max_element() > 1e-4 {
                    return None;
                }
                let l = r1 * DMat3::from_diagonal(ratio) * r0.transpose();
                let err = a.iter().zip(b).map(|(p, q)| (l * (*p - t0) + t1).distance(*q)).fold(0.0, f64::max) * 1000.0;
                if err > tol_mm {
                    return None;
                }
            }
            Some(name.clone())
        });
        let Some(attach) = attach else { continue };
        let (ra0, _, ta0) = xf(&b0[&attach]).unwrap();
        let turns: Vec<DMat3> = rb.iter().filter_map(|(_, b1)| xf(b1.get(&attach)?).map(|(r1, ..)| r1 * ra0.transpose())).collect();
        if !turns.iter().any(|r| angle(*r) > 0.05) {
            continue; // pure translation: conform / head_box handle it
        }
        // the CC joint turning exactly as the attach bone does
        let best = b0
            .iter()
            .filter(|(n, _)| n.starts_with("CC_Base_"))
            .filter_map(|(n, x0)| {
                let (r0, ..) = xf(x0)?;
                let errs: Vec<f64> = rb
                    .iter()
                    .zip(&turns)
                    .filter_map(|((_, b1), turn)| xf(b1.get(n)?).map(|(r1, ..)| angle((r1 * r0.transpose()).transpose() * *turn)))
                    .collect();
                (!errs.is_empty()).then(|| (errs.iter().cloned().fold(0.0, f64::max), n.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((err, joint)) = best {
            if err < 0.05 {
                out.insert(key(k), format!("bone:{joint}@{:.5},{:.5},{:.5}", ta0.x, ta0.z, -ta0.y));
            }
        }
    }
    Ok(out)
}

/// {mesh name: mode} for a part's meshes made from rigid source layers
/// (meshes are named `<part id>_<source>` and `<part id>_<source>_layer<n>`).
pub fn part_follow(mesh_names: &[String], part_id: &str, modes: &HashMap<String, String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for m in mesh_names {
        let Some(src) = m.strip_prefix(&format!("{part_id}_")) else { continue };
        let src = match src.rfind("_layer") {
            Some(i) if src[i + 6..].chars().all(|c| c.is_ascii_digit()) && i + 6 < src.len() => &src[..i],
            _ => src,
        };
        if let Some(mode) = modes.get(&key(src)) {
            if mode == "head_box" || mode.starts_with("bone:") {
                out.insert(m.clone(), mode.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_like_python() {
        assert_eq!(key("Viking Beard"), "viking_beard");
        assert_eq!(key("Brows_Hair.002"), "brows_hair");
        assert_eq!(key("--Side part wavy--"), "side_part_wavy");
    }

    /// The Viking group (CC5's own exports): the braid rides the jaw, the
    /// own-hair bangs move with the head box. Skipped where the data is absent.
    #[test]
    fn viking_follow_matches_python() {
        let dir = Path::new(r"D:\cc5_editor\smart_hair\Viking");
        if !dir.join("bones_base.json").exists() {
            return;
        }
        let bones = bone_modes(dir, 0.1).unwrap();
        assert_eq!(bones.get("viking_beard").map(String::as_str), Some("bone:CC_Base_JawRoot@0.00158,1.54476,0.08019"));
        let layers = layer_modes(dir, 0.05).unwrap();
        assert_eq!(layers.get("bang").map(String::as_str), Some("head_box"));
        assert_eq!(layers.get("real_hair").map(String::as_str), Some("head_box"));
        assert_eq!(layers.get("hair_base").map(String::as_str), Some("surface"));
        let names: Vec<String> = ["sh_viking_Viking_Beard", "sh_viking_Viking_Beard_layer1", "sh_viking_Chinstrap_Thick"].map(String::from).to_vec();
        let mut modes = layers;
        modes.extend(bones);
        let f = part_follow(&names, "sh_viking", &modes);
        assert_eq!(f.len(), 2);
    }
}
