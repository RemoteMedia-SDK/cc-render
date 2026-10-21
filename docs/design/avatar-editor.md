# Avatar shape editor (CC5 sliders outside CC5)

`examples/avatar_editor.rs` reshapes a Character Creator 5 character with CC5's own
Morphs-panel sliders (body type, height, face features — about 3,400 for a CC3+/HD
character) without CC5. `examples/shape_avatar.rs` does the same headless from a
weights JSON. Both write a shaped GLB for the normal avatar pipeline.

## How it works

CC5 is used once, unattended, to record what every slider does to *this*
character (`remotemedia-sdk/scripts/avatars/cc5_capture_sliders.py`):

- **bind mode**: per slider state, the displayed bind-pose mesh from CC5's
  subdivision-0 OBJ export. This already contains the skeletal stretch — many
  sliders act through the skeleton (Character Height moves CC5's internal mesh
  <1 cm but the bones ~12 cm).
- **capture mode**: per slider state, skin-bone positions (CC5 live pose). The
  pack builder maps them into the bind pose (`bind_joints.py`: chain rebuilt
  with per-bone pose rotations, grounding anchored on rigidly skinned joints).

`build_editor_pack.py` turns that into `pack.json` + `pack.bin`
([`cc5_shape::ShapePack`](../../crates/cc5-shape/src/pack.rs); the
[`cc5-shape`](../../crates/cc5-shape/README.md) crate is also a Python module,
`import cc5_shape`): sparse vertex and joint
deltas in the glTF frame. The reference GLB (CC5's FBX export converted by
`cc5-fbx`, ufbx) carries a `_CCIDX` vertex attribute, the CC5 vertex
index of every glTF vertex, so seam-split vertices map back exactly.

A slider mix is `bind + Σ tᵢ·Δᵢ` for vertices and joints, `tᵢ` linear on each
side of the slider's captured value. On CC5's internal (pre-skinning) mesh,
sliders are exactly linear and additive (0 error for a half weight and for two
sliders combined); for sliders that move bones, the bind-pose shape includes
skinning, so mixes are a close linear approximation there.
[`shape_glb`](../../crates/cc5-shape/src/glb.rs) writes it into a GLB: positions,
joint translations (rotations unchanged), and inverse bind matrices
`IBM' = W'⁻¹·W·IBM` so the new bind mesh renders exactly at bind.

Normals follow the shape ([`cc5_shape::normals`](../../crates/cc5-shape/src/normals.rs),
in `shape_glb` and in the editor, which also covers expressions): per vertex, the
rotation from the bind mesh's smooth normal to the shaped mesh's turns CC5's
authored normal (and tangent). Smooth normals are area-weighted and welded by
CC5 vertex, so seams stay closed; where the surface did not move the normal is
kept bit for bit. CC5's own normals are these smooth normals (Beautiful's body:
median 0.00°, p99 0.05° apart), so the turned normals are what the shaped
surface has. Without this, a large reshape (e.g. the neutral male base on a
female capture) kept the capture's shading: on the fitted male chest the old
normals were 24° off the surface (median; up to 88°) and rendered breasts on a
flat chest.

## Building the inputs

On the Windows machine with CC5 (5.13), Python and cc-render's `cc5-fbx` built
(`cargo build -p cc5-fbx --release --target-dir target-cc5`; no Blender):

```powershell
remotemedia-sdk\scripts\avatars\cc5_editor\build_cc5_editor_pack.ps1 `
    -Project C:\Users\me\Documents\avatars\Beautiful.ccProject -OutDir D:\cc5_editor\beautiful
