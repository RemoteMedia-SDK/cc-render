//! `<glb_basename>.physics.json` loader. Schema documented in the spec.
//!
//! All sections within the file are optional. A missing sidecar file
//! returns `Ok(None)` from [`load`] — callers (in `body_colliders.rs`,
//! `secondary.rs`, etc.) supply their own defaults rather than this
//! loader fabricating a `SidecarRaw`. That's why `SidecarRaw` does NOT
//! derive `Default` — every concrete sidecar comes from a parsed,
//! version-validated file.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct SidecarRaw {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub palm_offset_local: Option<[f32; 3]>,
    #[serde(default)]
    pub body_colliders_override: HashMap<String, Option<BodyColliderOverride>>,
    #[serde(default)]
    pub secondary_motion: Option<SecondaryMotionRaw>,
    #[serde(default)]
    pub play_volume: Option<PlayVolumeRaw>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum BodyColliderOverride {
    Capsule { radius: f32, half_height: f32 },
    Sphere { radius: f32 },
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SecondaryMotionRaw {
    #[serde(default = "default_jiggle_patterns")]
    pub auto_detect_patterns: Vec<String>,
    #[serde(default)]
    pub chains_override: Vec<JiggleChainRaw>,
}

fn default_jiggle_patterns() -> Vec<String> {
    vec![
        r"^CC_Base_Hair_.*".into(),
        r".*Breast.*".into(),
        r"^Skirt_.*".into(),
        r"^CC_Base_Tongue.*".into(),
    ]
}

#[derive(Debug, Clone, Deserialize)]
pub struct JiggleChainRaw {
    pub root_bone: String,
    #[serde(default = "default_stiffness")]
    pub stiffness: f32,
    #[serde(default = "default_damping")]
    pub damping: f32,
    #[serde(default = "default_mass")]
    pub mass_per_link: f32,
    #[serde(default = "default_collider_radius")]
    pub collider_radius: f32,
}

fn default_stiffness() -> f32 {
    30.0
}
fn default_damping() -> f32 {
    2.5
}
fn default_mass() -> f32 {
    0.05
}
fn default_collider_radius() -> f32 {
    0.015
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayVolumeRaw {
    pub half_extents_xz: f32,
    pub ceiling_y: f32,
}

/// Load + parse a sidecar. `Ok(None)` if the file doesn't exist (caller
/// uses pure defaults). `Err` only on read/parse failures.
pub fn load(path: &Path) -> anyhow::Result<Option<SidecarRaw>> {
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(path)?;
    let parsed: SidecarRaw = serde_json::from_str(&s)?;
    if parsed.version != 1 {
        anyhow::bail!(
            "physics sidecar at {:?}: unsupported version {} (expected 1)",
            path,
            parsed.version
        );
    }
    Ok(Some(parsed))
}

/// Resolve sidecar path: explicit override wins, otherwise look for
/// `<glb_path>.physics.json` (e.g. `assistant.alphabaked.glb` →
/// `assistant.alphabaked.glb.physics.json`).
pub fn resolve_sidecar_path(glb_path: &Path, explicit: Option<&Path>) -> std::path::PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    let mut s = glb_path.as_os_str().to_os_string();
    s.push(".physics.json");
    s.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(json: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".physics.json")
            .tempfile()
            .unwrap();
        write!(f, "{}", json).unwrap();
        f
    }

    #[test]
    fn loads_v1_with_all_sections() {
        let f = write_temp(
            r#"{
                "version": 1,
                "palm_offset_local": [0.08, 0.0, 0.0],
                "body_colliders_override": {
                    "Head": { "shape": "capsule", "radius": 0.10, "half_height": 0.06 },
                    "L_Hand": null
                },
                "secondary_motion": {
                    "auto_detect_patterns": ["^Hair_.*"],
                    "chains_override": [
                        { "root_bone": "Hair_Front", "stiffness": 40.0, "damping": 3.0, "mass_per_link": 0.06, "collider_radius": 0.02 }
                    ]
                },
                "play_volume": { "half_extents_xz": 2.5, "ceiling_y": 3.5 }
            }"#,
        );
        let parsed = load(f.path()).unwrap().unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.palm_offset_local, Some([0.08, 0.0, 0.0]));
        assert!(parsed.body_colliders_override["L_Hand"].is_none());
        match &parsed.body_colliders_override["Head"] {
            Some(BodyColliderOverride::Capsule {
                radius,
                half_height,
            }) => {
                assert!((radius - 0.10).abs() < 1e-6);
                assert!((half_height - 0.06).abs() < 1e-6);
            }
            other => panic!("expected Head capsule, got {:?}", other),
        }
        let sm = parsed.secondary_motion.unwrap();
        assert_eq!(sm.auto_detect_patterns, vec!["^Hair_.*".to_string()]);
        assert_eq!(sm.chains_override.len(), 1);
        let pv = parsed.play_volume.unwrap();
        assert!((pv.half_extents_xz - 2.5).abs() < 1e-6);
    }

    #[test]
    fn missing_sections_use_defaults() {
        let f = write_temp(r#"{"version": 1}"#);
        let parsed = load(f.path()).unwrap().unwrap();
        assert_eq!(parsed.version, 1);
        assert!(parsed.palm_offset_local.is_none());
        assert!(parsed.body_colliders_override.is_empty());
        assert!(parsed.secondary_motion.is_none());
        assert!(parsed.play_volume.is_none());
    }

    #[test]
    fn missing_file_returns_none() {
        let result = load(Path::new("/nonexistent/path.physics.json")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn unsupported_version_errors() {
        let f = write_temp(r#"{"version": 999}"#);
        let err = load(f.path()).unwrap_err();
        assert!(err.to_string().contains("unsupported version"));
    }

    #[test]
    fn malformed_json_errors() {
        let f = write_temp(r#"{"version": "one""#);
        let err = load(f.path()).unwrap_err();
        // Just confirm we surface a parse error, not an opaque success.
        let s = err.to_string().to_lowercase();
        assert!(
            s.contains("expected") || s.contains("parse") || s.contains("invalid"),
            "expected parse-error message, got: {}",
            err
        );
    }

    #[test]
    fn resolve_sidecar_path_appends_suffix() {
        let p = resolve_sidecar_path(Path::new("/avatars/assistant.alphabaked.glb"), None);
        assert_eq!(
            p.to_str().unwrap(),
            "/avatars/assistant.alphabaked.glb.physics.json"
        );
    }

    #[test]
    fn resolve_sidecar_path_explicit_wins() {
        let p = resolve_sidecar_path(
            Path::new("/avatars/assistant.glb"),
            Some(Path::new("/custom/path.json")),
        );
        assert_eq!(p.to_str().unwrap(), "/custom/path.json");
    }
}
