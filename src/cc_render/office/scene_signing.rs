//! Signing happens at the renderer producer, never in a forgeable output tap.
use super::{motion_intent::MotionGoal, scene_context::SceneSnapshot};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneSamplerCapability {
    pub protocol: u8,
    pub context_version: u8,
    pub policy: String,
    pub avatar_sha256: String,
    pub profile_identity: String,
    pub authenticated_context: bool,
    pub server_instance: String,
}

// No Debug/Serialize: a key must never be included in scene telemetry.
pub(crate) struct SceneSigner {
    key: [u8; 32],
    instance: String,
    sequence: u64,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = hmac_sha256::Hash::new();
    let mut buffer = [0; 65536];
    loop {
        let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 { break; }
        hash.update(&buffer[..size]);
    }
    Ok(hex(&hash.finalize()))
}

impl SceneSigner {
    pub(crate) fn load(key_file: &Path, avatar: &Path, capability: &SceneSamplerCapability) -> Result<Self, String> {
        let mode = std::env::var("CC_AVATAR_RETARGET_MODE").unwrap_or_else(|_| "calibrated".into());
        let align = std::env::var("CC_AVATAR_SMPL_ALIGN_Y_DEG").unwrap_or_else(|_| "0".into());
        if mode != "calibrated" || align.parse::<f32>().ok() != Some(0.) {
            return Err("scene calibration requires the registered calibrated retarget without alignment override".into());
        }
        if capability.protocol != 3 || capability.context_version != 1
            || capability.policy != "model-owned-scene-v1" || !capability.authenticated_context
            || capability.server_instance.len() != 32 || capability.profile_identity.len() != 64
            || file_hash(avatar)? != capability.avatar_sha256 {
            return Err("scene sampler capability/avatar mismatch".into());
        }
        let metadata = std::fs::symlink_metadata(key_file).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("scene signing key must be a regular file".into());
        }
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err("scene signing key must be owner-only".into());
            }
        }
        let bytes = std::fs::read(key_file).map_err(|e| e.to_string())?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| "invalid scene signing key length")?;
        Ok(Self { key, instance: capability.server_instance.clone(), sequence: 0 })
    }

    pub(crate) fn sign(&mut self, goal: &MotionGoal, snapshot: &SceneSnapshot,
                       phase: &str, description: &str, hand: &str) -> Result<serde_json::Value, String> {
        if goal.scene_outcome().is_none() || !snapshot.matches(goal) {
            return Err("scene job requires matching renderer-owned snapshot and v3 goal".into());
        }
        let sequence = self.sequence.checked_add(1).filter(|n| *n < (1_u64 << 53))
            .ok_or("scene signing sequence exhausted")?;
        let payload = serde_json::to_string(&serde_json::json!({
            "request": goal, "snapshot": snapshot, "phase": phase,
            "description": description, "hand": hand, "sequence": sequence,
            "server_instance": self.instance,
        })).map_err(|e| e.to_string())?;
        let signature = hex(&hmac_sha256::HMAC::mac(payload.as_bytes(), self.key));
        self.sequence = sequence;
        Ok(serde_json::json!({"kind":"scene_step", "payload":payload, "signature":signature}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::controller::OfficeController;

    #[test]
    fn startup_key_and_avatar_are_bound_before_signing() {
        use std::io::Write;
        let mut key = tempfile::NamedTempFile::new().unwrap();
        key.write_all(&[7;32]).unwrap();
        let mut avatar = tempfile::NamedTempFile::new().unwrap();
        avatar.write_all(b"test avatar").unwrap();
        let mut capability = SceneSamplerCapability { protocol: 3, context_version: 1,
            policy: "model-owned-scene-v1".into(), avatar_sha256: file_hash(avatar.path()).unwrap(),
            profile_identity: "0".repeat(64), authenticated_context: true, server_instance: "1".repeat(32) };
        assert!(SceneSigner::load(key.path(), avatar.path(), &capability).is_ok());
        capability.avatar_sha256 = "0".repeat(64);
        assert!(SceneSigner::load(key.path(), avatar.path(), &capability).is_err());
    }

    #[test]
    fn hmac_matches_rfc4231() {
        let signature = hmac_sha256::HMAC::mac(b"Hi There", [0x0b; 20]);
        assert_eq!(hex(&signature), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }

    #[test]
    fn signer_binds_native_identity_and_monotonic_sequence() {
        let goal: MotionGoal = serde_json::from_value(serde_json::json!({"kind":"motion_goal",
            "version":3,"session_id":"test","generation_revision":1,"seed":42,"pts_ms":0,
            "goal":{"operation":"navigate","target_id":"desk"}})).unwrap();
        let snapshot = SceneSnapshot::from_office(&OfficeController::default().snapshot(), "test", 1).unwrap();
        let mut signer = SceneSigner { key: [7;32], instance: "instance".into(), sequence: 0 };
        let first = signer.sign(&goal, &snapshot, "arrive", "A person walks.", "right").unwrap();
        let second = signer.sign(&goal, &snapshot, "arrive", "A person walks.", "right").unwrap();
        assert_ne!(first["signature"], second["signature"]);
        // Opt-in diagnostic fixture export; never present in production builds.
        if let Ok(path) = std::env::var("CC_SCENE_SIGNING_TEST_EXPORT") {
            std::fs::write(path, serde_json::to_vec(&first).unwrap()).unwrap();
        }
        let payload: serde_json::Value = serde_json::from_str(second["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["sequence"], 2);
        let wrong = SceneSnapshot::from_office(&OfficeController::default().snapshot(), "other", 1).unwrap();
        assert!(signer.sign(&goal, &wrong, "arrive", "walk", "right").is_err());
        assert_eq!(signer.sequence, 2);
    }
}