```

It drives CC5 with no clicks (loads CC5's slider library via "Load CC5 HD
Neutral Base" — without that step CC5 exposes no head/body sliders to Python —
opens the project, captures, exports the reference FBX), converts it with `cc5-fbx`,
bakes hair/brow alpha (`bake_alpha.py`) and builds the pack. The first run takes
hours (bind capture ≈ 3.5 s per slider state); reruns resume. Any running CC5
is closed first; the project is never saved.

## Running

```
cargo run --example avatar_editor -- D:\cc5_editor\beautiful\character.glb D:\cc5_editor\beautiful\pack [shaped.glb]
cargo run --example shape_avatar  -- character.glb pack weights.json shaped.glb
```

With no `character.glb pack_dir`, the editor opens `$CC5_EDITOR_DIR` (default
`D:\cc5_editor\beautiful_e2e`): its `character.glb` and `pack_xl` (else `pack`).

Editor: category tree with search, per-slider reset, "Reset all", "Save GLB";
with `--expressions <dir>`, an "Expressions" section (see below).
Right-drag orbits, middle-drag pans, wheel zooms. `weights.json` maps slider
names or ids to values (`{"Character Height": 0.5}`).

The editor starts from, and "Reset all" returns to, its base body: by default
CC5's neutral female when the pack has it (`pack_xl`, where `avatar_slider.py`
adds the neutral bases as `avatar_neutral_f` / `_m` sliders), else the captured
character. `--base none | neutral_f | neutral_m | <slider> | <recipe.json>`
chooses another. Changed sliders are counted against the base, and saved recipes
keep it apart from the changes: `{"base": {...}, "sliders": {...}}` (loading
applies both, so a recipe means the same body in any session).

## Accuracy (Beautiful / HD Ariana, Character Height = 1 vs CC5's own FBX export)

| | error |
|---|---|
| bind mesh, 67,419 vertices | 0.314 cm, uniform (CC5's OBJ and FBX exports ground the character 3 mm apart); shape itself exact |
| joints | median 0.35 cm, max 0.79 cm (mostly the same 3 mm shift; fingers worst) |
| skin consistency `W·IBM` | 1e-6 |

For scale, the slider moves vertices up to 12.3 cm and joints up to 11.8 cm.

## Facial expressions (CPU morphing)

A CC5 HD character carries ~400 facial morph targets per face mesh (body 349,
brows/lashes 395, tear line 437...). Bevy 0.15 refuses meshes with more than 64
(`MAX_MORPH_WEIGHTS`), so the reference GLB has none (`cc5-fbx` drops them)
and the expressions live in a sidecar *expression pack*, applied on the CPU like
the shaping sliders:

```powershell
cc5-fbx expressions D:\cc5_editor\beautiful\ref\character.fbx D:\cc5_editor\beautiful\pack D:\cc5_editor\beautiful\expressions
cargo run --example avatar_editor -- character.glb pack --expressions expressions
```

`expressions.json` + `expressions.bin` ([`cc5_shape::ExpressionPack`](../../crates/cc5-shape/src/expr.rs)):
per shape-key name, the sparse delta `key − Basis` at weight 1, merged over every
mesh that has that key, in the slider pack's global vertex space and glTF frame
(the FBX's vertex order is CC5's). Beautiful: 480 expressions (56 empty
correctives, e.g. `Mouth_Lips_Together_*`, which CC5 exports as zero), 785k
vertex deltas, 12.6 MB. A mix is `bind + shape Δ + Σ wᵢ·exprᵢ`; morphing is
linear, so this is exact — against Blender evaluating the FBX's shape keys
(one key at 1, one at 0.5, two 3-key mixes across body, lashes, brows, tear
line, eye occlusion, eyes and tongue; up to 29 mm of motion) the max error is
1.2e-4 mm.

The editor's "Expressions" section (0..1 sliders, shares the search box) layers
expressions on the shaped positions in `apply_shape`; an expression-only change
rewrites positions without touching joints. Expressions are a preview there:
"Save GLB" and recipes hold the shape only. Only morphs are covered: CC5's jaw
and eye *bones* (Jaw_Open also rotates CC_Base_JawRoot in CC5) are not part of the
pack, exactly as with glTF morph targets. `cc5_shape::ArkitMap` resolves ARKit-52
weights (Audio2Face) through a runtime `*.arkit_map*.json` (`{"mapping":
{arkitName: [{morph, weight, meshes}]}}`; `meshes` is ignored since an expression
spans all meshes).

### Runtime: meshes with more than 64 morphs (design, not implemented)

cc-render's runtime (`src/cc_render/bevy_app`) drives morph weights per frame
from Audio2Face ARKit weights. For a character whose GLB exceeds 64 morphs per
mesh it would switch that mesh to the CPU path:

1. Load the GLB without morphs (as now) and the character's expression pack +
   ARKit map. At spawn, record per primitive what the editor records: base
   positions, `_CCIDX` globals, and the bind-space rotation (`MeshRec`).
2. Per frame: ARKit weights → `ArkitMap::weights` → `ExpressionPack::add_to`
   into a reused per-character buffer (zero only the vertices touched last frame),
   then rewrite `ATTRIBUTE_POSITION` of the affected primitives only (face, brows,
   lashes, tear line, occlusion, eyes, tongue — not body-wide). Cost is linear in
   the active expressions' vertex counts (typically 20–50 active ARKit targets,
   a few ×10k vertex adds) plus the position upload (~15k body vertices ≈ 180 KB),
   well within a frame. Skip the write when weights are unchanged.
3. Normals: morph targets in the GLB carry normal deltas; the CPU path would
   turn the normals of the touched region with `cc5_shape::normals`, as the
   editor does. Bone-driven parts (jaw, eyes) stay on the skeleton as today.
4. Keep the GPU morph path for characters with ≤ 64 targets; pick per mesh at
   load (count targets in the glTF, or the presence of an expression pack).

Alternatives considered: splitting a mesh into ≤64-target primitives breaks
along seams and still caps per primitive; a custom storage-buffer morph shader
avoids the CPU upload but needs a forked skinning/morph pipeline in Bevy 0.15.

## Skin (CC5's skin settings and SkinGen, outside CC5)

The editor's **Skin** section (`examples/avatar_editor/skin.rs`) customizes the
character's skin the ways CC5 does:

- **Material settings**: CC5's own RLHead / RLSkin variables (names, ranges and
  groups of `Program/Assets/Share/Shaders/Human/RLHead.shader` / `RLSkin.shader`):
  colour (brightness, saturation, human colour: red / yellow / black / range, blend
  map), roughness per region, specular, subsurface, micro normal. Edits rebuild
  the skin uniform live (`Cc5Skin::set_vars`); `skin.wgsl` implements them.
- **Skins**: whole skin presets (CC5's Skin Base / Full Skin / HD skins): their
  textures and settings swap in.
- **SkinGen layers**: makeup (lips, eyes, blush, contour, foundation), blemishes,
  freckles, moles, stubble, scars, tattoos, ... laid over the diffuse in a
  background task (`cc5_skin::compose`), each at a strength and optionally recoloured.

Skins and layers come from a **skin pack** (`<character dir>/skin`, `--skin-pack`),
captured from CC5 itself — its presets are encrypted:

1. `remotemedia-sdk/scripts/avatars/cc5_editor/skin/cc5_skin_capture.py` (inside
   CC5, `run_capture.ps1` restarts CC5 per variant) applies each preset to the
   character, bakes the SkinGen layers (CC5's "Bake All Skin Textures"; they
   otherwise exist only on the GPU) and saves the textures, renders a face view
   and records the camera, lights and IBL (`scene.json`).
2. `cc5-skin build <capture> <pack> <spec.json>` solves every layer's colour and
   opacity per texel from the layer on two bases: the character, and the
   character saved with a contrasting skin (CC5's bake composites onto SkinGen's
   own base skin, so a flat diffuse in the material never reaches it). Each
   SkinGen blend mode is fitted — normal B·(1−α) + C·α (makeup colour), multiply
   (tattoos, contour, tans, beards, freckles), screen (highlights) — and the best
   kept per layer, with parts in other modes where a preset mixes them (full
   makeup's lipstick multiplies). Layers reproduce CC5's bakes to ~0.15 levels
   (0..255); free CC5 caps SkinGen at 15 layers, so a preset over the cap on the
   second skin is skipped.

Recipes (`--skin recipe.json`, Save / Load, snapshot jobs' `skin`) keep the skin,
the layers and the settings that differ from the character.

**Matching CC5's look.** `--cc5-scene scene.json --cc5-lights` renders with CC5's
camera (its 36x24 mm film gate: render at width x width/1.5), its lights (rect
lights without inverse-square falloff, soft tube lights) and its IBL prefiltered
in memory, so renders compare pixel for pixel with CC5's (`--hide-hair` as CC5
hides hair while editing SkinGen). Calibrated on a flat grey skin, then on the
character's own skin; the skin shader's constants are `SkinTune`. Findings:
CC5's skin normal maps are DirectX-style (flipped for glTF's tangents), and its
response to colour changes is ACES-like: with ACES tonemapping (+0.9 EV,
contrast-adaptive sharpening 0.15) our renders of CC5's own bakes match its
renders of 8 skins and 25 SkinGen layers at mean dE 4.4, the layers' change from
the base within 0.6 dE (AgX with a saturation boost: 5.1 and 0.9).

## Limits

- The reference GLB drops CC5's ~400 facial morph targets (Bevy loads at most
  64); they are applied on the CPU from the expression pack instead (above).
- Joint *rotations* are not captured; sliders are applied as joint translations.
- Normals are turned with the surface, not re-authored: a twist of the surface
  about its own normal does not turn tangents, and hard edges in CC5's normals
  (rare on the body) are carried as they are. Skin textures are the captured
  character's (e.g. a female capture's areolae on a neutral male body).
- The pack is specific to the character it was captured from.
- The deltas are derived from Reallusion's slider content packs; check the
  Reallusion content licence before distributing packs or shaped characters.
