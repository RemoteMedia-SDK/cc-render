# Base-avatar volumetric tissue

This branch implements an isolated volumetric replacement for the base
`beautiful_assistant` breast-bone springs. It remains **opt-in** on
`feat/breast-soft-tissue`: the reviewed deformation is promising, but live
playback and actual-avatar contact are not ready for default use.

The user reviewed the [boundary-pressure comparison video](../../artifacts/soft-tissue/volume-audit/tracked-off-on-boundary.mp4)
and said “Looks much much better.” That exact video remains the visual reference.
The [review page](../../artifacts/soft-tissue/volume-audit/index.html) shows physics
off/on with unchanged source motion and no displacement amplification.
The newer [localized-volume comparison](../../artifacts/soft-tissue/quality-pressure/reviewed-vs-local-volume.mp4)
retains broad movement while reducing local volume extremes; see its
[implementation and evidence](breast-soft-tissue-local-volume.md).

## Scope and ownership

ARDY owns all body/root motion. The adapter reads committed motion, samples chest
attachments and contact skin, and deforms the mesh. It does not prescribe or
repair body poses. See [motion ownership](model-owned-motion.md).

The requested base avatar resolves to:

- `../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb`
- `../remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json`
- GLB SHA-256: `23060843839654603f26d7aae6a955cf09c86bb2b39de8bf70bfc5e70d1062a9`

No separate `beautiful_avatar` asset was found. The dedicated lab uses this base
mesh and a focused front/side camera, with no clothing or condo geometry. Other
agents' worktrees are untouched.

## Current simulation

Current candidate: `assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json`.
The reviewed original remains `beautiful_assistant.base-boundary-nodal-volume-prototype.json`.
The left/right cages have 241/233 nodes, 657/605 tetrahedra and 130/128 fixed chest
attachments. An offline fit supplies the unloaded reference coordinates.

The f64 finite-element solver combines isochoric corotated shear, strain
hardening, bulk-volume resistance and objective continuum viscosity. The hybrid
pressure mode uses element pressure on tetrahedra with a fixed edge or low
authored-shape quality, and average nodal pressure elsewhere. A small element
share in the nodal partition penalizes local volume variation. The partitions
do not double-count bulk response.
This is an engineering formulation, not an anatomical calibration.

| Parameter | Selected value |
| --- | ---: |
| Density | 950 kg/m³ |
| Shear / bulk modulus | 500 / 500,000 Pa |
| Strain-hardening coefficient | 8 |
| Shear / bulk viscosity | 5 / 5,000 Pa s |
| Element-pressure quality cutoff | 0.15 |
| Element / nodal bulk share in averaged cells | 8,000 / 492,000 Pa |
| Runtime / reviewed-capture rate | 120 / 240 Hz |
| Maximum nonlinear iterations | 32 |

BDF2 advances position and velocity, with backward-Euler startup and history
restart on excessive timestep growth. Invalid or unconverged trials are rejected;
Bevy retries smaller source-time intervals. Fractional display solves operate on
copies, preserving the fixed-step state. The [current accuracy policy](breast-soft-tissue-accuracy.md)
scales BDF2 stopping limits with the effective timestep and retains the independent
convergence tests. Implicit Euler keeps its established limits.

## Rendering and contact

The asset embeds 1,670 skin samples directly in the volume. The adapter updates
mesh displacement, normals and tangents before ordinary skinning/rendering.
`CC_AVATAR_TISSUE_ASSET` selects the asset and excludes legacy breast spring
writers. When that setting is absent, the legacy path remains available. Other
secondary chains retain their existing behavior; the comparison lab disables them.

Normal body contact uses sampled kinematic skin, swept vertex/face queries and
area-weighted penalty forces. Contact refreshes re-solve from the original state;
they do not project the final mesh or alter body motion. Compression/release
fixtures pass, but the reviewed jump has **zero active contacts**. Prior arm-fold
and self-hug recordings also failed to produce contact, so they do not validate
actual-avatar compression.

