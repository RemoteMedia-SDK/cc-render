# Condo restart checkpoint

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
