//! Material override system.
//!
//! IMPORTANT: per the empirical bug found in the prototype
//! (avatars/README.md "Common issues" + memory note
//! `feedback_bevy_material_mutation`): in Bevy 0.15 you must NOT
//! mutate `mat.base_color` (or `cull_mode`/PBR factors) through
//! `Assets<StandardMaterial>::get_mut()`. Doing so corrupts unrelated
//! meshes' GPU bindings (skin/cloth/hair stop rendering).
//!
//! All PBR-factor patches (alphaMode, baseColorFactor, metallic,
//! roughness) are baked into the GLB JSON by `scripts/avatars/bake_alpha.py`
//! before the renderer ever sees the asset. This system is currently
//! a no-op tracker: it walks every newly-loaded material once,
//! records its name+entity-chain for diagnostics, and stops there.
//!
//! Kept around as a hook for the day Bevy 0.16+ fixes the
//! base_color-mutation bug — the resolution + lookup work is already
//! plumbed.

use bevy::prelude::*;
use std::collections::HashSet;

use super::assets::MaterialNames;

pub(crate) fn override_hair_alpha(
    asset_server: Res<AssetServer>,
    mat_names: Res<MaterialNames>,
    parent_q: Query<&Parent>,
    name_q: Query<&Name>,
    mat_q: Query<(Entity, &MeshMaterial3d<StandardMaterial>)>,
    mats: Res<Assets<StandardMaterial>>,
    mut applied: Local<HashSet<AssetId<StandardMaterial>>>,
) {
    let mut patched = 0u32;
    for (entity, mat_handle) in mat_q.iter() {
        let id = mat_handle.0.id();
        if applied.contains(&id) {
            continue;
        }

        // Authoritative material name via the GltfLoader's
        // `Material{N}` label → index into the side table parsed at
        // startup.
        let _mat_name_lc = asset_server
            .get_path(id.untyped())
            .and_then(|p| {
                let label = p.label()?.to_string();
                let idx_str = label.strip_prefix("Material")?;
                idx_str.parse::<usize>().ok()
            })
            .and_then(|idx| mat_names.0.get(idx).cloned())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        // Walk up to ~8 ancestors collecting node names — fallback
        // hint for the rare material whose name didn't resolve.
        let mut e = entity;
        let mut chain_lc = String::new();
        for _ in 0..8 {
            if let Ok(name) = name_q.get(e) {
                chain_lc.push('/');
                chain_lc.extend(name.as_str().chars().map(|c| c.to_ascii_lowercase()));
            }
            match parent_q.get(e) {
                Ok(p) => e = p.get(),
                Err(_) => break,
            }
        }

        // No mutation today — see module docstring.
        let _ = mats.get(&mat_handle.0);

        applied.insert(id);
        patched += 1;
    }
    if patched > 0 {
        debug!(
            "cc_render override_materials: tracked {} new material(s) (total: {})",
            patched,
            applied.len()
        );
    }
}
