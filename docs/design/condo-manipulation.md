# Condo manipulation: current priority

## Active physics goal (2026-10-17)

The owner replaced the broader visual goal with this physics scope. Keep all
items open until their actual assets/runtime behavior and evidence are verified.

- Kitchen and island drawers: 15 passive rails implemented and checked with
  45 loose contents through ten simultaneous cycles (checkpoint 195).
- Upper cabinets, desk drawer and entry door: movable geometry and joints pending.
- Bed sheets: self-collision, moving-object collision and fast-motion collision
  protection pending; separate duvet layers remain unfinished.
- Contents and durability: expand beyond the vanity cube to loaded drawers,
  shelf contents, repeated cycles and simultaneous mechanisms.
- Optional realism: assess latches, soft-close behavior, door closers and
  improved mass/friction calibration after the core mechanics are reliable.

The functionality agent owns grip zones and interaction phase transitions.

Current scope: **physics and movable condo parts only**. The owner replaced the
former visual/avatar goal with the active physics goal above.
The owner assigned grip zones and the physical grip/pull-or-push/release lifecycle
to a separate agent. This workstream should not edit that functionality.

## Checkpoint 195 — kitchen/island rails and loaded cycling

Latest opt-in package: `scene-kitchen-drawers.json`, extending the vanity package.
New mechanisms `kitchen_drawers` and `island_drawers` have nine and six moving
parts respectively. Stable IDs are `kitchen_drawer_{column}_{row}` (columns
0–2, rows 0–2) and `island_drawer_{column}_{row}` (columns 0–2, rows 0–1).
Each drawer has a named `_pull` mesh. Grip-zone authoring is left to the
functionality agent. Both banks have 0–0.40 m passive rails; kitchen axis is
`[1,0,0]`, island axis `[-1,0,0]`. Estimated empty masses are 5 and 7 kg.

The kitchen keeps its oven, sink, counter and fixed carcass. Two upper trays
use side storage and shallow front pockets to clear the basin. The island's
solid body is replaced by a hollow carcass with fixed dividers and back.
All moving parts use real open trays and compound collision boxes.

The first preview exposed an import bug: library-loaded object transforms
were read before Blender evaluated them. Retained cabinet meshes and their
bounds were consequently misplaced. The same pattern affected the previous
vanity towel and soap import. Both scripts now evaluate transforms before
relocation. Source-coordinate regression checks and the assembled capture
verify the corrected positions. Earlier positive checks on those misplaced
parts were insufficient; the rejected preview and failed logs remain archived.

Swept checks pass 8,103 kitchen and 4,200 island comparisons, with at least
3 mm clearance after a 1 mm reserve. All recorded solver poses pass 6,806,520
and 3,528,000 fixed-room comparisons, with approximately 4 mm minimum gap.
The full-scene physics fixture checks obstruction, transverse forces, stops,
release, closing and published current bounds. It instantiates all mechanisms,
so the opposing banks collide with real neighboring parts during testing.

The loaded test puts a sphere, cylinder and box in every drawer: 45 loose
bodies, 2 kg total contents per drawer. All 15 drawers complete ten simultaneous
open/close cycles. Content centers remain within their bins and retain support
height throughout; all drawers reach their endpoints. A two-second quiet
interval initially failed for rolling contents. The final test measures up to
ten seconds and requires 30 consecutive frames below 0.05 m/s without changing
body velocity or damping. The longest measured interval is 5.083 seconds;
5,675 physics steps are checked. This is bounded durability evidence, not
arbitrary-load or lifetime certification. Shelf-content coverage remains open.

The live capture verifies 61 asset hashes, 37 mechanism parts and 26 passive
joints over 120 frames. Gallery 195a records the rejected import; 195b/c are
14-second recorded kitchen/island sequences at 10 fps; 195d shows installation.
Evidence is under `artifacts/toronto-condo-captures/kitchen195/`.
Structural review retains similar phase assertions, the explicit obstruction
fixture's branching, and the drawer builder's parameter count; the loaded
cycle test was split into force/containment and endpoint checks for readability.
All 335 Rust library tests pass (two ignored). The existing Python ownership
source-text failure remains unchanged; its other six tests pass.

Previous vanity integration handoff (included in `scene-kitchen-drawers.json`):
Its new moving IDs are `vanity_lower_drawer` and `vanity_upper_drawer`, each with
parent `vanity_shell`, world-aligned axis `[1, 0, 0]`, and limits `[0, 0.40]` m.
The wall-mounted towel is a separate fixed part, `vanity_wall_towel`.
Existing `handle_region` entries are authoring metadata for the functionality
agent to review; they do not certify a grip. Published articulation observations
use the existing checkpoint-190 contract. Mechanism part positions use the
authored origin; published prop transforms use the collider-bounds center.

## Checkpoint 194 — vanity drawers and loose contents

`scene-vanity-drawers.json` extends the bathroom-door package. Two estimated
5 kg drawers move along passive X rails with 0–40 cm limits. The lower tray
has a full open cavity; the upper drawer uses two side bins to clear the
retained basin. The cabinet bottom/back/sides remain fixed. Counter and basin
collision envelopes conservatively reserve their solid volume; the basin is
not a usable hollow sink. Rails have no motor, latch or soft-close mechanism.

The first clearance and solver checks found that the original static towel
rail and hanging towel obstructed the fronts. The authoring script relocates
their actual meshes to the opposite bathroom wall in a separate fixed part.
Soap remains on the counter. Per-item accessory bounds replace the original
aggregate collider. The front panels gained 3 mm clearance; assertions and
solver settings were not weakened. Failed check and setup logs are retained locally.

The exact swept-volume check passes 1,805 comparisons with a 3 mm reserved
minimum gap. The 840-step physics trace passes 1,516,200 fixed-room comparisons
with a 4 mm minimum gap. The diagnostic fixture checks transverse loads,
obstructed upper travel, both end stops, release, closing and current bounds.
A separate loose-content test drops a 200 g, 8 cm cube onto the lower tray,
opens and closes it under diagnostic loads, checks support height throughout,
and checks containment at the settled endpoints.
The cube returns to approximately its starting position without a coupling.
These probes establish the tested geometry and loads, not arbitrary contents.

All 332 Rust library tests pass, two ignored. Python has six passes and its
existing ownership source-text failure. Structural review retains explicit
fixture/assembly similarities, the new fixture's branch count, historical
render-source duplication and preview-helper churn. The animation helper adds
an optional 10/15/30 fps setting (default remains 30), sampling original 60 Hz
poses at the matching interval without interpolation or changing physical time.
The 194 video is a 14-second, 15 fps physics demonstration. Its isolated studio
omits the retained basin/counter; the condo integration view includes them.
No avatar behavior or SDK code changed in this checkpoint.

Latest owner direction, 2026-10-17: **scene conditioning is disabled in the
interaction plan; use the `interact` function**. This supersedes checkpoint 189's
gesture direction. The SDK scene-conditioned proposal is withdrawn, not waiting
for approval. Existing passive articulation and physical grip/outcome checks
remain applicable. Runtime `interact` wiring and actual avatar handling require
their own verification; this documentation change does not certify them.

## Checkpoint 193 — passive bathroom door

`scene-bathroom-door.json` extends the balcony-slider scene with a 1.18 m oak
leaf and an ideal passive 0–90 degree inward hinge (estimated mass 20 kg).
The fixed part retains the exact replaced south-wall envelope. Both bronze
levers are modeled; only the hallway-side grasp region is exposed. There is
no latch, lever actuation, runtime motor or avatar manipulation claim.
The Blender source is `interaction-authoring/bathroom-door/bathroom-door.blend`;
`scripts/condo/bathroom_door.py` shares the upright leaf builder with the bedroom.

The continuous conservative sweep passes 78,735 comparisons with 3.65 mm
minimum reserved clearance; all 1,080 recorded physics samples pass 469,800
fixed-room comparisons with 9.80 mm minimum gap. Both are limited to authored
fixed colliders. The diagnostic fixture checks obstruction, full opening,
release, closing, settling and current published bounds/joint observations.
The read-only route test confirms closed blockage, open passage and stale-path
invalidation without moving the avatar. Its initial point was inside the
kitchen island's navigation margin; the corrected point lies in the actual
aisle. The failed attempt remains archived.

