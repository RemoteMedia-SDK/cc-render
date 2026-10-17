# Condo restart checkpoint

Latest scope (2026-10-17): the owner replaced the broader goal with an active
**physics and movable condo parts** goal. Do not resume avatar
generation, interaction routing or calibration as part of this physics checkpoint.
The owner assigned handle grip zones and the physical grip/pull-or-push/release
phase lifecycle to a separate agent. This workstream owns authored geometry,
collision shapes, passive joints, and physics evidence. Preserve mechanism IDs.

Checkpoint 200 adds frictionless swept self-contact to the live cloth solver.
Vertex/face and edge/edge constraints share response by inverse mass, account for
fixed support reactions, and reject unresolved steps transactionally. Reusable
spatial envelopes and separating axes retain coverage when contact corrections
change trajectories. The connected folded-sheet diagnostic settles for ten
seconds with a minimum measured feature gap of 0.0009999919 m and final speed
0.00009714 m/s. Full Rust suite: 358 passed, two ignored; the existing Python
ownership source-text failure remains. Evidence and playable fold previews live
under `artifacts/toronto-condo-captures/cloth200/` and gallery 200a/b. Final 43 cloth
checks pass; all 101 video frames decode with exact render/solver mesh parity.
Two preview colours are halves of one connected sheet, not separate duvet layers.
Required next: shared live stepping across separate bedding layers, moving-body
collision with pose/time alignment, and authored duvet layers with rendered
interaction evidence. The latest rigid scene remains `scene-entry-door.json`.

Checkpoint 199 adds swept cloth contact against fixed cuboids/rounded cuboids.
Seven new crossing/grazing/settling regressions pass; the full Rust library suite
passes 346 tests (two ignored). The existing Python ownership source-text failure
remains. Swept-contact witnesses use pinned Parry f64 0.17.6. Time-ordered
interval subdivision avoids a grazing-contact advancement stall; unresolved
queries fail the transactional step. Whole-surface and trajectory culling retain
byte-identical bed traces and reduce the observed suite time from 242.84 to
38.81 seconds. See checkpoint 199 in `condo-manipulation.md` for response limits.
Gallery 199a/b includes playable Blender recordings of the Rust solver at 10 fps
(201 frames). Final source hashes, timestamps and full video decoding pass;
finite authored-mattress samples report zero penetration. Evidence is in
`artifacts/toronto-condo-captures/cloth199/`.
Required next work remains self-collision, moving-body collision with aligned
pose/time intervals, and separate interacting duvet layers. The articulated
scene package remains `scene-entry-door.json` (45 parts, 31 passive joints).

Checkpoint 198 adds the inward-opening entry door. Latest opt-in package:
`scene-entry-door.json` (45 parts, 31 passive joints). Its retained frame matches
source coordinates; observed open/closed poses respectively clear/block the
70 cm by 190 cm doorway envelope. All 339 Rust library tests pass (two ignored);
the unchanged Python ownership source-text test still fails. Gallery 198 adds
authoring, recorded hinge animation and assembled placement. All explicitly
requested rigid furniture parts now have passive joints. Remaining required
work is sheet self/moving/fast collision and separate interacting duvet layers;
see the concrete cloth integration gaps following checkpoint 198 in
`condo-manipulation.md`. Optional mechanical realism remains unassessed.

Checkpoint 197 adds three hollow upper cabinets with passive 0–90 degree hinges.
The latest opt-in package is `scene-upper-cabinets.json`; it contains 43 parts
and 30 joints. Twelve loose shelf objects and 48 drawer objects stay contained
through ten simultaneous hinge/rail cycles. All 338 Rust library tests pass
(two ignored); the pre-existing Python source-text check still fails. Gallery
197 includes authoring, a recorded hinge animation and assembled placement.
Remaining required physics: entry door, sheet self/moving/fast collision and
separate duvet layers; optional realism remains unassessed.

Checkpoint 196 adds a hollow desk drawer to `scene-desk-drawer.json`.
Its passive rail travels 35 cm along -X; the source
desktop/frame/cable organizer are retained. All 16 kitchen/island/desk drawers
pass ten simultaneous cycles with 48 loose contents. All 336 Rust library tests
pass (two ignored); the unchanged Python source-text ownership check still
fails. Gallery 196 retains failed authoring, a recorded physics animation and
the assembled condo capture (39 parts, 27 passive joints). Remaining scope:
upper cabinets, entry door, cloth collisions/duvet layers, shelf contents and
optional mechanical realism. The functionality agent still owns grip zones
and interaction phase transitions.

Checkpoint 195 adds nine kitchen and six island drawers to the preceding opt-in
`scenes/toronto_condo/scene-kitchen-drawers.json`. Both banks use passive 40 cm
rails. Forty-five loose spheres/cylinders/boxes (2 kg per drawer) pass ten
simultaneous open/close cycles, with measured settling up to 5.083 seconds.
Swept and recorded-pose clearance pass. A Blender import transform bug was
fixed in both the new retained carcasses and the previous vanity accessory
placement; regression checks verify source coordinates. Gallery 195 preserves
the failed import, recorded mechanics and corrected assembled scene. The live
capture verifies all 61 asset hashes and 26 passive joints. See the active
physics checklist and checkpoint 195 in `condo-manipulation.md` for the full
remaining scope: upper cabinets, desk drawer, entry door, cloth collisions and
duvet layers, shelf contents/durability, and optional passive realism.

