# Condo garments

One directory per garment, each holding a standalone skinned GLB and a small
sidecar:

```
garments/
  condo_knit_tank_top/
    condo_knit_tank_top.glb      cream sleeveless knit top (slot top, layer 1)
    garment.json
  condo_charcoal_trousers/
    condo_charcoal_trousers.glb  charcoal trousers (slot bottom, layer 0)
    garment.json
```

These are the two meshes that `build_outfit.py` appended to
`../beautiful_assistant.condo-outfit-v13.glb`, split out by
`scripts/condo/split_outfit.py`. They are not re-exported. Positions, normals,
UVs, weights, indices, inverse bind matrices, materials and the knit PNGs are
copied byte for byte. Only `JOINTS_0` is re-encoded, because each garment's skin
lists just the joints it uses. The outfit GLB is unchanged and remains the
current review asset. The renderer does not load these files yet.

## What a garment GLB contains

- One mesh node holding the garment mesh: one primitive, `POSITION`, `NORMAL`,
  `JOINTS_0`, `WEIGHTS_0` and, for the top only, `TEXCOORD_0`. Neither garment
  has `TANGENT` or morph targets. Influences are 4 per vertex.
- One material. It is double-sided PBR. The top has the knit base colour and
  normal textures; the trousers are a flat charcoal colour with no textures.
- One skin. It lists the CC5 joints that `JOINTS_0` references (19 for the top,
  16 for the trousers) in the outfit skin's order. They keep their names and
  their inverse bind matrices from the 78-joint `CC_Base_Body` skin.
- The CC5 bone nodes from `RootNode` down to those joints, with their rest
  transforms unchanged. This includes bones that carry no weight, such as
  `CC_Base_Hip`, `CC_Base_L_Thigh` and `CC_Base_L_Upperarm`. The nodes are
  there so the skin is valid glTF and its bind pose can be computed. A loader
  binding to the avatar should not spawn them as a second skeleton.
- No body, hair, eye or other avatar meshes, and no animations.

Two deliberate differences from the outfit:

- `skin.skeleton` is `CC_Base_Hip`, the joints' actual common root. The source
  skin names `CC_Base_Pelvis`, but `CC_Base_Waist` is a sibling of the pelvis,
  so that value fails glTF validation. `skeleton` does not affect skinning.
- Vertex-attribute and index buffer views have a `target`.

The garment node keeps the outfit's parent (`RootNode`) and its -90° X
rotation. glTF ignores a skinned mesh node's own transform, and Bevy's skinning
uses only joint matrices. The Khronos validator reports
`NODE_SKINNED_MESH_NON_ROOT` and `NODE_SKINNED_MESH_LOCAL_TRANSFORMS` as
warnings, the same warnings it reports on the outfit.

## `garment.json`

| Field | Meaning |
|---|---|
| `schema` | `cc-render.garment/0`. Bump it when a field changes meaning. |
| `id`, `display_name` | Stable id, which is also the directory and GLB name, and a human-readable name. |
| `slot` | Exclusive body region: `top` or `bottom`. Two worn garments must not share a slot. |
| `layer` | Stacking ordinal; a higher layer is worn over a lower one. The top's hem overlaps the trouser waistband, and `check_garments.py` measures it at least 11.8 mm outside the trousers in all 324 sampled bins of that band. So the top is 1 and the trousers 0. |
| `asset` | GLB file name, size and SHA-256. |
| `binding` | `skeleton: CC5`, `bind_by: joint_name`, the ordered skin joint names, the SHA-256 of those names joined with `\n`, the maximum influences, and the bind pose they assume. |
| `geometry` | Vertex and triangle counts, the attribute names, and the morph target count. |
| `materials` | Material name, double-sidedness and the texture slots it uses. |
| `deformation` | `static skinned, no simulation`. |
| `provenance` | Path and SHA-256 of the source outfit GLB, SHA-256 of the clothing-free source avatar, the source node, mesh and skin indices, and the scripts that authored and extracted the garment. |

The sidecar has no physics, body-hide mask, cage or fabric fields yet. Add them
when they are measured or authored, not as placeholders.

## Rebuild and verify

```sh
python scripts/condo/split_outfit.py \
  scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb \
  scenes/toronto_condo/garments

python scripts/condo/check_garments.py \
  ../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb \
  scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb \
  scenes/toronto_condo/garments [report.json] [--khronos-validator DIR]
```

The splitter is deterministic. A re-run with identical output is a no-op, and
it refuses to replace differing files without `--overwrite`. The audit checks
the following, and `tests/test_condo_garments.py` runs it with negative
controls:

- GLB and glTF structure.
- Every attribute is byte-identical to the outfit's garment, and `JOINTS_0` is
  identical once both files' indices are mapped to joint names.
- Joint names, inverse bind matrices and bone rest transforms match both the
  outfit and the clothing-free avatar.
- Bind-pose skinned world positions match the outfit's (tolerance 1e-6 m; the
  measured error is 0).
- Every JSON entry and BIN byte the outfit added is carried by a garment.
- The sidecar records and layer order match the files and geometry.
- Re-running the splitter reproduces the committed GLBs.

`--khronos-validator DIR` also runs `DIR/node_modules/gltf-validator` (from
`npm install gltf-validator`) and fails on any validator error.

## What a runtime loader needs (not implemented)

- Load the garment GLB's mesh, material and inverse bind matrices without
  spawning its bone nodes. Alternatively, spawn its scene and then replace its
  skin.
- Build a `SkinnedMesh` whose `joints` are the avatar's existing bone entities,
  found by `Name` for each entry in `binding.joints`, in that order. Reuse the
  garment's inverse bind matrices. Reject the garment if a name is missing.
  Check `binding.joints_sha256` and `asset.sha256` before trusting the record.
- Spawn the mesh entity under the avatar's `AvatarRoot`. Skinning ignores the
  mesh's own transform. Under that root, the existing systems treat it like the
  body's skins: `normalize_avatar_skin_weights` in `bevy_app/skinning.rs` and
  `keep_avatar_skins_visible` (`NoFrustumCulling`) in `bevy_app/visibility.rs`.
  A garment spawned elsewhere gets neither, and its rest-pose bounds can cull
  it while it is on screen.
- Enforce one garment per `slot`. Use `layer` for future push-out and hide-mask
  ordering. Equipping a garment must not touch the pose writer, retargeting or
  calibration.
- Before the outfit GLB can be retired, the scene-conditioning profile
  (`calibration-v13`) must be rebuilt from avatar plus garments, because it
  currently hashes the combined outfit GLB.