All 330 library tests pass, two ignored. The Python ownership suite retains
six passes and its existing source-text failure; no policy test was weakened.
Scoped structural review retains two duplication findings: two explicit
navigation test wrappers and similar assembly metadata between the hinged
leaf and the distinct balcony slider. Actual leaf construction and physics
assertions are shared. The static scanner also marks the two new Rust tests
as dead code, although both execute. Findings were not suppressed or reset.

The live capture verifies 40 asset hashes, all 16 mechanism parts and nine
registered passive joints over 120 frames. All 240 existing native packets
are replayed unchanged; root interpolation error is 7.62e-8 m and the hand
observer reports zero contacts. The avatar is outside this camera. Gallery
193a is a four-second encoded integration capture, not real-time interaction.
Gallery 193b preserves the full 18-second diagnostic physics trace at 30 fps,
using alternate 60 Hz samples without interpolation. Source traces, checks
and failed attempts remain under `artifacts/toronto-condo-captures/bathroom193/`.
The scene-conditioned avatar interaction proposal is now withdrawn; use `interact`.

User direction, 2026-10-16: pause visual detail work and prioritize fridge,
drawers, cabinets, doors and **bed sheets** that the avatar can manipulate.
The condo scene goal remains intact. Lighting and reflection polish are deferred.

Current opt-in package: `scene-balcony-slider.json` combines the passive fridge,
bedside drawers, entry cabinet doors, bedroom door, balcony slider, live top-sheet solver and split bed
collision shapes. Grasping/manipulation by
the avatar is still unverified. Checkpoint 180 adds a solver contact-clamp
prototype; checkpoint 181 adds triangle contact and clears the measured
triangle/mattress penetration in the lift/release fixture. Runtime hand
contact observations are available in checkpoint 182; grasp admission and
avatar handling remain unfinished. See the progress gallery.

## Checkpoint 190: observed articulation facts

`PropSnapshot.articulation` now publishes the permanent hinge or rail attached
to that prop. It records its parent ID, physics registration, both world-space
anchors, parent axis, enabled motor status, signed angle/travel and limits with
explicit radians/metres. Values use the same observed transforms as the prop and
handle. They are never clamped to limits, so solver overshoot remains visible.
Revolute angles use Rapier's principal [-pi, pi] convention. Fixed objects and
temporary child hand grips do not appear as articulated props.

This observer has immutable access to joint/transform data. It adds no actuator,
body-motion conditioning or grip admission. The physics fixtures now compare
published joint facts against actual mechanism transforms during opening,
obstruction, release and closing. Additional tests cover a rotated parent,
negative signed angles, overtravel outside limits and unregistered joints.

All 328 library tests pass, two ignored. The rebuilt condo renderer published
960 observations across its eight passive joints in 120 captured frames; an
independent manifest/transform comparison measured maximum anchor/axis error
3.05e-7. All 240 unchanged seed-189 packets were delivered, root interpolation
error stayed below 8.48e-8 m, and the video decoded. This is observation evidence,
not a successful door manipulation: the replay still never reaches the handle.
Gallery 190 preserves the capture and audits. The preview again compresses
twelve seconds of native motion into four encoded seconds.

The Python suite retains its pre-existing source-text ownership failure (six
passes, one failure); no ownership gate changed. Ripwire still reports artifact
clones, existing module-size findings and publisher churn, and does not resolve
the ECS test edges. Its gates are not claimed green; full Rust tests and the live
capture provide the actual execution evidence.

Next: carry these observations into the signed scene context with coordinated
Rust/Python schema validation, connect articulated interaction transitions to
physical contact grips, and feed back observed grip/release and joint state.
The current SDK context parser still rejects unknown fields; do not send this
new field into it without updating and testing that contract. The public goal
continues to describe an outcome, not a wrist or root track.

## Checkpoint 189: scene-conditioned gesture direction

User steering, 2026-10-17: probably use the scene-conditioned gesture. Keep
mechanism articulation and actual manipulation ahead of cosmetic work.

The single text-only baseline (seed 189, 12 seconds, Core40) generated a gesture
near the origin. In the unchanged CC5 replay, the closest observed wrist stayed
4.5997 m from the balcony handle; observed rail travel was zero. All 240 source
packets were delivered, six source audits reported zero future constraints, and
the root interpolation audit passed at 8.46e-8 m. No grab intent was armed;
this failed the approach prerequisite, not an attempted physical grip. Gallery
189 preserves the playable capture, audits and setup-failure log. Its four-second
preview compresses twelve seconds of source motion. Do not infer real-time motion
quality from the preview speed.

Existing integration points, verified from current source:

- `office/scene_context.rs` exports current object transforms, compound solids,
  approach/grasp regions and declared interaction zones. `scene_signing.rs`
  signs the internal `scene_step` envelope for the SDK scene-conditioning path.
- The scene snapshot currently has no hinge/rail axis, limits or joint state.
  Those environmental facts need a defined renderer-to-service representation
  before claiming constraint-aware door generation.
- `OfficeController::apply_coupling` explicitly rejects attaching an articulated
  part. Generic declared attachment makes free objects follow the hands and is
  unsuitable for a constrained door. Preserve that rejection until articulated
  transitions use `physics/contact_grasp.rs` and its physical contact admission.
- Scene context currently exports held objects from legacy declared couplings;
  an admitted physical door grip also needs observable feedback to the motion
  service. Do not report a requested grip as an observed grip.

Next bounded integration slice: the balcony slider. Ground the gesture in its
actual handle geometry and live obstacles; include its passive +X rail and
0–2 m limits as scene facts. Let the motion service generate approach and body
motion, use contact admission for the temporary hand joint, and retain the
permanent rail joint. Observe opening, release and settling from physics.
Refresh scene facts as the door moves. Stale geometry, failed contact and
unsupported conditioning must remain explicit failures.

Acceptance must separately show unchanged source/displayed motion, actual
contact before grip, door travel, release and sustained settling. Publish a
playable opening/closing recording in the progress gallery. Do not enable the
SDK's optional planned-grasp experiment merely because scene conditioning is
selected; it has its own scoped approval and audit contract. Sheets still need
their separate deformable-contact/grip integration.

This checkpoint documents the integration boundary; it does not implement or
certify scene-conditioned door manipulation. Existing SDK changes and avatar
calibration files were left untouched.

## Verified starting point

`scripts/condo/prepare_interaction_assets.py` reads the Blender source without
modifying it and exports `scenes/toronto_condo/interaction-authoring/`:

- `inventory.json`: 94 current meshes in 11 interaction groups, with actual
  object/mesh names, parents, bounds and collider-group membership.
- `bed-cover-surface.json`: the duvet's single midsurface, 3,705 vertices,
  7,168 triangles, rest edges, material coordinates and 240 boundary edges.
- `validation.json`: connectivity, rest-length and unchanged-source checks.

These are authoring assets, not a runtime interaction manifest. No joint, motor,
cloth solver or avatar grip is enabled by this export. The separate top sheet,
bedroom/bathroom door leaves and balcony sliding leaf needed authoring at this
starting point; checkpoints 174–179 add the separate top sheet.
The duvet surface retains its current folds; solver material/rest-state
calibration remains necessary before treating it as physical bedding.

## Checkpoints 169–170: fridge authoring fixture

`scripts/condo/fridge_mechanism.py` now creates a separate hollow fridge with
shelves, two door/handle assemblies, local collider boxes and passive hinge
metadata in `interaction-authoring/fridge/`. Each assembly has its own GLB;
`fridge-mechanism.blend` contains a Blender physics diagnostic. The normal condo
retains its static fridge; checkpoint 172 adds a separate articulated package.

The original 105-degree design failed the conservative geometry sweep against
the bathroom wall. The revised 0–90-degree sweep passes 162,900 box-pair checks
and rejects an intentionally obstructing block. Under explicitly diagnostic
lateral gravity, both Blender doors open to 90 degrees and return within 0.01
degrees of closed. Observed angles span -0.078 to 90.981 degrees, with maximum
hinge anchor error 2.74 mm. This is asset mechanics evidence, not avatar handling
or Bevy/Rapier acceptance.

