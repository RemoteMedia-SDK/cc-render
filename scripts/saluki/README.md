# Saluki (Call of Duty) characters → cc-render GLB

Converts a character exported by Saluki as SMPL-X (`exported_files/<game>/smplx/<name>/<name>_smplx.fbx`,
with the game maps under `exported_files/<game>/models/`) into a GLB cc-render can load and render with
the avatar editor's studio look and its CC5 skin shader.

1. `smplx_to_glb.py` (Blender 4.2+, headless): FBX → GLB.
   - Folds the game's facial rig (the bones that aren't SMPL-X, ~330 under head/jaw) into their nearest
     SMPL-X ancestor. Bevy 0.15 skins at most 256 joints, and skinning is unchanged for any pose that
     leaves those bones at rest.
   - Renames the 22 SMPL body joints (plus jaw and eyes) to the `CC_Base_*` names cc-render's bind
     capture looks for. This is a fixed name mapping, with no pose change.
   - Keeps albedo alpha only on real cut-outs.

   ```
   blender -b --factory-startup --python-exit-code 1 --python smplx_to_glb.py -- valen_smplx.fbx valen_smplx.glb
   ```

2. `glb_materials.py` (Python, numpy + Pillow): adds the CoD maps the FBX doesn't wire.
   - Normal map (DirectX → glTF).
   - ORM texture (AO, roughness = 1 − gloss, metal).
   - Opacity on hair cards, lashes, lace and chains.
   - `--cc5-skin` marks the skin materials (CoD's skin techset) as CC5 `RLSkin` (`extras.cc5`), so
     `src/cc_render/cc5_materials` renders them with `skin.wgsl`. That adds subsurface wrap, light
     through the ears and nose wings, and dual-lobe specular. Their SSS and transmission maps come from
     map 0x4e and are written to `<out>.cc5maps/`, next to the GLB.

   ```
   python -I glb_materials.py valen_smplx.glb valen.glb --models <exported_files/mw7/models> \
       --dirs rex_body_mp_valen_1_1,rex_head_mp_valen_1_1_high_fe --cc5-skin
   ```

   Without `--cc5-skin`, pass `--skin-gloss 0.7`. Bevy's StandardMaterial has no subsurface scattering,
   so at full gloss the skin reads oily.

The SMPL-X FBX itself comes from the operator pipeline in `smplx_pipeline/` (`run_operator.py <op>`; see its
README).

Facial expressions: the operator pipeline (`smplx_pipeline/scripts/cc5_transfer.py`) writes
`<op>_smplx_cc5.fbx`, whose `<op>_face` mesh carries the 335 CC5 expression shapes as shape keys.
Convert that FBX with `--arkit-map` to keep the shapes an ARKit map drives, and its correctives, as
glTF morph targets (cc-render's Bevy takes at most 256 per mesh). This also writes
`<out>.arkit_map.json`, with each morph's `meshes` set, and `glb_materials.py` carries it to its
output name. Use the CC5 HD map (`crates/cc5-shape/data/cc5_hd.arkit_map.json`, written by
`scripts/cc5_arkit_map.py`): smiles on CC5's lip corner puller and 93 of its correctives, which the
renderer drives at the product of their drivers' weights.

```
blender ... --python smplx_to_glb.py -- valen_smplx_cc5.fbx valen_cc5.glb \
    --arkit-map ../../crates/cc5-shape/data/cc5_hd.arkit_map.json
python -I glb_materials.py valen_cc5.glb avatars/valen_arkit.glb --models ... --dirs ... --cc5-skin
```

The pipeline's head-only rig (`run_operator.py <op> --head-only`, `smplx\<op>_head\`) converts the same
way: `smplx_to_glb.py` keeps its spine3 → neck → head → jaw / eyes (6 joints after folding), and
`glb_materials.py` needs only the head folder in `--dirs`. The runtime renderer takes it as a partial
rig: face and head are driven, and body poses are skipped. Its default camera frames a body, so set
`CC_AVATAR_CAMERA_POS` / `CC_AVATAR_CAMERA_LOOK_AT` (e.g. `0,1.62,0.7` / `0,1.6,0`) for `pose_snapshot`.
`studio_snapshot --view head` frames it on its own.

The operators have no `Mouth_Lips_Together_*`, so `mouthClose` is dropped. The renderer then
opens the jaw by the lip gap instead. `jawOpen` rotates `CC_Base_JawRoot` about the character's
side axis (eye to eye), which is head +X on these rigs.

Render it:

```
CC_AVATAR_ENVMAP_DIR=<folder with pisa_*_rgb9e5_zstd.ktx2> \
cargo run --release --example studio_snapshot -- valen.glb head.png --view head --size 1600x1600
```

What the map ids hold was worked out on Valen (mw7), and the docstring of `glb_materials.py` lists it.
Removable masks: `glb_materials.py` tags a mask over the face (balaclava, ski mask, hockey or
ballistic mask) with `extras.face_mask` on its materials. The runtime renderer then takes it off and
puts it back on a loaded avatar: `CcRenderer::set_face_mask(false)`, the node's
`{"kind":"face_mask","worn":false}`, or `pose_snapshot --no-mask`. Headwear, glasses and headsets
stay. A mask is gear that comes within 1.6 cm of the upper lip, or is named as one, plus the clips and
buckles the game toggles with it (`*_toggle_on`). Of the mw7 operators, 9 have a full face under
their mask (beta, blackwell_4_1, kofi_2_1, kofi_2_2, milsim_fr_army_sniper, milsim_ksk_amphibious,
price_vault, uctf, valeria_2_1). The masks of blix, ctsfo, ghost, hunter_killer, milsim_ksk_arctic,
milsim_us_sf, park_2_1 and reece hide a face that was never modelled.

Skin (three CoD skin techsets: the usual one, the cinematic faces of some `high_fe` heads, and
luca's and ohji's), eyes, tear lines and hair/brow/lash cards are found by techset on any character. Metal is still Valen's material
names, so other operators' jewellery and helmet hardware render non-metallic. Images come from the
folder the material's `Name:` gives (`material_*`, `m/…` or `ma/…`).
