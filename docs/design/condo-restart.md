# Condo restart checkpoint

Latest: checkpoint 188 adds the passive balcony slider in
`scene-balcony-slider.json`. All 326 library tests pass (two ignored), and the
six capture-driver tests pass. Gallery entries 188a–c retain the failed replay
attempt, playable slider physics and corrected live integration. Live replay now
paces packet delivery against the observed motion clock, avoiding overflow when
rendering falls behind wall time. The corrected capture passes unchanged root,
cloth and contact audits. Bathroom leaves and actual avatar manipulation remain
unfinished; keep the original broader waterfront-view goal active.

Checkpoint 187 adds the passive bedroom door in `scene-doors.json`.
Its full-condo route test distinguishes closed/open passage without changing
avatar motion. Physics, clearance and preview evidence is in `door187/` and
the gallery. Bathroom/balcony leaves and actual avatar manipulation remain open.

Gallery playback follow-up: `preview_drawer_trace.py --animation` exports a
complete recorded 60 Hz mechanism trace to `view.mp4` at 30 fps, preserving
physical time. The bedroom recording is in `door187/animation/`. It shows
closed, blocked opening, full opening, release, closing and settled closed
states over 18 seconds. This is an isolated physics demonstration, not avatar
handling. Gallery video players now appear above the still images.

Resumed on 2026-10-17. Checkpoint 186 is now implemented in
`scene-cabinet.json`: the hollow entry cabinet and passive doors are integrated
and tested, with recorded-physics renders and a live entry-camera capture.
See `condo-manipulation.md` checkpoint 186 and the progress gallery. The remaining
text below records the original pre-restart checkpoint, not the latest status.

Stopped at the user's request before a PC restart. Resume only when requested.

The priority is manipulable furniture and bed sheets; cosmetic work is paused.
The broader waterfront condo scene objective remains unfinished.

## Saved state

- Latest opt-in scene: `scenes/toronto_condo/scene-drawers.json`.
- Passive fridge hinges, two bedside drawers with 40 cm travel, and a live
  deformable top sheet are integrated. Actual avatar manipulation is unverified.
- Use `beautiful_assistant.condo-outfit-v13.glb` and the adjacent SDK's
  `avatars/beautiful_assistant.arkit_map.resolved.json` together.
- Source, scripts, manifests, Blender authoring assets and scene assets are
  saved in the checkpoint commit. Generated capture history remains on disk
  under `artifacts/`; it is not included in this source/assets commit.
- Open `artifacts/toronto-condo/index.html` for screenshot/video progress.
  Latest entries are 185 (drawer physics) and 185b (bedroom integration).
- Evidence: `artifacts/toronto-condo-captures/drawer185/`. Its final library
  run passes 319 tests, two ignored. Python has six passes and one existing
  ownership source-text failure; the quality gate retains 23 existing major
  findings. No ownership test was weakened.
- The bedroom recording has 120 frames, 1,303 cloth ticks, matching mesh/solver
  hashes, zero observed particle penetration and a passing root-fidelity audit.
  All 30 asset hashes and both gallery entries' 40 archived file hashes match.

## Resume point

The entry cabinet was inspected as the next passive mechanism: replace its solid
case with a hollow shelved shell and two outward-opening doors, retaining stable
IDs and real handle geometry. **No entry cabinet implementation was started.**
The only checkpoint 186 output is a feature-map log.

Read `AGENTS.md`, `docs/design/model-owned-motion.md`, and
`docs/design/condo-manipulation.md` before continuing. Reuse the existing passive
hinge runtime and Blender assembly/panel helpers. Check swept clearance against
the bench and walls, obstruction, release, current bounds and navigation before
publishing recorded-physics and live integration evidence.

Generated finger input, actual avatar grip admission, moving-body/cloth
collision and demonstrated avatar manipulation remain unfinished. Diagnostic
forces and recorded physics renders do not prove avatar handling. The scene is
not yet certified as polished or complete.