Known limits include no edge/edge, mutual-breast, friction or environmental
contact, and no certificate for curved FK trajectories between sampled endpoints.
Position-affecting facial morph changes at a paused body timestamp fail explicitly;
this branch does not change the motion-clock ownership policy.

## Review evidence

The reviewed clip contains all 478 frames through 8 seconds, with unchanged
attachments and source packets. It reaches 36.15 mm peak skin displacement.
At its comparison frame, 44.85% of the skin patch moves more than 10 mm, showing
broad deformation. Whole-region volume changes remain below 0.28%, but local
volume ratios reach 0.310–2.732. The new candidate reduces the captured range to
0.562–1.533, with 32.56 mm peak movement and 40.77% of the patch moving over
10 mm at the comparison frame. Local distortion is reduced, not eliminated.

All 84 focused tissue tests pass, covering forces, viscosity, time convergence,
rollback, attachment fidelity, parallel region solves and compression/release.
The Python suite has 15 passes and one existing ownership source-slice failure;
no protected assertions were weakened. These checks do not establish anatomical
material accuracy or comprehensive contact behavior.

## Runtime measurements

The preceding dev-profile optimization delivers 170 / 174 readbacks over 7.934
source seconds in two runs, taking 8.062 / 8.048 wall seconds after the first
frame. Its p95 delivery gap is 114 / 112 ms, versus 183 ms at the preceding
checkpoint. This is still uneven playback; readback timing is not browser
presentation. The package now uses one codegen unit; its measured rebuild took
110 seconds. These shared-host runs do not establish release-build performance.

For that earlier compiler optimization, all 478 frames have identical mechanical
values to the preceding BDF2-policy capture, and differ from the strict solver by at most 0.751 um in
node position, with identical attachments. See the
[accuracy report](breast-soft-tissue-accuracy.md) and
[runtime optimization record](breast-soft-tissue-runtime.md) for exact conditions,
rejected candidates and shared-host timing limitations.
The new material candidate delivers 158 readbacks in its tracing-disabled live
run, with a 132 ms p95 gap and 228 ms end drift. It is a volume-behavior
improvement, not a demonstrated throughput improvement.

The subsequent preconditioner refresh retains that candidate's deformation to
roundoff across all 478 frames. Refreshing once after two nonlinear updates
reduces measured CPU solve time by 6–7%. Live playback improves from 165 to
195 / 196 readbacks, with p95 gaps of 73 ms versus 125 ms and end drift of
51–56 ms versus 137 ms. This remains uneven playback on a shared host; see the
latest section of the runtime record for conditions and full evidence.

Remaining work is smooth live playback, reduced local distortion and rendered
active-contact validation before enabling the replacement by default.
The [uniform pressure-share experiment](../../artifacts/soft-tissue/pressure-stabilization/README.md)
was rejected on its own: it reduced expansion but left excessive compression
in thin right-side elements. Combining a small share with geometry-selected
element pressure passes the declared volume and broad-movement screens.

## Implementation and history

- `src/cc_render/soft_tissue/`: material, implicit solve, pressure, viscosity and integration.
- `src/cc_render/bevy_app/tissue*.rs`: source-time sampling, skin embedding and body contact.
- `examples/tissue_lab.rs`: focused studio replay and frame-correlated evidence.
- [Boundary-pressure checkpoint](breast-soft-tissue-boundary-checkpoint.md): selected asset and visual baseline.
- [Preserved viscosity-era design](breast-soft-tissue-viscosity-checkpoint.md): earlier detailed derivations and evidence, with superseded settings.
- [Development history](breast-soft-tissue-history.md), [pressure study](breast-soft-tissue-pressure.md), and [paused checkpoint](breast-soft-tissue-paused.md): earlier designs and failures.

Artifacts, source snapshots, input hashes and rejected attempts live under
`artifacts/soft-tissue/`; necessary work is not left in `/tmp`.