Checkpoint 194 adds two passive vanity drawers with 40 cm travel in
`scenes/toronto_condo/scene-vanity-drawers.json`, now the latest opt-in scene.
The upper drawer has separate bins around the recessed basin; the lower tray
holds a loose 200 g cube through opening and closing in the physics fixture.
The formerly obstructing towel/rail is moved to the opposite bathroom wall,
with separate accessory bounds replacing the overly broad shared collider.
All 332 Rust library tests pass (two ignored). Swept and observed-pose clearance
checks pass; Python retains its existing source-text failure. Evidence and
the recorded physics preview are under `artifacts/toronto-condo-captures/vanity194/`
and gallery 194. No avatar subsystem was changed.

Latest owner direction (2026-10-17): **disable scene conditioning and use the
`interact` function**. This supersedes the earlier scene-conditioned gesture
direction and approval request. The SDK proposal is withdrawn; do not implement
its context-v2/scene-conditioning work. Keep passive mechanisms and physical
contact/outcome validation available for the `interact` integration. This
checkpoint records the direction; it does not claim that runtime routing or
configuration has been changed or that avatar manipulation has succeeded.

Checkpoint 193 adds the passive bathroom door in the latest opt-in manifest,
`scenes/toronto_condo/scene-bathroom-door.json`. Its inward 0–90 degree hinge
retains the original wall, with both handles modeled and the hallway grip exposed.
Physics obstruction/open/release/close, continuous clearance, recorded-pose
clearance and read-only closed/open navigation checks pass. All 330 Rust library
tests pass (two ignored); Python retains its existing source-text failure.
The live capture verifies 40 asset hashes, 16 mechanism parts and nine passive
joints. Evidence is under `artifacts/toronto-condo-captures/bathroom193/`;
gallery 193a shows the live integration and 193b the recorded physics sequence.
This is mechanism readiness, not demonstrated avatar handling. The SDK
scene-conditioned interaction proposal is withdrawn by the latest direction.

Checkpoint 192 prepares an isolated outfit-bound calibration at
`scenes/toronto_condo/calibration-v13/profile.json`. It was prepared for the
formerly proposed scene-conditioning integration and is not installed into the running service.
The exporter verifies additive asset preservation and original calibration
parity, retains verified contact patches, and adds both garment skins and the
full 121-node hierarchy. The shared SDK calibration is untouched.
All 240 seed-189 native frames pass CPU world/root parity; six frames also pass
the existing Rust exported-geometry test. Nine surfaces over five frames give
413,335 checked skin vertices with maximum difference 8.89e-16 m. Evidence is in
`artifacts/toronto-condo-captures/calibration192/`. No new screenshot was needed
for this read-only calibration export. This checkpoint did not implement the
interaction proposal, which the owner subsequently withdrew.

Checkpoint 191 prepares the cross-repository implementation proposal at
`../remotemedia-sdk-office-main/openspec/changes/add-articulated-scene-interactions/`.
Strict OpenSpec validation passes. It covers context v2, observed physical
grip/release, geometry-grounded open/close, outcome checks and real-time video
acceptance. SDK OpenSpec instructions require owner approval before implementing
this new capability; the proposal was subsequently withdrawn. No new motion exception was requested.
Preflight also found that the existing shared scene-calibration profile binds a
different avatar hash than the condo outfit. Preserve the dirty shared files;
export an isolated profile from the actual outfit before scene-conditioned runs.
Evidence: `artifacts/toronto-condo-captures/scene191/profile-preflight.json`.
No new generation or screenshot was produced in checkpoint 191.

Latest: checkpoint 190 adds read-only articulation observations to each constrained
prop: parent, world anchors/axis, signed angle or travel, limits, motor flag and
backend registration. All 328 library tests pass (two ignored), including actual
mechanism open/block/close fixtures and rotated-parent/overtravel observations.
A rebuilt live capture publishes all eight passive joints across 120 frames,
with maximum geometry error 3.05e-7 m and unchanged avatar replay. Evidence and
video are in `articulation190/` and gallery 190. These facts are not yet included
in the signed SDK scene context; physical grip/lifecycle integration remains next.

Checkpoint 189 preserves a text-only door-gesture baseline in the gallery
as **Failed: approach prerequisite**. All 240 native packets were displayed
unchanged, but the closest observed wrist remained 4.60 m from the handle and
the door did not travel. No grab intent was armed. This is not a grip-failure
test. Source and capture evidence are in `interaction189/` under the capture
artifacts directory. The user prefers the existing **scene-conditioned gesture**
path next; do not spend more text-only seeds. See checkpoint 189 in
`condo-manipulation.md` for the integration gaps. No model/physics behavior was
changed by this checkpoint.

Checkpoint 188 adds the passive balcony slider in
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
