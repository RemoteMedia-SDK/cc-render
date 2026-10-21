//! `cc5-hair check`: the editor's hair conform against CC5 itself on whole
//! recipes (was check_conform_cc5.py).
//!
//! Reads the bind-pose OBJs cc5_export_smart_hair.py writes in "shapes" mode
//! (`<dir>/<group>/shape_base.obj`, `shape_<recipe>.obj`), binds every hair
//! object of the base OBJ to CC_Base_Body as the editor does (layers CC5 moves
//! rigidly: "head_box"; other scalp hair to the head; beards / brows to the
//! face), applies the body's move to each recipe and reports the distance to
//! where CC5 put the hair, next to how far CC5 moved it.

use std::path::Path;

use anyhow::{anyhow, Result};
use cc5_shape::{Conform, ConformOptions};
use glam::DVec3;

use crate::follow::{key, layer_modes, not_hair, read_obj};
use crate::regions::regions;

fn percentile(v: &mut [f64], q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    let x = q / 100.0 * (v.len() - 1) as f64;
    let (i, f) = (x.floor() as usize, x.fract());
    v[i] + (v[(i + 1).min(v.len() - 1)] - v[i]) * f
}

fn f32s(p: &[DVec3]) -> Vec<[f32; 3]> {
    p.iter().map(|v| v.as_vec3().to_array()).collect()
}

/// Print the conform error of `group` (a folder under `root`) on every recipe.
pub fn check(root: &Path, group: &str) -> Result<()> {
    let d = root.join(group);
    let base = read_obj(&d.join("shape_base.obj"))?;
    let bname = base.keys().find(|k| k.starts_with("CC_Base_Body")).ok_or_else(|| anyhow!("{group}: no CC_Base_Body in shape_base.obj"))?.clone();
    let body = &base[&bname];
    let reg = regions(body);
    let hair: Vec<&String> = base.keys().filter(|k| !not_hair(k)).collect();
    println!(
        "{group}: body {} verts (scalp {}, face {} anchors); hair objects {:?}",
        body.len(),
        reg.scalp.len(),
        reg.face.len(),
        hair.iter().map(|k| (k.as_str(), base[*k].len())).collect::<Vec<_>>()
    );
    let modes = layer_modes(&d, 0.05)?;
    let body32 = f32s(body);
    let mut conf = Vec::new();
    for k in &hair {
        let pts = &base[*k];
        let mut z: Vec<f64> = pts.iter().map(|p| p.z).collect();
        let lk = k.to_lowercase();
        // scalp hair: most vertices above the eyes (as the catalog's kind)
        let scalp = percentile(&mut z, 50.0) > 1.62 && !["brow", "beard", "mous", "goat", "stubble"].iter().any(|w| lk.contains(w));
        let (mode, anchors, opts) = if modes.get(&key(k)).map(String::as_str) == Some("head_box") {
            ("head_box", &reg.head_box, ConformOptions::HEAD_BOX)
        } else if scalp {
            ("scalp", &reg.head, ConformOptions::SCALP)
        } else {
            ("skin", &reg.face, ConformOptions::SKIN)
        };
        conf.push((k.to_string(), mode, Conform::bind(&body32, anchors, &f32s(pts), opts)?));
    }
    let mut recipes: Vec<_> = std::fs::read_dir(&d)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter_map(|f| f.strip_prefix("shape_").and_then(|r| r.strip_suffix(".obj")).map(str::to_string))
        .filter(|r| r != "base")
        .collect();
    recipes.sort();
    for r in recipes {
        let sh = read_obj(&d.join(format!("shape_{r}.obj")))?;
        let Some(moved) = sh.get(&bname).filter(|m| m.len() == body.len()) else {
            println!("  recipe {r}: body missing / vertex count changed");
            continue;
        };
        let db: Vec<[f32; 3]> = moved.iter().zip(body).map(|(a, b)| (*a - *b).as_vec3().to_array()).collect();
        let mut bm: Vec<f64> = db.iter().map(|v| glam::Vec3::from(*v).length() as f64).collect();
        println!("  recipe {r}: body moved p99 {:.1} mm", percentile(&mut bm, 99.0) * 1000.0);
        for (k, mode, c) in &conf {
            let a = &base[k];
            let Some(b) = sh.get(k).filter(|b| b.len() == a.len()) else {
                println!("    {:<28} missing / vertex count changed", k);
                continue;
            };
            let pred = c.apply(&db);
            let mut e: Vec<f64> = b.iter().zip(a).zip(&pred).map(|((b, a), p)| ((*b - *a) - DVec3::from(glam::Vec3::from(*p).as_dvec3())).length() * 1000.0).collect();
            let mut mv: Vec<f64> = b.iter().zip(a).map(|(b, a)| (*b - *a).length() * 1000.0).collect();
            let name: String = k.chars().take(20).collect();
            println!(
                "    {:<20} {:<8} CC5 moved it p50 {:5.1} p99 {:5.1} | conform error p50 {:5.2} p90 {:5.2} p99 {:5.2} max {:5.2} mm",
                name,
                mode,
                percentile(&mut mv, 50.0),
                percentile(&mut mv, 99.0),
                percentile(&mut e, 50.0),
                percentile(&mut e, 90.0),
                percentile(&mut e, 99.0),
                e.iter().cloned().fold(0.0, f64::max)
            );
        }
    }
    Ok(())
}
