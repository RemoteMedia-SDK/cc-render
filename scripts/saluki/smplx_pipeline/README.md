# CoD operator → SMPL-X (+ CC5 expressions)

Turns a Call of Duty (MW4 / `mw7` handler) operator exported by Saluki into an SMPL-X-rigged character with
the 335 CC5 facial expression shapes, ready for SMPL/AMASS body motion and CC5/ARKit face animation.

```
python scripts/inventory.py                # what's exported: body/head pairing, face rig, problems
python run_operator.py <operator>          # e.g. valen, jae, blackwell_4_1 — all steps
python run_operator.py jae --from cc5      # rerun from a step
python run_operator.py jae --steps poses   # just one step
python run_operator.py --all --skip-done   # every exported operator; continues past failures
python run_operator.py valen --head-only   # the head alone (no body) -> smplx\valen_head\
python scripts/batch_report.py            # smplx\_report.md (all numbers, outliers flagged) + _review.png
```

Operator names are the body id without a `_1_1` skin (`valen`, `jae`, `blackwell_4_1`, `kofi_2_2`).
Body and head come from `scripts/inventory.py`; head names vary (`_high_fe`, `_lod0_fe`, `_fe_high`, `_high`,
sometimes without the skin number, e.g. `ohji_high_fe` for body `ohji_3_1`), so it ranks candidates:
same skin with the full face rig (~356 bones) > skin-less full rig (`blix_high_fe`) > same skin with the
simplified rig (~146 bones, often masked faces) > skin-less > another skin's head (flagged).
Heads with **no face rig** (gas masks, full helmets: only `j_head` + neck) get the body rig and mocap; jaw and
eye joints are placed at average offsets and the face steps are skipped. `--all` writes
`smplx\_batch_summary.json`; `scripts/inventory.py --json` writes the pairing.