A separate -0.1–91.5-degree angular sweep passes; an expanded -1.5-degree closing
overshoot fails against the shell. The angular sweep does not include observed
anchor translation, so combined solver clearance remains to be validated.
The initial reversed hinge-limit convention and cropped diagnostic screenshots
are retained alongside corrected full-fridge views in gallery iteration 169.
Iteration 170 supplies the verified final asset hashes, rerun mesh audits and
complete source snapshot. It corrects 169's evidence packaging: its door audits
predated the last export and a failed hash assertion prevented the final source
snapshot from being staged. Both entries are preserved.

Python compilation passes. The repository Python suite reports six passes and
one existing motion-ownership text-check failure; the broader quality gate still
reports 17 existing findings. No new fridge-script quality findings remain after
the sweep refactor. No runtime interaction is certified by this checkpoint.

Next: integrate permanent hinge ownership independently from temporary grips,
then validate the actual runtime collider poses and contact/release behavior.
For bedding, author a separate top sheet and fitted sheet before solver trials;
the exported duvet surface is only the starting topology. First cloth evidence
must show gravity drape, mattress collision, a local material-space grip, release
and settling, with deformation and penetration measurements preserved.

## Deliverable sequence

| Work | Asset changes | Runtime and acceptance |
| --- | --- | --- |
| Fridge vertical slice | Hollow shell, interior liner/shelves, two complete door assemblies, handles, explicit hinge frames and limits | Independent rigid bodies and passive hinge constraints; current door/handle poses and opening fraction observed; open/close through real contact with unchanged generated motion |
| Drawers | Replace solid cases where needed; add actual drawer boxes; front and handle move with each box | Passive prismatic constraints, travel stops, resistance, moving collision geometry and contents; verify blocked drawer and release/settling |
| Cabinets and room doors | Hollow storage, shelves, door leaves and hardware; stable part IDs | Revolute constraints with tested handedness, full swept clearance, current navigation obstacles and no motor-driven success animation |
| Bed sheets and duvet | Separate fitted sheet/top sheet/duvet; stable low-resolution simulation topology and render bindings | Deformable solver; mattress/body/self-collision as needed; material-space edge grips, release and settling; measure stretch, penetration, grip slip and runtime cost |

Fridge and bedding are the initial exemplars. Extend verified mechanisms to
kitchen/island drawers, upper cabinets, vanity, bedside unit, desk, entry storage
and room/balcony doors. Opening an empty box is not the final furniture asset:
interiors and contents need valid collision/containment too.

## Integration requirements

The ordinary scene solids become static office objects and fixed aggregate
collision boxes. Checkpoint 172 replaces both representations for the opt-in
fridge; other furniture still needs that conversion. Retaining an old full-volume
box would block the interior and moving door/drawer.

Keep a fixed carcass and dynamic child parts with local joint frames. Author
physical limits, material properties and geometry-based handle regions; never
author body paths, wrist targets or contact times. Scene queries and navigation
must use current child poses. Detached render nodes must retain their original
world transforms and stable source identities.

The rigid-grip path now creates a dedicated child `ContactGrip` joint for each
hand (`physics/grab.rs`). Release and target swap remove only that child's joint,
preserving permanent articulation and any other hand's grip. Prop removal and
replacement recursively remove children and clear both hands' attachment state.
Admission retains the actual shape-contact test in `contact_grasp.rs`, preserving
the captured relative frames without teleporting.

Checkpoint 171 tests this through the actual Bevy/Rapier backend: two grips and
one permanent hinge coexist; swapping and releasing individual grips preserves
the original hinge component and backend handle. After both releases, diagnostic
initial angular velocity rotates the body to its hinge limit while its anchor
stays fixed. Removing/replacing a two-hand prop leaves no grip entities or stale
child references. These are synthetic physics fixtures, not avatar evidence or
validation of the exported fridge's exact geometry.

`cargo test --offline --lib` passes 269 tests with 2 ignored. The Python suite
still has its existing motion-ownership text-check failure (6 pass, 1 fail).
The broad quality delta retains 17 existing gating findings; the static test
mapping gate also remains nonclean and does not recognize the executed Rust
fixture coverage. No new gating finding is attributed to the grip changes.
Source snapshots and logs are in
[`grip171`](../../artifacts/toronto-condo-captures/grip171/validation.json),
including the failed low-disk linker attempt and successful rerun. This changes
the physics source and test binary; it has not yet been accepted in a live
renderer capture. The next step is loading the fridge's separate rigid bodies,
removing its old aggregate collider/render parts and reporting current part poses.

## Checkpoint 172: opt-in articulated runtime package

`scripts/condo/prepare_articulated_package.py` creates `scene-articulated.json`
from the existing lightmapped scene and audited fridge exports. The strict
mechanism schema describes fixed parts, dynamic parts, collider boxes, handles
and passive upright hinges. It rejects motion/motor fields, invalid rest frames,
duplicate IDs, missing parents, invalid dimensions and changed geometry hashes.
Articulated packages require physics. The original scene manifests are unchanged.

The renderer creates three independent compound rigid bodies and two hinges,
skips `kitchen_fridge`'s old full-volume collider, and hides exactly five old
fridge nodes within the environment scene. Readiness waits for body/joint
registration, every replacement node and all part GLB dependencies. Each body's
origin is recentered on its collider bounds; render geometry, joint frames and
grasp regions subtract the same fixed offset. Generic prop observations publish
current transforms, bounds and grasp points; office metadata uses the same
body-centered bounds.

The exact exported shapes pass cavity, mass, anchor and metadata checks in
Rapier. Under labelled diagnostic forces they open to 90 degrees and close,
with maximum anchor error 0.0485 mm. Snapshot bounds are independently checked
against transformed authored box corners at both ends of the cycle. A separate
test verifies that replacement visibility cannot hide identically named nodes
outside the environment scene and missing replacements prevent readiness.
The Rust library suite passes 275 tests with 2 ignored.

Gallery 172 is a rebuilt-renderer, four-second kitchen capture: 120 frames,
three stable part observations, maximum closed-door angle 0.000058 degrees and
maximum hinge-anchor error 0.00014 mm. All five old nodes were hidden. The
beautiful_assistant avatar remains in her original location outside this camera;
119 observed pelvis samples match original packet interpolation within 9.3e-8 m.
This is a closed-fridge integration preview, not rendered opening or avatar
manipulation evidence. The opening cycle is a separate physics fixture.

Still required: live model-generated contact/grasp/open/release; articulation
state and action semantics; generated navigation using current geometry;
lighting invalidated by moving geometry; and extension to drawers, cabinets,
other doors and deformable bedding. Checkpoint 173 addresses held-part obstacle
membership and stale read-only waypoint previews. At checkpoint 172 the Python
ownership text-check failure and 17 broad quality findings remained unchanged.

## Checkpoint 173: constrained navigation observations

Package installation records which parts have permanent joints. Those parts
remain in the navigation obstacle list while held or coupled. The waypoint API
now uses that same observed list instead of independently excluding everything
in an occupied hand.

Every waypoint preview includes `navigation_geometry_sha256`. Call
`SceneState::waypoint_geometry_is_current(&path)` on the latest scene state to
check its session and obstacle-geometry identity. This is a conservative exact
geometry check, independent of avatar timestamp or motion revision. It does not
reserve the route, verify the avatar's current start position, execute movement
or automatically replan model motion. Any changed geometry requires a fresh
preview. Older previews without a hash fail the freshness check.

Physics observations update object position, rotation and navigation bounds.
Constrained parts do not trigger the legacy motion rebuild path. The declared
kinematic attachment command rejects them, and its transform-writing system
excludes bodies that have permanent root joint components. Temporary hand grips
continue through contact physics.

Tests use the actual package installer and fridge geometry, isolating the
appliance for route checks because the nearby bathroom wall otherwise masks the
door's navigation footprint. They verify that a held door remains an obstacle,
0.001-degree observed rotation invalidates a prior preview without avatar-clock
progress, and a 90-degree door blocks a previously clear endpoint. Queries and
observations leave motion data/revision unchanged. A separate actual ECS-system
test verifies that legacy release cannot teleport or freeze a jointed body.

