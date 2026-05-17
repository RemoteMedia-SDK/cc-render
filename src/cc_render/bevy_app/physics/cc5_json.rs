//! CC5 native physics JSON loader. Parses the `Physics.Collision Shapes`
//! block from a Reallusion CC5 export's `<glb_basename>.json` (sibling
//! to the GLB), converting world-at-bind cm coordinates into bone-local
//! meters for use as body colliders.
//!
//! Note: this is the ARTIST-AUTHORED multi-shape rig source. It slots
//! into the body-collider priority chain ABOVE auto-fit and BELOW the
//! sidecar (`<glb>.physics.json`) override. The two are different
//! conventions:
//!   - `<glb>.json`         — Reallusion CC5 native, this module
//!   - `<glb>.physics.json` — our own sidecar, see `sidecar.rs`

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Top-level shape: one `Capsule` / `Box` per bone, possibly with
/// duplicate-keyed entries (`Capsule`, `Capsule(0)`, `Capsule(1)`, ...).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "Bound Type")]
pub enum Cc5Shape {
    Capsule {
        #[serde(rename = "Bone Active", default = "default_true")]
        bone_active: bool,
        #[serde(rename = "WorldTranslate")]
        world_translate: [f32; 3],
        #[serde(rename = "WorldRotationQ")]
        world_rotation_q: [f32; 4],
        #[serde(rename = "Radius")]
        radius: f32,
        #[serde(rename = "Capsule Length")]
        capsule_length: f32,
        #[serde(rename = "Friction", default = "default_friction")]
        friction: f32,
        #[serde(rename = "Elasticity", default = "default_elasticity")]
        elasticity: f32,
    },
    Box {
        #[serde(rename = "Bone Active", default = "default_true")]
        bone_active: bool,
        #[serde(rename = "WorldTranslate")]
        world_translate: [f32; 3],
        #[serde(rename = "WorldRotationQ")]
        world_rotation_q: [f32; 4],
        #[serde(rename = "Extents", default)]
        extents: Option<[f32; 3]>,
        #[serde(rename = "Friction", default = "default_friction")]
        friction: f32,
        #[serde(rename = "Elasticity", default = "default_elasticity")]
        elasticity: f32,
    },
}

fn default_true() -> bool {
    true
}
fn default_friction() -> f32 {
    0.4
}
fn default_elasticity() -> f32 {
    0.1
}

/// Soft-physics properties CC5 stores per-mesh-material (cloth + hair).
/// Authored values from iClone/CC5; we map them onto our jiggle-body
/// motor parameters when a hair mesh's name matches a jiggle bone.
#[derive(Debug, Clone)]
pub struct Cc5SoftPhysics {
    pub mass: f32,
    pub damping: f32,
    pub drag: f32,
    /// Spring stiffness in CC5's frequency form (Hz). Convert to our
    /// motor stiffness via ω² = k → k = (2π·freq)².
    pub stiffness_freq_hz: f32,
}

/// Per-avatar breast (translation-mode) jiggle tuning, read from a
/// non-Reallusion `"Breast Tuning"` block we add under `Physics`. All
/// fields optional; missing values fall back to the "large bust"
/// defaults baked into the physics code (see `secondary.rs`).
///
/// Sizing presets (rough starting points):
///
/// | Build      | mass | linear_stiffness | linear_damping | translation_limit |
/// |------------|------|------------------|----------------|-------------------|
/// | flat / xs  | 0.2  | 200              | 8.0            | 0.02              |
/// | small      | 0.5  | 120              | 5.0            | 0.05              |
/// | medium     | 1.0  | 70               | 2.5            | 0.09              |
/// | large      | 2.0  | 40               | 1.2            | 0.14              |
/// | very large | 3.0  | 28               | 0.8            | 0.18              |
///
/// Higher mass + lower stiffness + lower damping + larger limit reads
/// as more pendulous, longer-ringing swings. Smaller breasts read
/// tighter (less mass, stiffer spring, more drag, tighter envelope).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Cc5BreastTuning {
    /// Per-side breast bone mass (kg).
    #[serde(rename = "Mass", default)]
    pub mass: Option<f32>,
    /// Linear motor stiffness (k). f_natural ≈ sqrt(k/m) / (2π).
    #[serde(rename = "Linear Stiffness", default)]
    pub linear_stiffness: Option<f32>,
    /// World-frame velocity drag (Rapier `Damping.linear_damping`).
    #[serde(rename = "Linear Damping", default)]
    pub linear_damping: Option<f32>,
    /// Half-width of the per-axis translation cube (meters).
    #[serde(rename = "Translation Limit", default)]
    pub translation_limit: Option<f32>,
}