Outputs land in `C:\Users\mail\Downloads\saluki\exported_files\mw7\smplx\<operator>\`; check renders,
`summary.json` (every step's numbers) and logs in its `checks\` folder.

## Before running: get a complete export

1. **Cordycep** (`mw7`): `loadcommonfiles` first, then the operator packs
   (`loadlist OperatorModels.txt`, or `loadwc mp_*_rex_*_mp_<name>_*`). Without the common files ~17% of
   materials (mouth interior, eyelashes, buckles, radios…) export as empty stubs.
2. **Saluki**: export `rex_body_mp_<op>_1_1` and `rex_head_mp_<op>_1_1_high_fe` (the `_fe`, `_base` and arms
   variants are optional). Saluki **skips models whose folder already exists**, so to re-export rename the old
   folders (e.g. to `*_old`) first.
3. The `audit` step refuses to continue if anything is missing or untextured (`--force` overrides).

Other body/head names: `--body rex_body_mp_<x> --head rex_head_mp_<y>`.

## Steps

| step | what it does | output |
|---|---|---|
| audit | export complete? (bones, materials, images) | — |
| import | Cast import of body + high_fe head (LOD0) | `<op>_cod_import.blend` |
| rig | head snapped onto body (j_spine4); cm→m, faces −Y, Z-up; 55-joint SMPL-X skeleton from the CoD joints; CoD weights folded into SMPL-X joints; ~330 face bones kept under head/jaw; T-pose rest shape by **inverse skinning**, so the rig posed back to the game's A-pose reproduces the game mesh exactly | `<op>_smplx.blend/.fbx`, `checks/rig/` |
| poses | that round trip (joint offsets, mesh error) + 10 standing poses, skeleton overlay | `checks/<op>_poses_*.png`, `<op>_shoulder_*.png` |
| mocap | AMASS sample (SMPL-H, seated) on the rig, feet on floor, limb directions vs mocap markers | `<op>_smplx_amass.blend/.fbx`, `<op>_amass_sample.mp4` |
| face | straight-on face render + MediaPipe Face Landmarker (478 points) | `checks/face/` |
| cc5 | CC5 expressions + hinged-jaw mouth weights (below) | `<op>_smplx_cc5.blend/.fbx` |
| cc5_sheet | 13 expressions on the operator next to the CC5 source | `checks/<op>_cc5_expressions.png` |

## Head only (`--head-only`)

Rigs just the head export, without a body, into `smplx\<operator>_head\` (files named `<operator>_head_*`); the
full-body outputs are untouched. Steps: audit, import, rig, face, cc5, cc5_sheet (no poses or mocap).
`scripts/op_head_only.py` places the head where it sits on a standing operator (head skeleton root `j_spine4` on
Valen's body `j_spine4`, bind pose), so heights and face orientation match the full rigs, then builds
`spine3 → neck → head → jaw / eyes` plus the same ~330 face bones as the full rig. Neck weights fold into `neck`;
the clavicle/deltoid helpers along the cut edge fold into `spine3`. No T-pose or inverse skinning is needed, since
the head export is already in its bind pose. The face steps run unchanged. Heads exported without a body
(carpenter, hero_kwon) work too: the head is found as the `rex_head_mp_<operator>*` folder with the biggest face
rig, or pass `--head`. Heads without a face rig are refused. One operator per run (`--all` is not supported).
The joint maps, face-bone and weight-fold code shared with the full rig live in `scripts/cod_rig.py`.

## CC5 expression transfer (`cc5`)

Source: CC5 character "Beautiful" (`cc5_source/character.fbx`, 349 body + 29 tongue shapes).
0. The face render for landmarks shows only face skin + eyeballs (glasses, beards, hair, lashes, helmets hidden),
   like the CC5 render. Face skin = the material most weighted to the nose bones (lip bones also carry beards).
   If MediaPipe finds no face it retries untextured grey (dense tattoos, a lone eye strip under a balaclava),
   then with every material. Landmark rays count only on camera-facing skin/eyes no deeper than 5 cm behind
   the eyes (holes in the face skin otherwise hit the back of the head). Masked faces with no lip skin skip
   the lip-seam split; the hinge plane splits the whole mouth.
1. Landmarks → 3D by casting the render rays onto each head; similarity fit, RBF warp through the pairs,
   smoothed snap onto the operator skin (landmark error after warp ≈ 0.02 mm median).
2. Each operator face vertex blends the CC5 shape motion of nearby fitted-CC5 vertices (Gaussian, radius
   follows the CC5 vertex spacing; a single closest point creases at nose/lip folds). Verts > 1 cm from CC5
   (mouth interior) are filled from mesh neighbours. Near the lips, each side samples only its own lip.
   Eyes, tongue, gums and teeth are left out by their bones, but skin on the game's gum bones (`j_insidegum_*`
   carries the inner lip corners) is kept: left still, it creased every shape that moves the corners.
   Head-weighted vertices take shapes only on the skin, so earrings and hair near the face don't bend.
3. **Mouth weights for a hinged jaw** (cc-render folds face bones into head/jaw and rotates the jaw): the game
   face is posed bone-by-bone and tears under a plain jaw rotation, so around the mouth the head/jaw split is
   rebuilt — lip skin split exactly at the seam by surface distance to the upper/lower lip centres (the lips
   only connect at the corners), mouth lining split by the plane through the jaw hinge and lip line; CC5's
   head/jaw split further out and within ~1 cm of each lip corner, where a seam split stretched the open
   mouth's sides into vertical walls (the lining's split widens from 1 cm before the corners too).
   cc-render turns the jaw by `cc5_sheet`'s `jaw_deg_matching_cc5_28deg` (Valen 23.6°), which
   `../smplx_to_glb.py` writes into the GLB's ARKit map.
4. The moving face region is split into `<op>_face` (holds the shape keys; border delta 0), the rest stays `<op>`.
   Shape values are saved at 0.

The plain `<op>_smplx.*` files keep the game face bones and weights untouched.

## Notes for downstream (cc-render)

- **Jaw angle**: the SMPL-X `jaw` joint is at the jaw hinge, ~11 cm behind the lips; CC5's `CC_Base_JawRoot`
  sits near the mouth. The same lip opening as CC5 at 28° is ≈ 23.6° here (`checks/summary.json` →
  `cc5_sheet.jaw_deg_matching_cc5_28deg`). cc-render's default 45° gives a very wide gape.
- `<op>_smplx_cc5.fbx` has **two meshes** (`<op>` and `<op>_face`); `../smplx_to_glb.py --arkit-map` converts
  both and keeps the face's shape keys as morph targets (see `../README.md`).
- No blendshapes exist in the game exports — the CC5 shapes are transferred, not extracted.

## Folder

- `run_operator.py` — the driver; `scripts/config.py` — every path in one place.
- `scripts/` — Blender steps (`op_*.py`, `face_front_render.py`, `cc5_*.py`), `cod_rig.py` (CoD → SMPL-X joint
  maps, face bones, weight folding; shared by `op_smplx.py` and `op_head_only.py`) and helpers (`face_landmarks.py`
  runs in `deps/mpenv`; `mat_audit.py`, `castbones.py`, `glb_info.py`, `pose_sheets.py`, `expr_montage.py`).
- Data, not in the repo (~400 MB): `deps/` and `cc5_source/` live under `SMPLX_PIPELINE_DATA` (default
  `C:\Users\mail\Downloads\saluki\smplx_pipeline`; see `scripts/config.py`).
  - `deps/` — Cast add-on v2.01 (loaded from here, not installed in Blender), AMASS sample clip, MediaPipe model,
    `mpenv` (Python venv with mediapipe 1.2.0; recreated automatically by the `face` step if deleted).
  - `cc5_source/` — the CC5 FBX + textures and its cached landmarks (`landmarks/cc5.png*`; delete to recompute).

Requires Blender 5.2 (`scripts/config.py: BLENDER`), ffmpeg on PATH, and a Python with numpy + Pillow to run
the driver.