The library suite passes 279 tests with 2 ignored. The existing Python ownership
failure remains. The broad quality gate reports 23 findings: the previous 17
plus six short-horizon churn flags on intentionally changed navigation/controller
code and its tests; no new structural gating finding was introduced. Static
test mapping remains nonclean. Generated avatar navigation and actual hand-door
contact are still unverified.

Gallery 173 archives a rebuilt-renderer living-room regression: 120 frames, no
reported motion errors, both doors present in the navigation list throughout,
unchanged input packets and 119 observed pelvis samples within 7.1e-8 m of
original interpolation. This capture contains no hand-door interaction and
does not change the deferred visual-polish assessment.

Cloth is not a rigid prop and is not covered by the existing secondary-bone
simulation. Bind temporary grips to material triangle coordinates and a local
patch after observed contact. Re-evaluate edge exposure/occlusion in the current
deformed state; the exported boundary lists alone do not establish a valid grip.
Do not pin the whole sheet or attach it as one rigid body. Start with no tearing
or remeshing so material identity remains stable. A solver spike must establish
stable collision, coupling, release, stretch bounds and session-clock integration
before adding live avatar handling.

## Checkpoint 174: separate bed sheet, settling not yet stable

Bed sheets are part of the manipulation scope. `scripts/condo/check_bed_sheet.py`
authors a separate 2.3 m square top sheet with 1,221 material vertices, 2,304
triangles, UV coordinates and 136 boundary edges. Its head, foot and side edge
IDs are candidates for future contact grips. There are no pins or avatar motion.
The standalone Blender fixture loads the actual condo mattress without changing
the source scene; the existing static duvet remains unchanged. Reusable assets
are under `scenes/toronto_condo/interaction-authoring/bedding/`.

Both gravity runs failed acceptance and are retained in `sheet174b` and
`sheet174c`, with screenshots in the progress gallery. The first run exceeded
the 10% stretch limit (16.1%) and did not settle. The revised 48-step, ten-second
run stayed within 8.95% extension and measured no vertices inside the mattress,
but maximum vertex speed over its final half-second remained 0.801 m/s, above
the 0.05 m/s settling threshold. Screenshots also show unresolved wrinkles and
angular edges. This is a failed solver calibration, not ready cloth handling.
The file named `settled-positions.json` records the final observed positions;
its historical filename does not mean the settling test passed.

