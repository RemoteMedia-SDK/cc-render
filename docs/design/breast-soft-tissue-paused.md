# Paused tissue prototype checkpoint

Paused on 2026-10-17 at the user's request. Branch: `feat/breast-soft-tissue` in
the isolated `cc-render-soft-tissue` worktree, based on `ea65ebb68`.
This is a preservation commit, not a completed physics implementation or merge
recommendation. The user's latest visual assessment was **zero visible bounce**.
Realistic whole-breast deformation has not been demonstrated or accepted.

## Saved implementation

The opt-in runtime replaces breast-bone springs with tetrahedral soft tissue,
implicit integration, separate shear/bulk viscosity, mesh transfer and limited
body collision. A focused Bevy lab replays unchanged ARDY motion on the base
`beautiful_assistant.smpl_aligned.glb`; clothing and condo geometry are absent.
ARDY retains sole ownership of body/root motion. See the
[design record](breast-soft-tissue.md) and [history](breast-soft-tissue-history.md).

The latest candidate is
`assets/tissue/beautiful_assistant.base-skin-volume-v2-prototype.json`.
`scripts/avatars/build_skin_tissue_volume.py` builds an anterior shell from the
source skin with a posterior attachment cap and chest collar, tetrahedralizes it,
and embeds the selected skin without multiplying motion by breast-bone weights.
Morph-sensitive shoulder vertices and their normal stencil are excluded.
Left/right volumes have 241/233 nodes, 657/605 tetrahedra and 130/128 fixed nodes.
The cap, collar, attachment and material parameters are engineering approximations,
not anatomical calibration. Near-incompressible tetrahedral locking is an
unverified possible contributor to stiffness; no locking remedy was implemented.

## Latest evidence

- [Tracked physics-off/on comparison](../../artifacts/soft-tissue/skin-volume/tracked-off-on-v2.mp4).
  Identical torso-tracked crops, original timing and motion, no deformation
  amplitude multiplier. Encoding and all 478 timestamps were validated. This
  newly encoded comparison was not visually accepted before the pause.
- [Full front capture](../../artifacts/soft-tissue/skin-volume/skin-volume-front-v2.mp4).
- [Launch and source hashes](../../artifacts/soft-tissue/skin-volume/front-v2-launch.json),
  [video validation](../../artifacts/soft-tissue/skin-volume/comparison-video-validation.json)
  and [crop provenance](../../artifacts/soft-tissue/skin-volume/tracked-crop.json).
- Captures and traces: `artifacts/soft-tissue/lab-skin-volume-front-02/`.
  The run completed 478 frames. Peak reported skin displacement was 15.04 mm;
  at the 7633 ms snapshot, area-weighted displacement was 2.47 mm, maximum 8.22 mm.
  These measurements do not establish convincing visible bounce.

Earlier geometric-transfer experiments distorted the attachment rim. Fixing the
whole old rim made motion negligible. The first skin-volume capture failed on a
morph-overlap guard; its logs and failed shell assets are retained. None of these
attempts should be presented as an accepted result. Older review pages still show
earlier candidates; use the links above for the latest checkpoint.

## Verification and open issues

- Last focused Rust run: 67 passed, recorded in
  `artifacts/soft-tissue/solver-runtime/viscosity-split-all-tissue-tests.log`.
  Runtime solver code was unchanged by the subsequent skin-volume authoring work.
- Last Python suite: 16 tests, 15 passed, one existing ownership-test failure
  (`test_root_acceptance_does_not_rewrite_the_candidate`); its source slice matches
  an unrelated `for ` beyond the function under test. Protected tests were not
  changed. All four new skin-volume authoring tests passed. Logs:
  `artifacts/soft-tissue/skin-volume/python-tests-02.log`.
- A historical broad Rust run was interrupted (exit 143), not a full-suite pass.
- Precommit structural review is retained under `artifacts/soft-tissue/checkpoint/`.
  Quality checks still report gating findings; no blanket acknowledgement or
  cleanup refactor was made while paused.
- Visible responsiveness, material/attachment calibration and live performance
  remain unresolved. The jump registered no tissue contacts, so compression and
  contact appearance are unvalidated. Collision lacks edge-edge, mutual-breast
  and friction coverage. The facial-update policy while body time is paused also
  remains unresolved. This prototype is not enabled by default.

## Preservation and reproduction

Necessary task-specific `/tmp` sources, assets and logs have verified copies in
`artifacts/soft-tissue/preserved-tmp/`. Its `manifest.json` maps original paths to
durable paths and SHA-256 hashes. Rebuildable ELF executables and matplotlib
caches are excluded; no required original source remains available only in `/tmp`.
Historical logs and source snapshots retain their original paths as provenance.
The historical profiler now reads its input from the preserved directory and
puts regenerable output in `target/tissue-profile/`.

Code, authored assets, scripts, reports, failed-attempt logs, source archives,
the latest two videos and selected motion/traces are committed. Approximately
15 GB of raw captures and older trajectories remain locally under
`artifacts/soft-tissue/`, excluded from Git by its `.gitignore`. The checkpoint
inventory lists those files and sizes; exclusion does not delete them. Build
outputs remain in ignored `target/`. The original avatar GLB and ARKit map remain
in the sibling `remotemedia-sdk-office-main/avatars/` directory and are external
inputs, with hashes documented in the design record.

Use the exact arguments/environment in `skin-volume/front-v2-launch.json` for the
latest run, choosing a fresh output directory to preserve old attempts. Source
motion is `lab-jump-01/packets.ndjson`; authoring inputs, build logs and archived
fitter sources are in `skin-volume/`. New work requires an explicit resume.