/// Parsed CC5 physics.
///   - `by_bone`: collision shapes per CC5 bone name (Capsule / Box).
///   - `soft_physics_by_mesh`: Soft Physics block keyed by mesh name
///     (e.g. "Side_part_wavy" → Hair material values). CC5 doesn't
///     author per-bone soft physics — values live on the MESH, and we
///     apply them to bones whose names hint at the same body part.
///   - `breast_tuning`: optional non-CC5 `Breast Tuning` block we read
///     from Physics, used to override the four hardcoded breast jiggle
///     knobs per avatar (see `Cc5BreastTuning`).
#[derive(Debug, Clone, Default)]
pub struct Cc5Physics {
    pub by_bone: HashMap<String, Vec<Cc5Shape>>,
    pub soft_physics_by_mesh: HashMap<String, Cc5SoftPhysics>,
    pub breast_tuning: Option<Cc5BreastTuning>,
}

impl Cc5Physics {
    /// Approximate radius of the avatar's chest in METERS, derived
    /// from the longest authored Capsule on `CC_Base_Spine02` /
    /// `CC_Base_Spine01` / `CC_Base_Hip` (in that priority order).
    /// Used to scale the pendulum offset for jiggle bodies so a
    /// child rig and an adult rig get proportional dynamics. Returns
    /// `None` when no chest-area capsule is authored — caller should
    /// fall back to a hardcoded default (~0.05 m).
    pub fn chest_radius_m(&self) -> Option<f32> {
        for bone in ["CC_Base_Spine02", "CC_Base_Spine01", "CC_Base_Hip"] {
            if let Some(shapes) = self.by_bone.get(bone) {
                let max_radius = shapes
                    .iter()
                    .filter_map(|s| match s {
                        Cc5Shape::Capsule { radius, .. } => Some(*radius),
                        _ => None,
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
                if max_radius.is_finite() && max_radius > 0.0 {
                    // CC5 stores measurements in cm; convert to meters.
                    return Some(max_radius * 0.01);
                }
            }
        }
        None
    }

    /// Approximate breast bone radius in METERS, taken from the largest
    /// authored Capsule on `CC_Base_L_Breast` / `CC_Base_R_Breast`. CC5
    /// stores capsule radius in cm; converted to meters here. When the
    /// breast bone has no authored collision shape (artist didn't paint
    /// soft physics — common case for many CC5 exports), returns
    /// `None` and the caller should fall through to a non-CC5 source.
    pub fn breast_capsule_radius_m(&self) -> Option<f32> {
        for bone in ["CC_Base_L_Breast", "CC_Base_R_Breast"] {
            if let Some(shapes) = self.by_bone.get(bone) {
                let max_radius = shapes
                    .iter()
                    .filter_map(|s| match s {
                        Cc5Shape::Capsule { radius, .. } => Some(*radius),
                        _ => None,
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
                if max_radius.is_finite() && max_radius > 0.0 {
                    return Some(max_radius * 0.01);
                }
            }
        }
        None
    }

    /// Best-effort body soft-physics lookup: returns the first
    /// soft-physics entry whose mesh name suggests a body / skin
    /// surface (`Body`, `Skin`, `Torso`). Used to read `Mass`,
    /// `Damping`, and `Stiffness Frequency` for the breast region —
    /// CC5 keys these per-mesh, not per-bone, and the breast tissue
    /// is painted into the body mesh's soft-vertex mask. The mesh-
    /// level value is therefore an APPROXIMATION (the breast region
    /// inherits the mesh's mass scaled by paint weight, which we
    /// can't read), but it's a reasonable proxy and preserves the
    /// artist's intent better than a hardcoded universal default.
    pub fn body_soft_physics(&self) -> Option<&Cc5SoftPhysics> {
        for (mesh, sp) in &self.soft_physics_by_mesh {
            let n = mesh.to_lowercase();
            if n.contains("body") || n.contains("skin") || n.contains("torso") {
                return Some(sp);
            }
        }
        None
    }

    /// Best-effort lookup of hair soft-physics values: returns the
    /// first entry whose mesh name contains "hair" (case-insensitive),
    /// e.g. `Side_part_wavy` doesn't match this rule, but most CC5
    /// rigs name the hair mesh with `Hair` somewhere. When none
    /// matches, returns the first soft-physics entry whose material
    /// name contains "hair" (covers `Hair_Transparency` materials).
    pub fn hair_soft_physics(&self) -> Option<&Cc5SoftPhysics> {
        // First try mesh-name match
        for (mesh, sp) in &self.soft_physics_by_mesh {
            if mesh.to_lowercase().contains("hair") {
                return Some(sp);
            }
        }
        // Fallback: caller looks up by mesh name (set externally).
        // We can't introspect material names from this map (we keyed
        // by mesh, not material), but by convention any soft-physics
        // entry on a hair-shaped mesh applies. Pick the first entry
        // whose mesh is NOT obviously a cloth garment.
        for (mesh, sp) in &self.soft_physics_by_mesh {
            let n = mesh.to_lowercase();
            if !n.contains("sweater")
                && !n.contains("shirt")
                && !n.contains("pants")
                && !n.contains("dress")
                && !n.contains("cloth")
            {
                return Some(sp);
            }
        }
        None
    }
}

/// Locate `<glb_basename>.json` next to the GLB. CC5 exports name the
/// JSON the same as the FBX/GLB stem (without the `.glb` suffix). Note
/// this differs from the physics sidecar convention which APPENDS
/// `.physics.json` to the full GLB name.
///
/// Example: `avatars/alika_v2.glb` -> `avatars/alika_v2.json`.
pub fn resolve_cc5_json_path(glb_path: &Path) -> Option<std::path::PathBuf> {
    // We can't use `set_extension("json")` here because for paths like
    // `assistant.alphabaked.glb`, `file_stem()` returns `assistant.alphabaked`
    // and `set_extension` would replace the trailing `alphabaked` rather than
    // append. Build the file name explicitly: `<stem>.json`.
    let stem = glb_path.file_stem()?.to_str()?;
    let parent = glb_path.parent()?;
    Some(parent.join(format!("{}.json", stem)))
}

/// Load + parse a CC5 physics JSON. `Ok(None)` if file doesn't exist
/// (no CC5 sidecar shipped — caller falls back to auto-fit). `Err`
/// only on read/parse failures.
///
/// The CC5 JSON is deeply nested:
/// `<root>.<name>.Object.<name>.Physics.Collision Shapes`.
/// We auto-discover `<name>` by taking the first key (CC5 nests by
/// avatar name, which varies per file).
pub fn load(path: &Path) -> anyhow::Result<Option<Cc5Physics>> {
    use serde_json::Value;
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(path)?;
    let root: Value = serde_json::from_str(&s)?;

    // Walk: root -> first key -> "Object" -> first key -> "Physics" -> "Collision Shapes"
    let Some((_, top_obj)) = root.as_object().and_then(|m| m.iter().next()) else {
        return Ok(Some(Cc5Physics::default()));
    };
    let Some(object_block) = top_obj.get("Object") else {
        return Ok(Some(Cc5Physics::default()));
    };
    let Some((_, inner_obj)) = object_block.as_object().and_then(|m| m.iter().next()) else {
        return Ok(Some(Cc5Physics::default()));
    };
    let Some(physics) = inner_obj.get("Physics") else {
        return Ok(Some(Cc5Physics::default()));
    };
    let Some(coll_shapes) = physics.get("Collision Shapes").and_then(|v| v.as_object()) else {
        return Ok(Some(Cc5Physics::default()));
    };

    let mut by_bone: HashMap<String, Vec<Cc5Shape>> = HashMap::new();
    for (bone_name, shapes_obj) in coll_shapes.iter() {
        let Some(shapes_map) = shapes_obj.as_object() else {
            continue;
        };
        let mut shapes_for_bone: Vec<Cc5Shape> = Vec::new();
        for (_shape_key, shape_val) in shapes_map.iter() {
            // Each shape carries its own `Bound Type` field; serde's
            // tagged enum dispatches on it. Silently skip unrecognized
            // (e.g., future shape types we don't support).
            if let Ok(parsed) = serde_json::from_value::<Cc5Shape>(shape_val.clone()) {
                shapes_for_bone.push(parsed);
            }
        }
        by_bone.insert(bone_name.clone(), shapes_for_bone);
    }

    // Soft Physics block (cloth + hair, when present). CC5 nests
    // values under <root>...Physics > Soft Physics > Meshes > <mesh>
    // > Materials > <mat> > {Mass, Damping, Drag, Stiffness Frequency}.
    // We pick the first material per mesh — multi-material meshes
    // are rare in CC5 jiggle contexts.
    let mut soft_physics_by_mesh: HashMap<String, Cc5SoftPhysics> = HashMap::new();
    if let Some(soft) = physics.get("Soft Physics") {
        if let Some(meshes) = soft.get("Meshes").and_then(|v| v.as_object()) {
            for (mesh_name, mesh_block) in meshes {
                let Some(mats) = mesh_block.get("Materials").and_then(|v| v.as_object()) else {
                    continue;
                };
                let Some((_, mat_block)) = mats.iter().next() else {
                    continue;
                };
                let pick = |k: &str, def: f32| -> f32 {
                    mat_block
                        .get(k)
                        .and_then(|v| v.as_f64())
                        .map(|f| f as f32)
                        .unwrap_or(def)
                };
                let sp = Cc5SoftPhysics {
                    mass: pick("Mass", 1.0),
                    damping: pick("Damping", 0.5),
                    drag: pick("Drag", 0.0),
                    stiffness_freq_hz: pick("Stiffness Frequency", 10.0),
                };
                soft_physics_by_mesh.insert(mesh_name.clone(), sp);
            }
        }
    }

    // Non-Reallusion `"Breast Tuning"` block we author. Sits as a
    // sibling of `Collision Shapes` / `Soft Physics` under `Physics`.
    let breast_tuning = physics
        .get("Breast Tuning")
        .and_then(|v| serde_json::from_value::<Cc5BreastTuning>(v.clone()).ok());

    Ok(Some(Cc5Physics {
        by_bone,
        soft_physics_by_mesh,
        breast_tuning,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(json: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, "{}", json).unwrap();
        f
    }

    #[test]
    fn loads_dual_capsule_hip() {
        // Mirrors the structure of avatars/alika_v2.json.
        let f = write_temp(
            r#"{
          "alika_v2": {
            "Object": {
              "alika_v2": {
                "Physics": {
                  "Collision Shapes": {
                    "CC_Base_Hip": {
                      "Capsule": {
                        "Bone Active": true,
                        "Bound Type": "Capsule",
                        "WorldTranslate": [-5.43, 0.92, 106.30],
                        "WorldRotationQ": [0.81, 0.07, -0.10, 0.57],
                        "Radius": 6.81,
                        "Capsule Length": 28.67,
                        "Friction": 0.05,
                        "Elasticity": 0.5
                      },
                      "Capsule(0)": {
                        "Bone Active": true,
                        "Bound Type": "Capsule",
                        "WorldTranslate": [5.43, 0.92, 106.30],
                        "WorldRotationQ": [0.81, -0.07, 0.10, 0.57],
                        "Radius": 6.81,
                        "Capsule Length": 28.67,
                        "Friction": 0.05,
                        "Elasticity": 0.5
                      }
                    },
                    "CC_Base_Pelvis": {},
                    "CC_Base_R_Eye": {
                      "Box": {
                        "Bound Type": "Box",
                        "WorldTranslate": [3.0, 1.0, 165.0],
                        "WorldRotationQ": [0.0, 0.0, 0.0, 1.0],
                        "Friction": 0.4,
                        "Elasticity": 0.1
                      }
                    }
                  }
                }
              }
            }
          }
        }"#,
        );
        let physics = load(f.path()).unwrap().unwrap();
        let hip = physics
            .by_bone
            .get("CC_Base_Hip")
            .expect("CC_Base_Hip should be present");
        assert_eq!(hip.len(), 2, "CC_Base_Hip should have 2 capsules");
        match &hip[0] {
            Cc5Shape::Capsule {
                radius,
                capsule_length,
                ..
            } => {
                assert!((radius - 6.81).abs() < 1e-3);
                assert!((capsule_length - 28.67).abs() < 1e-3);
            }
            _ => panic!("expected Capsule"),
        }
        let pelvis = physics.by_bone.get("CC_Base_Pelvis").unwrap();
        assert!(pelvis.is_empty(), "empty bone vec");
        let eye = physics.by_bone.get("CC_Base_R_Eye").unwrap();
        assert_eq!(eye.len(), 1);
        assert!(matches!(eye[0], Cc5Shape::Box { .. }));
    }

    #[test]
    fn missing_file_returns_none() {
        let result = load(Path::new("/nonexistent/cc5.json")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn empty_root_returns_empty() {
        let f = write_temp("{}");
        let p = load(f.path()).unwrap().unwrap();
        assert!(p.by_bone.is_empty());
    }

    #[test]
    fn missing_physics_block_returns_empty() {
        let f = write_temp(r#"{"avatar":{"Object":{"avatar":{"Type":"Standard"}}}}"#);
        let p = load(f.path()).unwrap().unwrap();
        assert!(p.by_bone.is_empty());
    }

    #[test]
    fn resolve_cc5_json_path_strips_ext() {
        let p = resolve_cc5_json_path(Path::new("/avatars/alika_v2.glb")).unwrap();
        assert_eq!(p.to_str().unwrap(), "/avatars/alika_v2.json");
    }

    #[test]
    fn resolve_cc5_json_path_with_dotted_stem() {
        // `assistant.alphabaked.glb` -> `assistant.alphabaked.json` (CC5
        // sibling convention strips ONLY the last extension).
        let p = resolve_cc5_json_path(Path::new("/avatars/assistant.alphabaked.glb")).unwrap();
        assert_eq!(p.to_str().unwrap(), "/avatars/assistant.alphabaked.json");
    }

    #[test]
    fn loads_soft_physics_for_hair() {
        // Mirrors structure of avatars/assistant.json's Hair material.
        let f = write_temp(
            r#"{
          "test": {
            "Object": {
              "test": {
                "Physics": {
                  "Collision Shapes": {},
                  "Soft Physics": {
                    "Meshes": {
                      "Side_part_wavy": {
                        "Materials": {
                          "Hair_Transparency": {
                            "Mass": 1.5,
                            "Damping": 0.25,
                            "Drag": 0.20,
                            "Stiffness Frequency": 8.0
                          }
                        }
                      },
                      "Sweater": {
                        "Materials": {
                          "Cloth": {
                            "Mass": 1.0,
                            "Damping": 0.5,
                            "Drag": 0.0,
                            "Stiffness Frequency": 10.0
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }"#,
        );
        let p = load(f.path()).unwrap().unwrap();
        let hair_sp = p.soft_physics_by_mesh.get("Side_part_wavy").unwrap();
        assert!((hair_sp.mass - 1.5).abs() < 1e-5);
        assert!((hair_sp.damping - 0.25).abs() < 1e-5);
        assert!((hair_sp.drag - 0.20).abs() < 1e-5);
        assert!((hair_sp.stiffness_freq_hz - 8.0).abs() < 1e-5);

        // hair_soft_physics() prefers a mesh whose name contains "hair".
        // Side_part_wavy doesn't, so fallback rule (skip cloth-y names)
        // should pick Side_part_wavy over Sweater.
        let picked = p.hair_soft_physics().unwrap();
        assert!(
            (picked.stiffness_freq_hz - 8.0).abs() < 1e-5,
            "picked Side_part_wavy values, not Sweater"
        );
    }

    #[test]
    fn chest_radius_picks_largest_spine_capsule() {
        let f = write_temp(
            r#"{
          "test": {
            "Object": {
              "test": {
                "Physics": {
                  "Collision Shapes": {
                    "CC_Base_Spine02": {
                      "Capsule": {
                        "Bound Type": "Capsule",
                        "WorldTranslate": [0,0,0],
                        "WorldRotationQ": [0,0,0,1],
                        "Radius": 8.5,
                        "Capsule Length": 20.0
                      },
                      "Capsule(0)": {
                        "Bound Type": "Capsule",
                        "WorldTranslate": [0,0,0],
                        "WorldRotationQ": [0,0,0,1],
                        "Radius": 7.0,
                        "Capsule Length": 20.0
                      }
                    },
                    "CC_Base_Hip": {
                      "Capsule": {
                        "Bound Type": "Capsule",
                        "WorldTranslate": [0,0,0],
                        "WorldRotationQ": [0,0,0,1],
                        "Radius": 12.0,
                        "Capsule Length": 25.0
                      }
                    }
                  }
                }
              }
            }
          }
        }"#,
        );
        let p = load(f.path()).unwrap().unwrap();
        // Spine02 wins priority over Hip even though Hip is larger.
        // Returns max-radius Spine02 capsule (8.5 cm), converted to m.
        let r = p.chest_radius_m().unwrap();
        assert!((r - 0.085).abs() < 1e-5, "expected 0.085 m, got {}", r);
    }

    #[test]
    fn chest_radius_falls_back_when_spine_missing() {
        let f = write_temp(
            r#"{
          "test": {
            "Object": {
              "test": {
                "Physics": {
                  "Collision Shapes": {
                    "CC_Base_Hip": {
                      "Capsule": {
                        "Bound Type": "Capsule",
                        "WorldTranslate": [0,0,0],
                        "WorldRotationQ": [0,0,0,1],
                        "Radius": 12.0,
                        "Capsule Length": 25.0
                      }
                    }
                  }
                }
              }
            }
          }
        }"#,
        );
        let p = load(f.path()).unwrap().unwrap();
        let r = p.chest_radius_m().unwrap();
        assert!((r - 0.12).abs() < 1e-5);
    }

    #[test]
    fn chest_radius_returns_none_without_capsules() {
        let f = write_temp(r#"{"test":{"Object":{"test":{"Physics":{"Collision Shapes":{}}}}}}"#);
        let p = load(f.path()).unwrap().unwrap();
        assert!(p.chest_radius_m().is_none());
    }

    #[test]
    fn breast_capsule_radius_reads_left_or_right() {
        let f = write_temp(
            r#"{
          "test": { "Object": { "test": { "Physics": { "Collision Shapes": {
            "CC_Base_R_Breast": {
              "Capsule": {
                "Bound Type": "Capsule",
                "WorldTranslate": [0,0,0], "WorldRotationQ": [0,0,0,1],
                "Radius": 5.5, "Capsule Length": 6.0
              }
            }
          }}}}}}"#,
        );
        let p = load(f.path()).unwrap().unwrap();
        let r = p.breast_capsule_radius_m().unwrap();
        assert!((r - 0.055).abs() < 1e-5, "expected 0.055 m, got {}", r);
    }

    #[test]
    fn breast_capsule_radius_none_when_empty_or_absent() {
        let f = write_temp(
            r#"{"test":{"Object":{"test":{"Physics":{"Collision Shapes":{
            "CC_Base_L_Breast": {}, "CC_Base_R_Breast": {}
        }}}}}}"#,
        );
        let p = load(f.path()).unwrap().unwrap();
        assert!(p.breast_capsule_radius_m().is_none());
    }

    #[test]
    fn body_soft_physics_picks_body_mesh() {
        let f = write_temp(
            r#"{
          "test": { "Object": { "test": { "Physics": {
            "Collision Shapes": {},
            "Soft Physics": { "Meshes": {
              "CC_Base_Body": { "Materials": { "Std_Skin_Body": {
                "Mass": 2.4, "Damping": 1.0, "Drag": 0.0, "Stiffness Frequency": 6.0
              }}},
              "Sweater": { "Materials": { "Cloth": {
                "Mass": 1.0, "Damping": 0.5, "Drag": 0.0, "Stiffness Frequency": 10.0
              }}}
            }}
          }}}}}"#,
        );
        let p = load(f.path()).unwrap().unwrap();
        let body = p.body_soft_physics().expect("body soft physics found");
        assert!((body.mass - 2.4).abs() < 1e-5);
        assert!((body.stiffness_freq_hz - 6.0).abs() < 1e-5);
    }

    #[test]
    fn breast_tuning_block_parses() {
        let f = write_temp(
            r#"{
          "test": { "Object": { "test": { "Physics": {
            "Collision Shapes": {},
            "Breast Tuning": {
              "Mass": 0.5,
              "Linear Stiffness": 120.0,
              "Linear Damping": 5.0,
              "Translation Limit": 0.05
            }
          }}}}}"#,
        );
        let p = load(f.path()).unwrap().unwrap();
        let bt = p.breast_tuning.expect("breast tuning block parsed");
        assert!((bt.mass.unwrap() - 0.5).abs() < 1e-5);
        assert!((bt.linear_stiffness.unwrap() - 120.0).abs() < 1e-5);
        assert!((bt.linear_damping.unwrap() - 5.0).abs() < 1e-5);
        assert!((bt.translation_limit.unwrap() - 0.05).abs() < 1e-5);
    }

    #[test]
    fn breast_tuning_partial_block_uses_some_for_set_keys() {
        let f = write_temp(
            r#"{
          "test": { "Object": { "test": { "Physics": {
            "Collision Shapes": {},
            "Breast Tuning": { "Mass": 1.0 }
          }}}}}"#,
        );
        let p = load(f.path()).unwrap().unwrap();
        let bt = p.breast_tuning.unwrap();
        assert_eq!(bt.mass, Some(1.0));
        assert!(bt.linear_stiffness.is_none());
        assert!(bt.linear_damping.is_none());
        assert!(bt.translation_limit.is_none());
    }

    #[test]
    fn breast_tuning_absent_when_no_block() {
        let f = write_temp(r#"{"test":{"Object":{"test":{"Physics":{"Collision Shapes":{}}}}}}"#);
        let p = load(f.path()).unwrap().unwrap();
        assert!(p.breast_tuning.is_none());
    }
}