The diagnostic measures vertex penetration, not continuous triangle collision
or self-intersection. The 0.8 kg total mass is distributed across vertices;
Blender's installed RNA identifies `ClothSettings.mass` as per-vertex mass.
Coefficients are prototype values, not measured linen. See Blender's
[cloth collision documentation](https://docs.blender.org/manual/en/latest/physics/cloth/settings/collisions.html).
The initial attempt's incorrect identity-transform assertion is retained in
`sheet174/failed-transform-assumption.log`; validation now uses the mattress's
evaluated world-space geometry.

Next: stable unpinned settling, then observed contact on a local material patch,
pull/lift/release, and only then live avatar handling. Fitted sheets, sheet/duvet
layer collision, folding and runtime solver/render binding remain unimplemented.
The seven Python checks still report six passes and the pre-existing ownership
text-check failure. No Rust code changed in this checkpoint. The broad quality
gate still has 23 prior findings; it also reports an informational duplication
between the two standalone diagnostic studio setups.

## Checkpoint 175: sheet settling passes a stronger fixture

The standalone top sheet now settles under gravity without pins. Its 2.3 m
square rest surface, 0.8 kg total mass and material vertex IDs are unchanged.
The final default-script run (`sheet175-final`) passes: maximum edge extension
0.542%, no measured vertex penetration into the mattress, maximum speed over
the final half-second 0.00887 m/s, and 47.0% of vertices within 8 mm of an upward
mattress surface. All simulated steps report solver success. The source condo
hash remains unchanged. A separate six-thread candidate run also passed;
the final eight-thread run produced different corner folds, so exact vertex
positions are not claimed deterministic.

Controlled diagnostics showed that disabling either collision type did not
remove the original instability. Reducing spring damping stopped the jitter,
but the previous air damping then made the unsupported sheet fall extremely
slowly. The old speed-only gate falsely passed that in-air sheet. Its original
result is retained with an explicit rejection review; a repeated run fails the
new mattress-support gate. Low-friction trials slid off the mattress. All those
attempts and setup failures remain under `cloth175/` and in the gallery.

The provisional settings now use spring damping 0.01, bending damping 0.0005,
air damping 0.01 and mattress friction 40 in Blender's property units. Bending
stiffness is 0.001; the former 0.15 sheet remained too stiff and failed the tighter
8 mm support check even after its motion settled. Both object and self-collision
stay enabled with 3 mm distances. Neither collision impulse clamping nor
task-specific actuation is used. These values are a stable fixture calibration,
not measured fabric properties. Blender documents its collision controls in the
[Python API](https://docs.blender.org/api/main/bpy.types.ClothCollisionSettings.html).

The script saves effective coefficients and its own source/hash before simulation,
then writes `final-positions.json` without implying that a failed run settled.
The saved `.blend` contains the initial fixture; it must be simulated on the
timeline to reproduce a drape. Reusable bedding assets now contain this passing
version. Earlier experimental runner source snapshots were read again at exit;
the diagnostic archive therefore identifies the actual archived baseline sources
and audits each saved `.blend`'s effective settings separately.

The support check is a geometric proximity measure, not a contact-force reading.
The mesh still has coarse corner folds. Continuous triangle collision,
self-intersection verification, bed-frame collision, fitted sheets/duvet layers,
temporary material-patch grips, pull/lift/release, runtime integration and live
avatar handling remain unverified or unimplemented. This checkpoint establishes
a gravity-settling fixture, not supported avatar bed-making.

Validation also checks manifold topology, 136 boundary edges, no pins and source
hashes. The Python suite remains six passes and the existing ownership text-check
failure. The broad quality gate retains 23 earlier findings; its final report
adds no finding for the sheet script. No Rust or avatar motion code changed.

## Checkpoint 176: local sheet constraint, lift and release

`scripts/condo/check_sheet_patch.py` adds a labelled Blender mechanics fixture.
It starts from the recorded drape with zero initial velocity while retaining
the flat material rest shape. Three vertices of a central foot-edge triangle
start attached through compliant pin springs (weights 0.85, pin stiffness 1).
All other vertices remain free. The orange sphere is a noncolliding target,
not a simulated hand or contact sensor. This is initial-condition constraint
testing, not an acquisition policy or successful avatar manipulation.

The synthetic target lifts 0.55 m and translates 0.05 m toward the head. At
frame 122 all three effective weights become zero; the target then continues
away while the cloth responds freely. The unattached control follows the same
target animation with zero attachment weight throughout. Blender's
[Vertex Weight Mix modifier](https://docs.blender.org/manual/en/latest/modeling/modifiers/modify/weight_mix.html)
provides the changing weights before the cloth modifier. The fixture has no
runtime action entry point and writes no body-motion data.

Both final runs pass. In the attached run, the patch centroid rises at least
0.54994 m during the held interval, with maximum centroid lag 1.37 mm over the
entire attached interval. It drops 0.54147 m after release. Maximum edge
extension is 0.886%; no vertices were measured inside the mattress. Final
half-second maximum speed is 0.0171 m/s and near-support fraction is 43.98%.
Exactly the three material IDs are constrained before release, and none after.
The unattached control does not follow the target; its held-interval height
change remains near zero. All simulated steps report solver success.

The first attempt activated the flat rest-shape key, inadvertently changing
the starting geometry. The saved-file audit rejects both that attempt and its
control, even though the old control's outcome-only report said pass. Those
artifacts remain immutable in the gallery. The repair sets rest-key influence
to zero, constructs target coordinates explicitly from the recorded basis and
sets the actual animation curves to linear. A new precondition checks the
initial mesh against the recorded drape; independent audits find zero error
for both corrected final fixtures.

Gallery 176 preserves the failed setup, the corrected trial and the final
recording/control. The video contains 121 samples at 8 fps, including both
endpoints of a 15-second simulation; its encoded duration is 15.125 seconds.
The held and released screenshots were visually inspected and the H.264 stream
decoded successfully. Input hashes match the current scripts, bedding assets
and unchanged source condo. A reusable initial fixture and evidence are under
`interaction-authoring/bedding/local-patch/`; no baked playback or runtime cloth
was added to the condo scene.

Remaining gates: physical finger contact and acquisition, exposure/occlusion,
a true barycentric contact constraint instead of this three-node approximation,
per-contact force/slip validation, continuous triangle/self-intersection checks,
other bed colliders, multiple cloth layers, runtime integration and live
beautiful_assistant handling. Centroid lag is not a per-vertex attachment-error
or grasp-force certificate. The seven Python checks remain six passes plus the
existing ownership text-check failure; the broad quality gate retains 23 earlier
findings and adds none for these sheet scripts. No Rust/avatar motion code changed.

## Checkpoint 177: observed sheet contact geometry

`src/cc_render/deformable_surface.rs` adds read-only shape contacts against a
fixed material topology with replaceable observed vertex positions. Contacts
retain triangle IDs and barycentric material coordinates, so a captured material
point follows deformation. Queries use collider shape and orientation and the
authored sheet thickness; nearby hands do not become attachments. Different
folded layers remain separate candidates.

Each geometry update invalidates previous contact observations. The
`contact_matches_surface` check covers only surface identity and revision: a
moving hand must be queried again before any attachment decision. Capturing a
material point is not permission to grasp. No layer selection, pinch policy,
forces, solver, render mesh binding or avatar action is installed by this module.

Seven focused tests pass, including probes against the recorded settled condo
sheet. The full Rust library suite passes 286 tests with two ignored. Python
remains six passes and the existing ownership source-text failure. The broad
quality gate retains 23 earlier major findings; the contact contract check finds
no incompatible callers. Evidence and input/source hashes are saved in
`artifacts/toronto-condo-captures/contact177/checkpoint.json`; the exact synthetic
probe observations are in `contact-observations.json` beside it. This checkpoint
has no new rendered screenshot; gallery 176 remains the latest visual evidence.

Next: connect observed runtime cloth geometry to this query layer, define actual
contact admission and local compliant attachment, then validate release and
settling before testing unchanged model-generated beautiful_assistant handling.

## Checkpoint 178: CPU cloth solver prototype

`cloth_solver.rs` advances the sheet under gravity with area-weighted particle
masses, compliant edge springs, approximate opposite-vertex bending springs,
and fixed box/rounded-box contacts. Internal spring damping follows
[Small Steps equation 8](https://mmacklin.com/smallsteps.pdf). Parameters remain
prototype coefficients for this topology, not measured linen properties.
The solver publishes each successful step through `DeformedSurface`, preserving
material coordinates and invalidating old contact observations. It takes explicit
simulation time and has no avatar transforms, action goals, pins or grip targets.
Failed steps leave the previous state intact and return an error.

The first undamped run gained energy and left the mattress. A separate contact
scan found a 1.8 mm discrepancy in the general rounded-shape penetration query.
The prototype now restricts fixed collisions to boxes and rounded boxes with
analytic signed distances. That removed the contact discrepancy but did not fix
the instability. Spring damping stopped the explosive response. Damping ratio
0.5 still failed the settling-speed gate at both 10 and 20 seconds. Ratio 1.0
passes the unchanged speed, support, penetration and stretch limits at 20 seconds:

- Maximum edge length ratio: 1.04257 (4.26% extension).
- Maximum measured particle penetration into the rounded mattress: zero.
- Last half-second maximum speed: 0.01912 m/s; support fraction: 52.42%.
- Eight solver tests pass, including gravity, mass/centre conservation,
  dissipating spring energy, support removal, invalid-input atomicity,
  rotated box normals and contact/material identity across simulated deformation.

The full Rust library suite passes 294 tests with two ignored. Python retains
the existing ownership source-text failure (six passes, one failure). The broad
quality gate retains 23 earlier major findings; new informational findings
include the long diagnostic test and solver API not yet used by the live app.

`artifacts/toronto-condo-captures/cloth178/` preserves the failed runs, diagnostics,
positions and final evidence. `preview_cloth_trace.py` renders those exact Rust
positions in Blender without running Blender cloth or adjusting the result.
Gallery 178 records failed and passing previews. The visible mattress uses the
authored mesh; the solver uses an explicitly labelled rounded-box approximation.

This solver is not installed in the live condo yet. The existing aggregate
`bed_base` collider includes bedding and reaches above the mattress; it must be
split before using the sheet in the scene. Remaining work includes render-mesh
binding, session-clock integration, accurate bed/body colliders, self/swept
collision, cloth layers, local compliant hand coupling and actual avatar handling.
Vertex penetration measurements do not certify triangle or self-intersection.
The 20-second diagnostic's wall time includes measurements and is not a live
renderer benchmark. The original scene, avatar and motion data remain unchanged.

## Checkpoint 179: live condo cloth, fixed scene collision and motion clock

`scene-cloth.json` is an opt-in extension of the articulated fridge package.
`prepare_cloth_package.py` measures the mattress and eight bed-frame pieces from
the Blender source, replaces the tall aggregate bed collider, and writes a
hashed material surface. The mattress radius is 0.1064 m, matching the authored
bevel width rather than the earlier 0.03 m fixture approximation. Rounded boxes
still approximate the rendered bevel. The fixed collider list also includes
the other static scene solids. Dynamic mechanism parts and avatar bodies are
not cloth colliders yet.

The Bevy cloth module owns only the sheet mesh and simulation. It samples the
already displayed native-motion timestamp, advances deterministic 1/60-second
steps and updates positions, normals and bounds. It does not advance during
loading, repeated timestamps or motion errors. Session changes, backward time
and jumps exceeding two seconds fail explicitly. Scene snapshots expose cloth
status, simulation ticks, stretch, particle penetration, speed and separate
surface/render-mesh position digests. Missing mesh failures remain observable.
The static duvet is hidden only within the environment hierarchy; the original
asset remains intact. Pillows remain static with no cloth-layer coupling.

Two initial states were rejected before the live capture. The previous
mattress-only drape overlaps the expanded fixed scene by 42.0 mm; it caused a
sharp correction and 13.4% observed stretch. A flat start still overlaps the
bedside lamp's conservative collider by 10.5 mm. The authored flat start now
places the sheet 30 mm farther west, leaving that collider clear. This changes
only the initial cloth asset placement, not avatar motion or runtime repair.
All rejected packages, states and logs remain under `cloth179/`.

The rebuilt GPU renderer recorded 720 bedroom frames (24 encoded seconds).
The sheet advanced through 1,303 fixed steps, or 21.7167 simulation seconds,
then paused with the recorded motion clock. Observed edge extension peaked at
4.38%; particle penetration was numerical roundoff (below 1e-15 m); maximum
speed during the final half-second was 0.01846 m/s. The sampled peak particle
speed was 5.58 m/s (the headless test samples a different cadence and records
8.34 m/s). These are discrete observations, not continuous-collision or
energy-plausibility certification. There were 149 unchanged repeated samples.
Every reported CPU render-mesh digest equals the simulated surface digest.
Matching fixed-step snapshots also agree with the headless ECS run.

Unchanged native packets produced 719 observed root samples within 1.09e-7 m
of their original interpolation. A second, four-second living-room capture
shows beautiful_assistant, with 119 root samples within 7.34e-8 m and no cloth
or motion errors. Both videos decode successfully. The bedroom previews show
deformation, but dark shading bands around the bed edges remain unresolved;
this is a functional prototype, not visual acceptance. State sampling is
asynchronous to GPU readback, so CPU digest equality is not a pixel certificate.

The library suite passes 300 tests with two ignored, including clock stalls and
discontinuities, mesh synchronization, collider/input validation, initial
clearance, missing-mesh failure and environment-scoped replacement. Python still
has six passes and the existing ownership source-text failure. The broad quality
gate retains 23 previous major findings, with additional informational system
parameter/test-length findings. No ownership check was weakened.

Remaining: actual finger contact and admission, compliant local material-point
attachment/release, body/pillow/duvet/dynamic-object coupling, self/swept collision,
high-resolution render binding and visual shading review. No avatar sheet
manipulation is claimed. The default office/static condo packages remain intact.

## Checkpoint 180: contact clamp and a failed surface-clearance check

`cloth_grip.rs` adds a crate-private, finite-force material-point clamp to the
existing solver. Acquisition re-queries the current collider against a chosen
material triangle, captures the existing collider-relative point without moving
the cloth, and distributes subsequent constraint response by barycentric weights
and inverse mass. Adjacent observed collider frames are interpolated during
substeps. Positive compliance avoids a rigid pin; overload releases before an
over-capacity correction is applied. Explicit release preserves velocity. Cloth
and clamp state commit atomically, including rollback after numerical failure.

This is an **abstract closed clamp**, not simulated finger closure, a physical
pinch-admission policy, sliding friction or a calibrated human grip. Capacity
and compliance are generic fixture material parameters. The force estimate uses
the same [Small Steps](https://mmacklin.com/smallsteps.pdf) formulation as the
sheet solver. Nothing connects this prototype to runtime hand commands, writes
avatar transforms or changes the original motion packets. The live cloth system
still calls the gravity/contact-only `step` method with no grips.

Seven new tests cover contact rejection, captured-frame rotation, known-weight
force balance and compliance, overload, independent release of two clamps,
momentum preservation, foreign-surface rejection and transactional failures.
The actual condo sheet settles for 20 seconds against the full fixed scene,
then a diagnostic tool lifts foot-edge triangle 32 by 0.4 m over two seconds,
holds one second, releases and observes ten seconds of settling. This authored
tool trajectory exists only in the test; it is not avatar movement evidence.
Measured lift is 0.39971 m, released drop 0.40528 m, peak applied force 3.226 N,
peak stretch ratio 1.07791 and tail maximum speed 0.00953 m/s. Particle
penetration is below 1e-15 m. The material values remain uncalibrated.

Four Blender previews render the recorded Rust positions at 0, 2, 3.3 and 13
seconds, with no Blender cloth solve or position repair. Visual inspection found
the mattress crossing through the sheet at its edges. The preview now measures
the authored mattress mesh against each sheet render triangle's centroid and
three edge midpoints: **0 vertex penetration but 6.93–10.03 mm sampled triangle
penetration** across those four frames. Thus vertex-only clearance is insufficient
and the overall sheet-interaction acceptance fails. These finite samples are
not a continuous-collision certificate. This is a collision problem to address
before avatar handling, independently of the deferred visual polish.

Evidence is under `artifacts/toronto-condo-captures/grip180/`: `first/` contains
the original successful solver trace, and `preview-audited/` contains the four
screenshots and failing authored-surface clearance measurements. The initial
preview and all logs remain preserved. The library suite passes 307 tests with
two ignored; Python still reports six passes and the existing root-ownership
text-check failure. The quality gate retains its 23 previous major findings;
the new solver-step complexity finding was resolved without changing behavior.
Blender wrote all images and metadata before its known audio-shutdown hang;
the completed render process was then interrupted.

Next: triangle-aware mattress contact, then runtime contact/closure admission,
clock-matched observed hand coupling and unchanged beautiful_assistant
lift/release evidence. Self-collision, body/dynamic-object response and actual
finger grip/sliding remain incomplete.

## Checkpoint 181: triangle-aware fixed-scene contact

`cloth_collision.rs` adds discrete triangle/convex contact alongside particle
contact. It queries in collider-local coordinates, captures a barycentric
material witness, rechecks its signed distance against the analytic collider,
and distributes normal/friction response by inverse mass. A contact witness
alone cannot apply a force when the material point is actually clear. Bounds
are refreshed after a collider changes the cloth; a convex supporting-plane
test skips triangles already wholly clear. No scene geometry, material
parameters, avatar transforms or original motion packets were changed.

The first run rejected a Parry f32 projection with a -2.83e-10 barycentric weight
at a vertex. The solver now accepts only bounded numeric roundoff, normalizes
the weights and reevaluates the actual contact point; larger invalid weights
still fail. A second full trial lifted/released successfully but failed the
existing settling threshold (0.06883 m/s versus 0.05 m/s). Both failed runs and
their source snapshots remain under `cloth181/`. Bounds reuse and supporting-
plane rejection remove redundant queries without increasing physical clearance.
The revised fixture passes: lift 0.39974 m, released drop 0.39997 m, peak applied
force 2.518 N, peak stretch ratio 1.02353, and tail maximum speed 0.04284 m/s.

The Blender preview can now use the exact runtime triangle indices instead of
implicitly triangulating quads. An expanded audit runs against the authored
mattress mesh at all 131 stored frames, with each triangle's centroid and three
edge midpoints plus the vertices. On **identical topology and sampling**, the
checkpoint-180 trace reaches 10.59 mm penetration; the new trace has zero
sampled penetration. Four new previews show the lifted and released edge
without the previous mattress strips crossing through the sheet. This is
1,207,296 triangle samples, not a continuous-time or whole-surface certificate.
The unchanged clamp trajectory is a diagnostic tool fixture, not avatar motion.

Four new tests cover triangle-interior overlap with clear corners, transformed
colliders, local mass-distributed response with dense independent samples, and
bounded projection roundoff. An initial new test incorrectly required exact
floating-point sum equality; it now checks f64 machine precision. Existing
ownership and acceptance thresholds were not weakened. The full library suite
passes 311 tests with two ignored; the existing Python ownership text-check
failure remains. The quality gate retains 23 previous major findings, with no
new collision-code complexity or verbosity finding.

The opt-in live cloth path uses these same triangle contacts. Remaining limits
include swept collision, cloth self-collision, dynamic/body response, runtime
finger/closure admission and actual beautiful_assistant sheet manipulation.

The rebuilt renderer records 720 bedroom frames at 1920×1080 (24 encoded
seconds, 51.26 seconds wall time; not real-time performance acceptance).
The sheet advances 1,303 fixed ticks / 21.7167 seconds and pauses with the
original native clock. All 719 observed CPU surface/mesh digests agree;
622 repeated-tick observations stay unchanged. Observed peak stretch is
1.02567, particle penetration is zero and final-half-second maximum speed is
0.01932 m/s. Initial sampled peak particle speed is 6.19 m/s, so fabric/energy
calibration remains unfinished. Original root interpolation error stays below
9.35e-8 m. The video fully decodes, and inspected frames no longer show the
previous mattress-edge penetration bands. The avatar is loaded but remains
outside the bedroom view; this is live cloth validation, not avatar grasping.

## Checkpoint 182: observed fingers, no automatic closure

`bevy_app/cloth_contact.rs` publishes per-hand and per-finger geometry under
`SceneState.cloths[*].hand_contacts`. It reads distal bone global transforms
after transform propagation and after cloth stepping, then queries the current
surface using the captured rig's distal capsule approximation. Observations
include segment endpoints, radius, material triangle/barycentric coordinates,
normal, separation and surface revision. Missing rigs, invalid geometry,
missing/failed motion, and clock/session mismatch are explicit unavailable
states; old contacts are cleared. The observer has no writable body or cloth
access and creates no attachment or force.

Inspection found `office::hand::conform_fingers` still registered in production.
It synthesized finger rotations around held objects, so its apparent contact
could not establish model-generated closure. That system and its state/timing
code have been removed; the captured rig geometry and historical calculation
helpers remain. The pre-change source is preserved in
`contact182/before-hand-source.json`. No automatic closing replacement, new
body target, pose fallback or renderer correction was added.

The current `SkeletalPose` envelope carries 22 SMPL body quaternions and no finger
rotations. **A generated pinch is not supported by this input.** Read-only
capsule contact is not pinch/force admission, skin collision certification or
proof of finger closure. Capsule radii and tip extension are the existing rig
approximations; exposure, opposing pads, layer choice and a validated generated
finger-motion input are still needed before enabling sheet grasps.

Four new tests cover observed orientation versus proximity, nonfinite geometry,
read-only ECS behavior, missing rigs, stale clock/session results and absence of
a finger writer in the installed office schedule. The full library suite passes
315 tests with two ignored. The existing Python ownership source-text check
still fails (six pass); it and other ownership thresholds were not weakened.
The quality gate retains 23 previous major findings, plus informational test
fixture duplication and previously known system-parameter findings.

A rebuilt living-room capture uses beautiful_assistant and the unchanged native
recording: 119 motion-bearing snapshots contain 1,190 finite finger observations,
with distal segment lengths 10.71–19.19 mm and zero sheet contacts, as expected
with the avatar in another room. Root interpolation error is below 6.69e-8 m;
surface/mesh hashes and content-clock checks pass. The video fully decodes and
the inspected frame shows the avatar, window/balcony and lake. Positive contact
is covered only by synthetic geometry/ECS fixtures, not a live avatar grasp.
The 120-frame/four-encoded-second capture took 30.96 wall seconds, with median
frame arrival about 345 ms. Runtime performance is still a substantial limit;
this is not real-time interaction acceptance.

Evidence and the new `audit_cloth_contacts.py` report are under
`artifacts/toronto-condo-captures/contact182/` and gallery 182. Next steps include
profiling the measured live slowdown, a model-owned finger-motion input and
explicit physical pinch admission. Other furniture articulation remains in the
deliverable sequence above; the read-only observer does not complete it.

## Checkpoint 183: measured cloth bottleneck; both speedup trials reverted

The previous live slowdown is reproducible: the same 120-frame living-room
capture takes 31.40 wall seconds, with median frame arrival 344.99 ms. Temporary
timers across 230 updates, including warmup, measure 29.23 seconds in cloth
advance and only 0.051 seconds in the read-only hand observer. `perf` is blocked
by the host's existing permission setting; the investigation did not change it.

The actual condo clamp fixture (1,980 steps, 33 simulated seconds including
pre-settle) attributes 39.22 seconds to collision, 4.10 seconds to springs/clamp
constraints, and 0.16 seconds to velocity reconstruction. A narrower diagnostic
measures 18.22 million support-map contact queries in its first 1,900 steps.
Those queries consume 23.43 seconds of the 35.44-second triangle phase in that
interval. This is measured CPU cost, not a GPU timing estimate.

Two bounded trials are preserved and rejected:

- Concrete box dispatch produces byte-identical fixture trajectories, but test
  duration increases from 43.82 to 44.79 seconds. No speedup is retained.
- Seeding GJK with the measured collider gradient reduces collision time to
  36.94 seconds (about 6%), but changes the trajectory and fails the existing
  settling limit: tail maximum speed 0.05350 m/s versus the required 0.05 m/s.
  The original achieves 0.04284 m/s. The failure was not waived or repaired.

All trial code and temporary timers were removed, and five observed source
files were restored byte-for-byte. Source snapshots, timing logs, full fixture
trajectories, the failed test output, a comparison plot, and the new living-room
preview are under `artifacts/toronto-condo-captures/perf183/`. This investigation
does not add avatar grasping or improve production performance. The next
performance change should address repeated triangle/collider queries while
preserving contact and settling behavior; changing query seeding alone did not
meet that requirement. Generated finger input and physical pinch admission
remain separate unfinished work.

## Checkpoint 184: faster development build with identical cloth traces

The measured query cost is reduced by compiling only `parry3d` at optimization
level 3 in the development profile. `Cargo.toml` records the package override;
the solver, collision routines, tolerances, scene parameters, motion handling,
dependency versions and lockfile are unchanged. Release already uses level 3;
this is a development/preview improvement, not a measured release-build gain.

The same 1,980-step condo clamp fixture takes 43.51 seconds at the previous
setting and 36.03 seconds with the Parry override, about 17% less time. The
saved `positions.json`, `observations.json` and `report.json` are byte-identical.
A second trial also optimized the renderer crate: it takes 36.56 seconds with
the same outputs and provides no additional measured benefit, so that override
is not retained. These are short local runs, not a sustained performance claim.

The complete library suite passes 315 tests with two ignored. The existing
Python ownership source-text failure remains (six pass), as do the 23 existing
major quality findings. No acceptance or ownership gate was changed. Sources,
build settings, test output and complete comparison traces are preserved under
`artifacts/toronto-condo-captures/perf184/`.

The rebuilt living-room preview takes 25.70 wall seconds for 120 frames, versus
30.96 seconds in checkpoint 182; median frame arrival falls from 344.85 to
285.14 ms. Both runs complete 1,303 cloth ticks and end with the same surface
hash. The clock/mesh/root audit passes, with root interpolation error below
8.15e-8 m, zero observed particle penetration and tail maximum speed 0.01932 m/s.
All 1,190 finger observations are finite and report no contact with the distant
bedroom sheet. The video fully decodes and the final view was inspected.
These timings include capture/encoding overhead and are not GPU timings.
The four encoded seconds still require 25.70 wall seconds; real-time interaction,
generated finger input and physical pinch admission remain unfinished.

## Checkpoint 185: passive bedside drawers beside the deformable sheet

`scene-drawers.json` extends the fridge-and-sheet scene with a hollow bedside
cabinet and two independently constrained drawers. Each drawer has a handle,
an open cavity, horizontal prismatic rails and 0–0.40 m travel. Hinges use radians;
sliders use metres. Mixed units, tilted slider axes and actuator fields are
rejected. The original solid bedside obstacle and its seven visual nodes are
replaced by the articulated assembly.

The sheet collides with the fixed cabinet shell boxes rather than the old solid
cabinet block. Fixed-part references reject moving parts: cloth coupling to
moving drawers remains unsupported. Bed sheets remain part of the interaction
scope: material-point grasping, pulling, release and settling, with generated
avatar contact still required before claiming usable manipulation.

The full library suite passes 319 tests with two ignored. New checks exercise
side loads, obstruction, opening, release, closing, rail limits, hollow geometry,
schema rejection and navigation invalidation after an observed drawer moves.
The Python suite has six passes and its existing ownership source-text failure;
no ownership gate was relaxed. The physics fixture applies diagnostic external
forces, not avatar motion. Four Blender previews render recorded physics poses,
including the obstructed state, from the 840-frame trace under
`artifacts/toronto-condo-captures/drawer185/`. These are mechanics evidence,
not proof that the avatar can open a drawer or grasp a sheet.

The rebuilt bedroom capture records 120 frames with the cabinet and sheet in
the same scene. All 30 asset hashes match; the three bedside parts remain at
their closed poses, and both drawers remain navigation obstacles. The cloth
clock/mesh audit passes through 1,303 simulation ticks, with zero observed
particle penetration and tail maximum speed 0.02587 m/s. The recorded native
avatar root is unchanged within 6.32e-8 m. Hand observation checks pass with
no sheet contact; beautiful_assistant is outside this bedroom camera view.
The video fully decodes. Both mechanics renders and the bedroom recording
are archived in the project progress gallery as prototypes.

## Checkpoint 186: entry cabinet and accurate bounds at partial hinge angles

The entry storage block is replaced with a hollow oak carcass, a divider, six
shelves and two separately hinged door/handle assemblies in `scene-cabinet.json`.
The left door uses -90–0 degrees and the right uses 0–90 degrees. There are no
motors or exported opening animations. Door masses are initial estimates and
the ideal hinges have no latch. Six original visual nodes and the old aggregate
collider are replaced. The cloth's fixed-collider references now use the shell;
moving door/cloth coupling remains unsupported.

The conservative room sweep performs 143,352 box-pair checks, with angular
inter-sample movement bounds plus a 1 mm reserve. Minimum separating gap is
8.05 mm; independently moving door envelopes retain a 5.85 mm X gap. An inserted
block is correctly rejected. In the actual Rapier fixture, diagnostic torques
open both doors to 90 degrees, an obstruction stops the left door at 29.13
degrees, removing it permits full opening, and both doors release, close and
settle. This is mechanism evidence, not generated avatar manipulation.

The partial-angle fixture exposed an observation bug: published compound AABBs
rotated a single enclosing local box, filling empty corners between the panel
and handle. Bounds now transform each compound leaf before merging. The existing
exact-published-bounds assertion remains unchanged, and a rotated/scaled compound
regression verifies the fix. No body motion or physics actuator was added.

The library suite passes 322 tests with two ignored. Final exported assets pass
the two cabinet tests again; their 720-frame physics trace is byte-identical to
the full-suite run. Python retains its existing source-text ownership failure
(six pass). Failed manifest validation, the partial-angle bounds failure and
the re-export digest review are preserved under
`artifacts/toronto-condo-captures/cabinet186/`.

The quality delta remains nonclean. New reported duplication includes explicit
cabinet dimensions and separate fixture phase assertions; the shared assembly,
panel, export and studio helpers are reused. The test driver and the enclosing
Rust test module also exceed descriptive size/complexity heuristics. These
findings are retained for review, not suppressed or used to weaken acceptance.

The rebuilt entry-camera capture records 120 frames with all nine mechanism
parts registered and all six constrained parts retained as navigation obstacles.
Observed bounds match transformed component boxes within 4.04e-7 m; all 33 asset
hashes match. The cloth clock/mesh audit passes through 1,303 ticks, with zero
observed particle penetration and tail maximum speed 0.01458 m/s. Native root
interpolation error stays below 8.67e-8 m; hand observations report no sheet
contact. The video fully decodes. The avatar is outside this camera view and
the cabinet remains unforced and closed; opening evidence comes only from the
separate diagnostic physics fixture. Both are archived as prototypes.

## Checkpoint 187: bedroom passage controlled by a passive door

The bedroom opening now has a separate full-height door leaf with lever geometry
on both faces and a 0–90-degree inward hinge. The existing jamb is preserved as
a fixed mechanism part, replacing its original collider and visual node. The
mass is an estimate; the hinge is ideal, the levers do not rotate, and there is
no latch. The current single-region grasp schema exposes only the room-side
lever. The bathroom and balcony leaves remain unfinished.

The full-condo navigation fixture proves that the closed door blocks passage
from `[2.85, 0, -0.55]` to `[2.85, 0, -1.85]`, opening restores the route, and
closing invalidates that route preview. Even while marked held, the door stays
a navigation obstacle. The query does not change avatar motion or revision.
This is read-only geometry evidence, not an avatar walking demonstration.

The actual Rapier fixture records 1,080 frames: unforced closed, obstructed
opening, unobstructed opening, release, closing and released settling. The
blocker stops opening at 23.71 degrees. Opening reaches 90 degrees; measured
maximum overshoot is 0.101 degrees and maximum hinge-anchor error is 0.080 mm.
The authored sweep passes 78,735 conservative box-pair checks with angular
inter-sample bounds and a 1 mm reserve. A separate full-3D separating-axis audit
checks the actual recorded poses, including drift and overshoot, against fixed
room boxes: 469,800 pairs and a minimum separating gap of 9.86 mm. It excludes
the intentional diagnostic blocker and does not certify continuous motion
between recorded frames, avatar contact or moving cloth collision.

The library suite passes 324 tests, two ignored; final re-exported assets pass
both bedroom-door tests, with byte-identical physics traces. The existing Python
ownership text-check failure remains (six pass), as does the nonclean structural
quality gate. No policy or acceptance gate was weakened. Initial cropped
inspection/capture views and the audit's initial f32-vs-JSON coordinate equality
failure are retained under `artifacts/toronto-condo-captures/door187/`; camera
framing was corrected and placement is compared after the runtime's f32
representation conversion. Physics and geometry were not changed to fix either.

The final rebuilt-scene recording shows the complete doorway for 120 frames.
All 11 mechanism parts load, all seven constrained parts remain navigation
obstacles, and all 35 asset hashes match. Published bounds agree with transformed
component boxes within 4.04e-7 m. The sheet audit passes through 1,303 ticks, with
zero observed particle penetration and tail maximum speed 0.01970 m/s; unchanged
native root interpolation error is below 9.12e-8 m. The video fully decodes.
beautiful_assistant is partially visible at the frame edge, without door contact.
Actual avatar opening, latching and passage are not demonstrated by this capture.

## Motion and evidence gates

Follow `model-owned-motion.md`: Ardy generates body motion; physics determines
object response. Asset pivots and handle/cloth regions describe world geometry,
not prescribed body poses. Existing spatial-conditioning support has limits;
unsupported contact generation remains an explicit failed/unsupported attempt.

Validate in separate stages:

1. Asset mechanics: assembly hierarchy, units, pivot/axis, stop geometry,
   cavities, accessible handles and cloth topology. Diagnostic fixture motion
   or forces must be labelled and kept separate from avatar evidence.
2. Physics: constrained response, obstruction, no-contact rejection, grip
   preservation, release, settling and current observed state. Test that grips
   never replace permanent joints and closed furniture does not contain a
   stale aggregate blocking collider.
3. Source motion: unchanged model artifacts and conditioning audit. No IK,
   root repositioning, timed furniture animation or task force controller.
4. Live render: beautiful_assistant actually acquires contact, manipulates,
   releases and leaves the requested state. Archive failed attempts as well as
   successful recordings in the existing progress gallery.

Neither this preparation pass nor a successful Blender cloth bake proves a live
avatar interaction. Screenshots, runtime observations and source-motion evidence
must agree before marking a capability supported.

## Checkpoint 188 — passive balcony slider and capture backpressure

`scene-balcony-slider.json` extends the bedroom-door scene with a 45 kg estimated
framed glass leaf on a passive X rail, travelling 0–2 m. It retains the original
right-hand glass pane and adds a separate fixed companion panel. Three GLBs and
`interaction-authoring/balcony-slider/balcony-slider.blend` are authored by
`scripts/condo/balcony_slider.py`. Glass retains the source material's 0.13 alpha;
the live view preserves the lake sightline. Only the interior pull is exposed.
There is no latch, roller model, runtime actuator or avatar manipulation claim.
The existing sill remains: route clearance is not threshold-crossing evidence.

The exact axis-aligned swept boxes, expanded by a 1 mm reserve, pass 736 fixed
obstacle comparisons with a 28.999 mm minimum gap. All 840 observed physics samples
pass 618,240 box comparisons with a 29.999 mm minimum gap. The diagnostic fixture
checks transverse loads, obstruction, full travel, release, closing, settling and
published bounds. The full-condo route preview is blocked closed and usable open,
retains held constrained objects as obstacles, invalidates stale previews after
closing, and never changes avatar motion. The existing bedroom passage check now
shares that read-only test helper. The full library suite passes 326 tests, two
ignored; existing drawer, cabinet and bedroom mechanics also pass.

A failed initial fixture used Rapier's single-substep default and exceeded the
unchanged 3 mm rail limit under the 150 N diagnostic load. The fixture now uses
`PhysicsConfig::default()` integration settings, matching the live renderer's
four substeps. No runtime solver, force, limit or tolerance was changed. Earlier
saved diagnostic traces retain their original integration settings. All attempts
remain under `artifacts/toronto-condo-captures/balcony188/`.

The first live integration capture verified 38 asset hashes, 14 registered parts,
and closed slider bounds within 0.00041 mm, but failed the root interpolation gate
by 2.039 mm. It is published as failed evidence. Investigation found capture replay
paced by wall time could overfill the 136-pose native timeline when rendering ran
slowly. `PlaybackCursor::feed` now caps its unchanged packet delivery at 500 ms
ahead of the matching observed motion clock. A slow-renderer regression verifies
bounded prefetch and all 436 original poses delivered without rewriting; all six
capture-driver tests pass. The runtime pose timeline and audit thresholds are
unchanged. The existing Python ownership suite still has six passes and its one
pre-existing source-text failure.

Quality checks remain nonclean across the accumulated worktree, including archived
source copies and explicit mechanism fixtures. The new slider fixture shares the
state/bounds observers but keeps its specific obstruction/load sequence explicit.
The capture test module grew to add the slow-renderer regression. These findings
were not blanket-acknowledged or hidden by resetting the quality baseline.

The corrected `live-paced/` capture delivers all 436 source packets, passes the
unchanged root gate at 9.23e-8 m maximum interpolation error, and verifies all 38
asset hashes and 14 bodies. Cloth advances 1,304 ticks with zero observed particle
penetration and 0.0259 m/s tail speed; the finger observer reports zero contacts.
The avatar is loaded but outside this camera: this remains scene integration,
not visible avatar handling. Gallery 188a retains the failed attempt, 188b has the
14-second, 420-frame slider opening/closing video, and 188c shows verified live
integration. Every video decodes; the mechanics frames match alternate samples of
the original 60 Hz trace without cuts or pose interpolation.
