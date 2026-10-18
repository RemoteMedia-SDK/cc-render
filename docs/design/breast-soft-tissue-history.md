# Historical development record

Superseded checkpoints are preserved below. See [the current design](breast-soft-tissue.md) for the active implementation, parameters, evidence and remaining work.

# beautiful_assistant breast physics: review and proposed replacement

Reviewed 2026-10-17 against cc-render `ea65ebb`, Bevy 0.15 and
bevy_rapier3d 0.28 / Rapier 0.22. Status: isolated, opt-in Bevy prototype;
replacement is not enabled by default or visually accepted. Existing unrelated cloth
changes were present during review.

## Collision search reuse checkpoint

`BodyContacts` now retains its Qbvh and update workspace. Every triangle's swept
bound is submitted again for every attempted interval, including fractional
display evaluation and recursive retries. Only acceleration data is cached;
contact planes and solver state are recomputed. The captured topology is
immutable. Parry can retain a containing leaf bound, adding conservative
candidates; sorted face IDs and the existing narrow phase retain deterministic
contact processing. The moving-bound test checks against an exhaustive scan,
including repeated intervals, large displacements, backwards samples and an
empty tree. The moving-triangle compression fixture also exercises reuse.

On the unchanged 240 Hz eight-second jump, all 478 traced region metrics,
centroid offsets and maximum surface displacements match the rebuilding
baseline exactly. Summed collision-stage time falls from 18.763 to 14.448 s
(23.0%); elasticity remains 35.740 / 36.024 s. Total offline capture wall time
is 65.647 / 62.777 s. Compilation and tests overlapped the beginning of the new
capture, and other agents share the host, so these are diagnostic stage timings,
not a controlled benchmark or real-time claim. The initial variant that also
rebalanced every step increased collision time to 29.615 s and was rejected;
its source and measurements are preserved. Long-session tree quality still
needs measurement. The next major runtime target is the tissue solve itself.

The current source passes 53 focused tissue tests. The Python suite remains
eight passing and the existing root-acceptance source-slice failure; that gate
was not weakened. Reports are in `artifacts/soft-tissue/temporal-integration/`.

## Volumetric viscosity candidate

The BDF2 candidate with edge damping did not pass the avatar timestep check:
240/480 Hz differed by 32.48 mm in surface envelope and 18.41 mm in centroid,
with sustained oscillation after landing. Its 8/32-iteration comparison differs
by less than 0.0009 mm, so an increased nonlinear solve budget does not explain
that discrepancy. This candidate is preserved as evidence and is not promoted.

The next candidate uses isotropic finite-strain Kelvin-Voigt dissipation per
tetrahedron. `Material.viscosity` has separate shear and bulk coefficients in
Pa s. It is mutually exclusive with the topology-dependent edge damping rate.
With Green strain `E = (F^T F - I)/2`, the incremental viscous potential is
`V/tau * [eta * ||dev(E-E_history)||^2 + 0.5*zeta*tr(E-E_history)^2]`.
History uses the active BDF coefficients. Forces are the exact energy gradient;
the nonlinear solve uses a PSD Gauss-Newton strain tangent. Both the gradient
and the independent strain-Jacobian quadratic form are checked numerically.
Different rigid orientations in successive states leave the strain unchanged,
so the law does not damp a rigid-body rotation or translation. Its volume
weighting gives consistent dissipation density for uniform affine strain.

An independent one-degree-of-freedom tetrahedron verifies the combined dynamics.
For a fixed base and an axially moving apex, its elastic force is exactly linear
and its viscous force is proportional to `(1+x/L)^2*x_dot`. A separate scalar
RK4 integration at <=10 microseconds provides the reference without calling
the tissue force code. Maximum state errors at 240/480/960 Hz are respectively
0.02169/0.00608/0.00160 mm. Objectivity, invalid/double-counted damping, contact
compression/release, and 30/60/120 FPS playback/pause tests also pass: 52 focused
tissue tests total. The full Python suite still has the existing root-acceptance
source-slice failure; the overall quality gate remains non-clean at eight
major findings.

`beautiful_assistant.base-viscous-prototype.json` retains the elastic fit and
uses BDF2, shear viscosity 15 Pa s and bulk viscosity 5000 Pa s, with edge damping
disabled. These correspond to an engineering time coefficient of 0.01 s times
the existing shear/bulk moduli. They are provisional simulation parameters,
not measured human tissue values.

The complete unchanged eight-second jump now has 478 matching frames at both
240 and 480 Hz. The maximum surface-envelope difference is 0.786 mm and maximum
centroid-vector difference is 0.432 mm, compared with 32.48 / 18.41 mm for the
preceding edge-damped BDF2 candidate. Whole-sequence peak skin displacement is
37.85 / 38.17 mm; minimum element volume ratio is 0.8553 / 0.8588. No contact
samples activated. Source inputs and encoded video timestamps match. The raw
frames at the largest envelope difference (source 850 ms) were inspected.
The [current comparison](../../artifacts/soft-tissue/temporal-integration/index.html)
provides normal and quarter-speed paired video, plots, and measurements.

This is a two-rate sensitivity result, not a converged reference or anatomical
calibration; envelope and centroid agreement does not bound every vertex's
error. The 120/240 Hz maximum envelope and centroid differences are 2.072 / 1.133
mm, decreasing to 0.786 / 0.432 mm at 240/480 Hz. At 240 Hz, increasing the solve
budget from 8 to 32 changes the envelope by at most 0.020 mm and centroid by
0.048 mm. All comparisons contain the same 478 frames and unchanged source.
The current side camera has exactly the same tissue trace as the front camera;
the combined front/side video has verified source timestamps. The peak side
frame (source 7216 ms) was inspected. Runtime performance, broader active contact
validation and the body-pause/facial-motion policy remain pending.
The 480 Hz debug capture took 97.44 wall seconds
for eight source seconds including capture overhead; it is not a real-time
benchmark. The candidate remains opt-in and production defaults are unchanged.

## Second-order integration candidate

An independent rigid-translation oscillator now isolates temporal error from
the tetrahedral constitutive model and strain damping. Each cube vertex has a
mass-proportional compliant attachment, so every point follows the same 6 Hz
harmonic motion without changing the cube's shape. The continuous undamped
solution retains its initial 10 mm amplitude. After 0.5 seconds, the existing
implicit Euler solver gives 0.594, 2.317 and 4.781 mm at 120, 240 and 480 Hz.
Those results match the closed-form backward-Euler recurrence: substantial
numerical dissipation exists even when physical strain damping is inactive.
An isolated edge dashpot separately matches exponential decay and conserves
momentum, distinguishing that operation from the elastic integration error.

`Material.time_integration` now permits an explicit `bdf2` candidate; its serde
default remains `implicit_euler`. The new base asset changes only that field
from the prior morph-compatible strain-stiffening asset. BDF2 differentiates
both position and velocity, with coefficients derived from the actual previous
and current intervals. The implementation agrees with the standard variable-step
coefficients documented by [Berkeley Lab's BELFEM](https://belfem.lbl.gov/doc/fem_kernel_bdf_timestepping_theory.html).
The first step uses backward Euler. A step growth above 2 restarts the derivative
formula for that step, preserving positions and velocities. Rejected steps do
not commit history; fractional display evaluation retains its existing cloned
state, and a session reset initializes fresh history.

For BDF2, edge strain-rate damping is assembled in the implicit solve rather
than applied afterward. With effective interval `tau`, the edge contribution is
`0.5 * c / tau * (length_new - length_history)^2`, where `length_history` uses the
same BDF coefficients and `c = damping_rate / (maximum_degree * inverse_mass_sum)`.
The exact gradient acts only along the current edge; a PSD tangent preserves
descent during compression. Length invariance prevents a rigid transform from
introducing damping strain. This retains the prior dashpot's continuous-time
coefficient, but it is still topology-dependent engineering damping, not a
measured tissue viscosity.

At 240, 480 and 960 Hz, the BDF2 oscillator retains 9.657, 9.932 and 9.986 mm.
Maximum state error falls from 1.496 to 0.387 to 0.097 mm, approximately fourfold
per halved timestep. Alternating step lengths of 0.8/1.2 times nominal show the
same order. These are independent reference-case results, not avatar convergence
or real-time proof. `artifacts/soft-tissue/temporal-integration/` preserves the
logs; the candidate asset is `beautiful_assistant.base-bdf2-prototype.json`.

## Timestep sensitivity: rate selection remains unresolved

The same full eight-second studio jump now has 120, 240 and 480 Hz captures,
with 478 matching display timestamps in every run. The body source, material,
camera and solver iteration budget are identical. Facial weights are neutral;
the newer asset only removes the prior inactive-morph guard. No contact activated.

| Measure | 120 Hz | 240 Hz | 480 Hz |
| --- | ---: | ---: | ---: |
| First landing window peak surface displacement | 34.14 mm | 35.14 mm | 34.08 mm |
| Whole-sequence peak surface displacement | 42.42 mm | 43.56 mm | 43.48 mm |
| Minimum element volume ratio | 0.8779 | 0.8635 | 0.8660 |

Peak agreement hides substantial differences during individual frames. The
120/240 comparison has maximum surface-envelope difference 11.35 mm and
maximum centroid-vector difference 5.01 mm. The 240/480 comparison still has
11.14 mm and 5.07 mm, respectively. These measurements do not bound every
vertex, and the worst difference is not shrinking appreciably across these
rates. **None of these rates is established as converged.** Do not promote the
120 Hz tuning or assume that switching to 480 Hz completes validation.

The [numerical review](../../artifacts/soft-tissue/morph-contact/index.html)
contains source-timed paired videos, response plots and machine-readable
measurements. `compare_steps.py` reproduces both comparisons (argument `480`
for the second pair) from the preserved PNGs and traces. Both source timestamps
and encoded video timestamps are checked. These offline runs are not performance
benchmarks; other builds were active during part of the 240 Hz capture.

The comparison videos now retain the full 900-pixel capture height. Inspection
at source time 7800 ms found that the earlier 560-pixel crop cut off the chest
during the deep landing crouch. Those earlier review copies are preserved with
`-crop600x560` suffixes; the default videos were regenerated from the original
frames/videos and their source PTS rechecked. The corrected worst-frame image
is `morph-contact/timestep-worst-frame.png`.

The next numerical check should isolate temporal integration and the split
edge-strain damping using an independently solvable response fixture. The
current solver uses backward Euler and an explicit post-solve edge damping
pass; numerical damping and the 20 Hz source's acceleration changes are
possible contributors, not proven explanations of this result. The general
step-size/dissipation relationship is discussed in
[Small Steps in Physics Simulation](https://matthias-research.github.io/pages/publications/smallsteps.pdf),
but that XPBD study does not validate this FEM implementation. Retain unchanged
source motion while investigating; smoothing or repairing the jump would change
the test input.

## Facial contact geometry checkpoint

The new opt-in `beautiful_assistant.base-morph-contact-prototype.json` retains
the strain-stiffening asset unchanged except for removing its inactive-jaw
requirement. Previous assets retain that requirement. `tissue_morph.rs` decodes
position deltas directly from Bevy 0.15's loaded morph image, then applies the
observed weights before the same skinning transform used for contact sampling.
The imported torso has three position-affecting targets; the arm primitive has
four. Negative weights are supported without clamping. Morphs overlapping the
fixed tissue embedding or its normal stencil are rejected explicitly.

Observed geometry weights are retained on the body presentation clock and
interpolated only between observed endpoints for internal substeps. This does
not assume that facial PTS is skeletal PTS, and it cannot reconstruct facial
events dropped before presentation. The retained history includes the left
endpoint of the uncommitted fractional timestep. Exact endpoint weights are
preserved. Surface/channel count changes and extrapolation fail explicitly.

The unchanged jump completed with the existing separate facial diagnostic
`jawLeft=1`: 478 GPU-stamped frames through 8000 ms, all 160 body packets, and
matching video PTS. Maximum sampled skin mismatch was 0.001971 mm. Region
states were identical to the neutral-face capture, as expected because no
contact activated in this jump. This is imported-asset integration evidence,
not evidence of a jaw-induced contact response. The video, logs, exact asset
delta and measurements are in `artifacts/soft-tissue/morph-contact/`.

Changes to geometry-affecting facial weights while body time is paused still
fail explicitly: there is no advancing interval on which to solve contact.
The existing motion-clock pause policy is preserved pending owner clarification
about independent secondary-tissue settling. Dynamic facial contact, paused
speech, broader collision coverage and runtime cost remain open. The replacement
is still opt-in.

Validation: 40 tissue tests pass, including official Bevy morph-image layout,
position-before-skinning, signed weights, endpoint/history retention, unchanged
embedding and malformed topology checks. The Python suite remains 8 passing
with the existing root-acceptance source-slice failure. The overall quality gate
has 12 major findings; the added finding normalizes a small diagnostic count
method into a clone of an unrelated count method. This is not a clean quality
or merge result. See the saved reports rather than interpreting unit success
as complete visual/contact acceptance.

## Strain-dependent material candidate

The studio identified a material-model issue rather than an iteration-count
issue: replaying the identical jump at 32 instead of 8 nonlinear iterations
changed the maximum surface-displacement envelope by at most **0.001085 mm**.
Both produced approximately 79.14 mm peak displacement. The original material
and all source captures remain available as references.

`Material.strain_stiffening` now adds a nonnegative, dimensionless quartic term
to the corotated strain energy. With `q = ||F - R(F)||²`, the energy density is
`mu * (q + 0.5 * a * q²) + 0.5 * lambda * (J - 1)²`. The coefficient defaults
to zero, preserving existing assets. Positive values strengthen large strain
without introducing a positional limit, mesh clamp, root change or bone writer.
The exact energy gradient and a positive semidefinite frozen-rotation tangent
are assembled into the same implicit solve, including normal contact.

This is an engineering constitutive model, **not** a fitted anatomical material.
The motivation for nonlinear response is supported by experimental work such as
[Dempsey et al., 72 ex vivo breast tissue samples](https://pubmed.ncbi.nlm.nih.gov/34496308/),
which evaluated several hyperelastic models. That study does not validate this
quartic corotated model or the chosen coefficient of 8. Skin/fascial anisotropy
and measured viscoelastic calibration are not represented by that coefficient.

The new asset is `assets/tissue/beautiful_assistant.base-stiffening-8-prototype.json`.
`tissue_fit` refitted its unloaded reference coordinates under gravity; maximum
equilibrium shape error is 0.141 mm, with per-node force residual below 0.000014 N.
The displayed initial mesh, fixed attachment footprint, topology and source GLB
were retained. Four near-zero barycentric entries differ only through JSON float
round-tripping, by at most 2.47e-32; there is no intentional embedding change.
The input, material choice and fitting log are in `artifacts/soft-tissue/hardening/`.

The updated [interactive review](../../artifacts/soft-tissue/lab-review-hardening-8/index.html)
shows original versus nonlinear material in both cameras. Its
`material-comparison.mp4` and `material-response.png` compare the same source
frames. `comparison.mp4` retains the nonlinear-versus-undeformed comparison.

Measured results for this one preserved sequence:

| Measure | Original | Strain stiffening |
| --- | ---: | ---: |
| Peak surface displacement during first landing window (1.3–1.9 s) | 79.14 mm | 34.14 mm |
| Peak over all eight seconds | 79.14 mm | 42.42 mm |
| Centroid enters the documented 2 mm / 0.5 s settling band after first landing | 0.700 s | 0.433 s |
| Minimum element volume ratio over the sequence | 0.8315 | 0.8779 |

These are mesh/centroid diagnostics, not clinical or human-motion validation.
No active contact occurred. Both cameras reproduced identical trajectories;
all 478 frame timestamps matched and were verified again after video encoding.
A separate 32-iteration nonlinear run differed from the 8-pass result by at
most 0.001174 mm in surface-displacement envelope and 0.000611 mm in centroid
components. The largest deformation was inspected in front and side views.

Validation: 37 tissue tests pass. The added checks cover strain-dependent
energy objectivity, unchanged small-strain modulus, invalid parameters,
finite-difference force gradients and independent sparse-tangent products.
The Python suite remains 8 passing / 1 existing source-slice failure, and the
overall quality gate remains non-clean at 11 findings. Full logs and numerical
comparison reports are retained under `artifacts/soft-tissue/hardening/`.

This candidate remains opt-in. Facial/contact source-time compatibility,
broader contact coverage, timestep sensitivity and real-time performance remain
open before replacing the production default. Source motion remains 20 Hz;
the 60 Hz video interpolates its samples without repairing their acceleration.

## Dedicated studio benchmark (current review)

The owner requested a focused jump test instead of condo-based acceptance. The
isolated `examples/tissue_lab.rs` now uses the base avatar, an empty studio, fixed
front/side cameras and unchanged native ARDY packets. The empty office resource
retains the existing stream validation and observation; no home/condo package is
loaded. Legacy secondary bone physics is disabled in **both** comparison modes.
Tissue-off means the original skinned mesh; tissue-on uses the opt-in volumetric
adapter and the same skinning, camera, material and body samples.

`RendererConfig.offline_frame_step` selects Bevy's fixed `ManualDuration` clock
for this diagnostic only. Production defaults remain automatic. Each render
advances 1/60 simulated second; tissue integrates at 120 Hz. Body source time is
extracted with the mesh as `BodyFramePts` and travels through GPU readback as
`RenderedFrame.body_pts_ms`, independently of facial and tissue timestamps.
These two new public struct fields require external Rust literal constructors
to be updated. The local constructors and renderer mock were updated.

The source recording is `artifacts/soft-tissue/lab-jump-01/`: one declared seed
(731), semantic-only Core40 generation, four native blocks, 160 original packets.
The prompt requests one jump, but the model repeats it later. All eight seconds
are preserved, including source responses and journals; no retiming, relocation,
pose repair or manually frozen settling interval was introduced.

Review artifacts are in `artifacts/soft-tissue/lab-review-01/`:

- `index.html`: video controls at normal and quarter speed, plots and measurements.
- `comparison.mp4`: fixed front/side, undeformed/tissue four-panel crop.
- `comparison-slow.mp4`: quarter-speed presentation of the same simulated frames.
- `response.png`, `measurements.json`: source-time response and definitions.
- `landing.png`: inspected peak-displacement comparison frame.

Each view has 478 frames from source 50 through 8000 ms, with maximum gap 17 ms.
Every individual video PTS was verified against the recorded source times, and
the comparison retains the same 478 frames. All four packet files are byte
identical. Source packets equal the saved semantic-only service responses.
The two cameras produce exactly identical tissue region traces, including
positions, energies and volume ratios. PNGs and per-frame traces are retained in
`lab-front-off-01`, `lab-front-on-01`, `lab-side-off-01`, `lab-side-on-01`.

The first source flight is observed at 0.8–1.3 s, using both toe heights exceeding
their initial medians by 30 mm. This is a 20 Hz source observation, not a measured
CC5 sole-contact event. Peak rendered surface displacement is **79.14 mm** at
1.416 s. The mean free-node displacement returns within 2 mm of the fitted initial
shape for 0.5 s starting 0.7 s after that source landing; this centroid measure
does not imply every surface point has settled. Minimum tetrahedral volume ratio
is 0.8315, maximum sampled-skin mismatch is 0.00197 mm, and no contact was active.
This does not validate compression/contact behavior.

Inspection exposes excessive stretching at landing and oscillation driven partly
by the 20 Hz motion's piecewise interpolated acceleration. Material/attachment
calibration and time-step/convergence comparisons remain necessary. The curve
and video are reproducible diagnostics, not anatomical validation. CPU rendering
plus solving took about 42 seconds for the front tissue-on run including startup;
the fixed clock deliberately preserves frames and is **not** a real-time result.

Reproduce after building `cargo build --offline --example tissue_lab`:

```sh
target/debug/examples/tissue_lab \
  ../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb \
  ../remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json \
  artifacts/soft-tissue/lab-jump-01/packets.ndjson \
  assets/tissue/beautiful_assistant.base-contact-prototype.json front NEW_OUTPUT
```

Use `off` for the tissue argument or `side` for the camera. Each output must be
new. `scripts/avatars/record_tissue_jump.py` records the declared source using the
existing SDK's managed Python and cached model; it refuses an existing output.
`scripts/avatars/review_tissue_lab.py` verifies source identity, camera invariance,
sampling and encoded timestamps before creating the comparison/report.

Validation: 36 tissue tests and 5 native replay tests pass. The full Python suite
has 8 passes and the same existing root-acceptance source-slice failure. The
overall ripwire quality gate still fails; its expanded capture API and broad
pre-existing prototype findings are not a clean merge result. Logs, build output
and source ownership evidence are retained under `artifacts/soft-tissue/lab-validation/`.

### Implicit solve warm start

The current solver starts Newton iterations at velocity-extrapolated positions,
while retaining gravity in the implicit-Euler inertial target. This changes the
initial guess, not the equations. A solve may stop when the free-gradient norm is
at most 0.1 mN **and** every Newton correction is at most 0.1 micrometre; the
configured iteration cap remains eight. A free-fall regression verifies that
gravity and implicit-Euler motion remain correct.

The production compression probe differs from the archived 32-pass reference by
at most 1.365 micrometres at node checkpoints and 0.000179 N in normal force.
Maximum overlap is 0.09036 mm. These checks are in
`artifacts/soft-tissue/convergence/production-comparison.json`. Alternative
preconditioners/direct solves were measured but not adopted. Timing experiments
are single-run diagnostics and do not establish a general real-time speedup.

## Current validation target: base avatar

The owner clarified that this work must use the base avatar, not the condo
outfit. The canonical base file is
`../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb`
(SHA-256 `23060843839654603f26d7aae6a955cf09c86bb2b39de8bf70bfc5e70d1062a9`).
No separate `beautiful_avatar` asset was found in the relevant checkouts.

`assets/tissue/beautiful_assistant.base-contact-prototype.json` is the linear
contact baseline; the strain-stiffening candidate above extends it.
`beautiful_assistant.base-fitted-prototype.json` remains the
non-contact reference. It retains the already fitted tissue regions and the exact base-body
binding, allows only the base GLB fingerprint, and contains no garment surface.
Its report records the source hashes and the 1,502 affected body vertices.
The original GLB, skeleton, skin weights and motion packets remain unchanged.

Garment repair has stopped. The outfit captures and geometry investigations below
are historical diagnostics; their garment defects are not acceptance blockers
for the base-avatar work. Garment support and coupled cloth are separate future
work. Remaining base-avatar work includes contact, solver convergence/performance,
smoother source-paced captures, and broader unchanged-motion validation.

The first base-avatar run is in `artifacts/soft-tissue/capture-12-base/`:
`view.mp4`, `frame-0450.png`, `frame-0899.png`, and `tissue-evidence.json`.
It completed 900 frames, replayed all 436 source packets byte-for-byte, reached
source time 21,800 ms, and bound exactly one body surface and two tissue regions.
The original GLB fingerprint was verified. The renderer reported no error;
breast springs were excluded while the remaining secondary bones stayed enabled.
The two saved motion frames were inspected as progress images, not final visual
acceptance. Fixed-30-fps encoding still lacks frame-correlated source timestamps,
so this video must not be used to certify response speed. The focused Rust tests
remain 24/24 passing; the Python suite remains 6 passing with the same pre-existing
root-acceptance source-slice failure. Logs are retained beside this capture.

### Source-paced base-avatar recording

`artifacts/soft-tissue/capture-13-timing/source-time.mp4` supersedes the earlier
fixed-fps videos for response-speed review. The tissue adapter stamps a source
time only after successful deformation. `TissueFramePts` is extracted alongside
the mesh into Bevy's render world, carried through GPU readback and the frame
bridge as `RenderedFrame.tissue_pts_ms`, then recorded in `frame-times.ndjson`.
Facial/A/V `pts_ms` is unchanged. This is an additive public frame field; external
Rust struct-literal constructors must initialize it. Both local constructors were
updated, and no external constructors were found in the workspace search.

The capture replayed all 436 original packets and produced 900 frames without a
renderer error. There are 541 unique tissue times, from 67 to 21,800 ms. The
retimed video spans 21.733 seconds plus a 1 ms final-frame duration; every encoded
presentation timestamp was checked against the recorded source offset with
ffprobe. The maximum sampling gap is 218 ms, so this is correctly paced but not
yet smooth enough for final visual acceptance. It is a second-generation encoded
review copy, with no pixel interpolation. At the final source time, body positions
differ from capture 12 by at most 0.000000127 m, consistent with float rounding.

Reproduce the timing conversion with
`python3 scripts/avatars/retime_tissue_capture.py <fresh-capture-directory>`.
The script preserves the original video and refuses to overwrite earlier output.
It uses FFmpeg's [concat durations](https://ffmpeg.org/ffmpeg-formats.html#concat)
and an explicit [encoder time base](https://ffmpeg.org/ffmpeg.html#Advanced-options)
of 1 ms. A codec round-trip test verifies both irregular timestamps and frame
pixel ordering, including removal of warmup and duplicate source times.

Validation: 24 tissue tests, 10 renderer-node tests and both new timing tests
pass. The full Python suite is 8 passing/1 unchanged source-slice failure.
Ripwire still reports 9 major pre-existing-worse findings for the overall diff;
this is not a clean quality gate. The capture callback's extra optional resource
is an intentional Bevy system-parameter change, verified by the build and GPU
run. Reports and failed codec-environment attempt are retained with capture 13.

### Source-time upper-body contact checkpoint

`tissue_collision.rs` connects the embedded normal penalty to source-time
kinematic skin. It uses the fixed area of the actual torso primitive plus the
separate arm/hand primitive (17,858 vertices), identified by exact position
hashes. Triangles incident to tissue-bound torso vertices are excluded from the
kinematic obstacle. Every skin joint and its ancestry is captured for read-only
FK sampling. At display time, all sampled contact vertices must agree with the
current skeleton-skinned vertices within 0.2 mm, or the renderer stops.

`build_body_contact_asset.py` reproduces the contact candidate from the original
fitted asset and base GLB. The generated JSON was reproduced byte-for-byte.
It preserves the existing fitted cages and embeddings; the GLB is unmodified.
The report identifies which material primitives and morph channels were audited.

The narrow phase uses f64 point/triangle witnesses and conservative advancement
for **linear endpoint vertex trajectories**. A swept Qbvh bounds candidate faces.
The pressure penalty is 1e10 Pa/m times each sample's represented rest area;
neighbouring face contacts share that area. The 0.5 mm numerical skin envelope
permits at most 0.1 mm residual overlap, checked both against the solved planes
and a second sweep of the accepted candidate through the inner 0.4 mm envelope.
Contact sets are refreshed up to four times, always re-solving from the original
physical state. Unresolved contact or inverted elements retry with source-time
subdivision (up to depth six), then fail without committing the candidate.
No skeleton transforms or rendered-position clamps are written.

The contact candidate defaults to eight nonlinear passes, based on the prior
8-versus-32-pass compression comparison. This is provisional engineering tuning,
not measured human tissue calibration. The no-contact reference remains available
by selecting its original asset. Per-frame logs report sampled skin error,
subdivisions, active contacts, peak residual overlap and peak normal force.

Important limits remain:

- This is vertex/face contact, not a complete edge/edge or mutual-tissue solver.
  Head, legs, nails, environment and friction are not included in this checkpoint.
- Sweeps certify the linearized paths only. Curved source-time FK between the
  endpoints still needs an interpolation-error bound or conservative treatment.
- The exported jaw morphs also affect shoulder skin: maximum source delta is
  about 0.233 mm on the torso and 1.163 mm on the arm primitive. Facial frames
  currently arrive through a separate latest-value clock, so sampling their
  historical geometry as if it matched skeletal time would be incorrect. The
  opt-in contact asset explicitly rejects active channels 58–61 for now; other
  expression channels remain available. Full simultaneous facial/contact support
  remains unfinished. This guard does not alter the face weights.

Validation so far: 34 tissue tests pass, including three new moving-triangle
checks and the unsupported-morph guard. The final adapter/telemetry build has
9 passing adapter tests. The Python suite remains 8 passing / 1 unchanged
root-acceptance source-slice failure. Ripwire remains at 9 major gating findings
for the overall diff; this is not a clean merge/quality result.

The completed base-only replay is in
`artifacts/soft-tissue/capture-16-body-contact/source-time.mp4`, with progress
image `frame-0450.png` and `tissue-evidence.json`. All 436 original packets are
byte-identical and were replayed through source time 21,800 ms. All reported
sampled-vs-displayed contact skin errors are exactly zero after adding the same
existing forearm-twist conversion to the read-only sampler. A dedicated fixture
also verifies its cumulative roll without writing any skeleton transforms.
The first live attempt (capture 15) correctly stopped on a 1.612 mm mismatch;
its log remains preserved. Capture 14 refused a pre-existing output directory.

Capture 16 produced 900 frames in 116.04 wall seconds. Only 94 unique source
frames were captured, from 66 to 21,800 ms, with a maximum 250 ms source gap.
Every presentation timestamp in the 21.735 s review video was verified. This
is slow, sparse **progress evidence**, not real-time or final visual acceptance.
The saved three-quarter image was inspected on the base model. Reported frame
summaries contain no active contacts, so this recording does not establish live
compression response. Moving-triangle compression is supported by the separate
numerical fixture. Performance profiling is the immediate next step.

The short instrumented capture 17 isolates the cost of four reported frame
integrations: mean skin sampling 35.1 ms, elasticity
561.0 ms, and collision queries 222.9 ms.
These are per displayed-frame integration costs (multiple 120 Hz substeps), not
per-substep timings or a controlled benchmark. Elasticity accounts for
68.5% of these measured stages, collision for
27.2%. The next performance work should address solver
convergence first and broad-phase construction second, preserving the compression
reference and the 0.1 mm contact residual test. Results and a chart are in
`artifacts/soft-tissue/body-contact/runtime-profile.json` and `runtime-cost.png`.
Capture 17 contains only the short profiling prefix, not full-motion acceptance.

### Coupled normal-contact checkpoint

`soft_tissue/contact.rs` now defines embedded skin points, unilateral contact
planes and residual-contact metrics. `Tissue::step_with_contacts` assembles their
energy, gradient and Hessian into the same implicit solve as material strain.
Contact forces therefore deform the volume through the actual skin embedding,
including its residual offset and influence. Fixed attachments remain fixed;
there is no post-solve position clamp. Invalid contact input is rejected before
committing state. The original `step` remains the empty-contact path.

The normal energy is `0.5 * k * min(gap, 0)^2`, with positive separation on the
plane's normal side. Stiffness is N/m per area-weighted quadrature sample.
Separated samples exert no force. This is a finite-compliance penalty model,
consistent with the [pressure-overclosure formulation](https://docs.software.vt.edu/abaqusv2025/English/SIMACAEITNRefMap/simaitn-c-expcontactconstraints.htm);
it is **not a nonpenetration guarantee**. Residual depth, force and contact energy
are returned explicitly. Splitting a quadrature sample requires dividing its
stiffness, which is covered by a response-invariance test.

`examples/tissue_contact_probe.rs` applies planar compression/release to the
fingerprinted base-avatar cage and its 1,502 visible-skin bindings. No avatar
skeleton is animated in this numerical fixture. The plate travels 15 mm from
an initial 3 mm clearance, at 120 Hz, then releases. The source skin quadrature
can be reproduced byte-for-byte with
`python3 artifacts/soft-tissue/contact/build-samples.py <new-output.json>`.
Run the probe with the base fitted asset, that sample file, iteration count,
rate, and optional pressure stiffness in Pa/m.

Results and the reproducible plot are in `artifacts/soft-tissue/contact/`:

| 8-pass result at pressure stiffness 1e10 Pa/m | Left | Right |
|---|---:|---:|
| Maximum visible-skin overlap | 0.0464 mm | 0.0904 mm |
| Peak normal force | 36.10 N | 35.67 N |
| Cage volume at peak compression / authored loaded volume | 99.584% | 99.576% |
| Peak node difference from 32 passes | 0.000042 mm | 0.000491 mm |
| Shape difference from authored loaded cage after release | 0.128 mm | 0.128 mm |

Force falls to zero after release. The recovered shape error is comparable to
the existing loaded-equilibrium fit error. These force values reflect the
provisional material and fixture, not measured anatomy. Lower pressure stiffness
1e8 Pa/m allowed up to 1.25 mm overlap; 8 and 32 passes agreed on that overlap,
so extra iterations alone could not fix the finite contact compliance.

The current two-pass runtime setting differs from the 32-pass contact reference
by up to 0.121 mm at peak indentation. Four passes reduce peak node error to
0.0112 mm, but the right-side transient force still differs by 1.52 N. Eight
passes agree much more closely in this fixture, with maximum force difference
below 0.000057 N. That 8-pass run took 7.82 CPU wall seconds for both cages over
3 simulated seconds in the development build: contact convergence and runtime
cost still need work. These are single-run measurements, not a production budget.

Validation is now 30 passing tissue tests, including contact energy derivatives,
Hessian action, rigid-coordinate invariance, no attraction, compression/recovery,
quadrature invariance and transactional invalid-input rejection. The initial
cube fixture exposed 0.427 mm overlap at its lower penalty stiffness; the
increased-stiffness fixture passes the unchanged 0.1 mm target. Both attempts
are preserved. The empty-contact moving-cage comparison remains within 30 nm
of the archived reference. Python remains 8 passing/1 pre-existing failure;
the broad quality gate still reports 9 pre-existing-worse major findings.

**Not connected to live body collisions yet.** Contact-plane generation from
calibrated torso/arm/hand geometry, swept detection, reciprocal tissue contact,
friction and environment coupling remain outstanding. The fixture's plane must
not become a hardcoded runtime obstacle. ARDY remains the sole body-pose owner.

## Implementation checkpoint (isolated worktree)

Work continues on `feat/breast-soft-tissue` in
`/home/acidhax/dev/personal/remotemedia/cc-render-soft-tissue`, based on committed
`ea65ebb`. The original checkout and its uncommitted cloth work are untouched.
This worktree has its own Cargo build directory. The replacement is connected
to Bevy behind `CC_AVATAR_TISSUE_ASSET`; the overall goal remains incomplete.

Implemented so far:

- `src/cc_render/soft_tissue/`: isolated world-space tetrahedral solver,
  volume-lumped masses, distributed fixed/compliant attachments, relative
  strain damping, transactional rejection of nonfinite/inverted states, and a
  versioned asset format with surface fingerprints and barycentric bindings.
- `scripts/avatars/build_tissue_asset.py`: reproducible offline cage authoring
  from the actual GLB skin buffers; preserves the GLB and source weights.
- `assets/tissue/beautiful_assistant.unfitted.json`: 256 nodes per side and
  828/819 tetrahedra. With the provisional 950 kg/m3 density, the generated
  reference volumes imply 0.709/0.704 kg. These are authoring estimates, not
  anatomical measurements. Body and v13 knit-top bindings are included.
- `examples/tissue_probe.rs`: numerical gravity/settling probe independent of
  the renderer and articulated skeleton.

Numerical investigation changed the provisional integrator. The scalar stable
Neo-Hookean constraint split left persistent fine element vibration; a scalar
co-rotated strain constraint also did not settle adequately. Nine co-rotated
strain components plus volume did settle, but local XPBD sweeps retained
appreciable convergence error. Increasing bulk stiffness alone did not fix it.
Those experiments and their results are retained in `artifacts/soft-tissue/`.

The current pure module is an **accuracy/reference prototype**, using a coupled
implicit co-rotated strain energy plus finite-volume energy, block-preconditioned
conjugate gradients, assembled sparse stiffness blocks and a line search. Analytic energy gradients, matrix symmetry,
positive quadratic response, linear residual and force equilibrium are tested.
This integrator still needs convergence and runtime-budget validation before
production selection. The old experimental XPBD projection code has been removed
from the working module; earlier measurements remain in the logs.

`examples/tissue_fit.rs` performs offline inverse rest-shape fitting, leaving
all source GLB positions and articulated bones unchanged. It uses a static
force-equilibrium solve, then updates the stress-free reference until gravity
reproduces the authored loaded cage. The checked-in
`beautiful_assistant.fitted-prototype.json` uses provisional density 950 kg/m3,
shear 1,500 Pa, bulk 500,000 Pa and strain damping 60/s. These are simulation
parameters, not measured tissue properties. With the reference solver it
reproduces the authored cage within 0.129 mm, with maximum free-node force
residual below 0.00002 N. Its standing per-tet volume ratios are approximately
0.955–1.027 on the left and 0.975–1.027 on the right. The fitted mass estimates
are 0.710/0.704 kg. No garment support or body contact is represented yet.

Before sparse assembly, a standalone optimized kernel build with opt-level 3
and the existing dependency artifacts, measured about 5.68 CPU seconds for both
cages over 6 simulated seconds at 120 Hz/two nonlinear passes, and 11.81 seconds
at 240 Hz/two passes. These are single-run kernel measurements, **not a full
renderer release benchmark**. The measured optimization below supersedes this
kernel implementation. Further changes to resolution, preconditioning or solver
iterations must retain deformation-error measurements against a reference.

The original unfitted asset is retained for reproducible baseline tests. Its
30,000 Pa bulk and 30/s damping are not runtime presets. The coarse embedding's
largest residual is about 11 mm for skin and 41 mm for the top; its exact source
position retains that offset. Surface/contact fidelity still needs visual review.

Validation to date: 14 core/reference tests pass, including a six-second real-cage
gravity fixture, and the fitted asset has a separate equilibrium regression.
The Python policy suite retains its pre-existing 6-pass/1-fail result: the failing
root-acceptance text slice includes unrelated subsequent functions. Neither that
validator nor its test has been changed or weakened. Logs are under
`artifacts/soft-tissue/`. No renderer or visual acceptance is implied by these
checks.

### Bevy integration checkpoint

The new `bevy_app/tissue.rs`, `tissue_motion.rs`, and `tissue_surface.rs` provide:

- Bounded retention of exact source knots in `PoseTimeline`, including pause and
  revision boundaries and rollback of rejected candidates. Secondary simulation
  samples every source segment, rather than interpolating render-frame endpoints.
- Read-only calibrated retarget/FK sampling. The adapter checks its sampled chest
  transform against the actual displayed skeletal transform before simulation.
  It has no mutable articulated-transform query.
- Fixed-step simulation with a fractional display-time solve on a copy. Invalid
  steps are deterministically subdivided and their attachment targets resampled
  from the same source knots. Failures are transactional; no visual clamp, pose
  correction or hidden reset is used. Session initialization is explicit.
- Exact GLB fingerprint checks, surface-position fingerprint checks, cloned mesh
  handles, and world-space displacements converted through the inverse blended
  skin matrix. The resulting mesh uses the ordinary Bevy PBR/depth/shadow paths.
  Differential surface normals and tangents retain the imported zero-deformation
  shading. This is a CPU pre-skin implementation, rather than the originally
  proposed custom GPU extension; the world-displacement equation is tested.
- Exclusion of just `CC_Base_L_Breast` and `CC_Base_R_Breast` from legacy springs
  when a validated replacement is requested. Other secondary motion remains.
- Readiness gating and explicit renderer failure on invalid simulation. The
  capture example now copies the tissue asset and notices withdrawn readiness.

Prototype invocation uses `CC_AVATAR_TISSUE_ASSET` pointing to the fitted JSON;
`CC_AVATAR_TISSUE_HZ` defaults to 120 and `CC_AVATAR_TISSUE_ITERATIONS` to 2.
These are experimental settings, not a selected final calibration. The source
GLB must be one of the asset's exact fingerprints.

Seven adapter tests pass: frame-rate independence at 30/60/120 fps, paused-state
and skeletal-transform preservation, source-corner retention, pause/revision and
rejection handling, surface displacement/normals, spring exclusion, and adaptive
step recovery/rollback. The full timeline subset passes 13 tests with one existing
ignored test; the legacy secondary-motion subset passes five. The Python policy
suite still has the original six-pass/one-fail result.

`artifacts/soft-tissue/morph-audit.json` confirms that all 64 body morph targets
have zero POSITION displacement on the 1,502 bound body entries in both source
GLBs. The knit top has no morph targets. The body and top each bind successfully
in the GPU run.

GPU evidence is in `artifacts/soft-tissue/capture-03/`. The first attempt failed
because the sandbox exposes no NVIDIA devices; a read-only host query confirmed
healthy GPUs. The second attempt bound the surfaces but exposed a tetrahedron
inversion during real motion. Its log is retained. After adding adaptive
subdivision, the third attempt completed all 436 byte-identical recorded ARDY
packets through source timestamp 21,800 ms, with no renderer failure. No other
agent's renderer, GPU process, or checkout was restarted or changed.

This is **functional integration evidence, not final visual acceptance**. The
180-frame recording took 46.62 wall seconds (approximately 3.86 capture fps,
including encoding and polling). The existing capture tool encodes those frames
at 30 fps, so its six-second MP4 compresses 21.8 seconds of source motion. It
must not be presented as normal-speed physics evidence. The saved close-ups
show the geometry path working, but seam-like highlights on the knit top still
need an identical-pose baseline comparison. There is no contact solver or
calibrated garment support yet. The completed capture, metrics, source packets,
asset, settings, screenshots and caveats are preserved in that directory.

### Sparse solve and contact geometry checkpoint

Timing counters in an isolated copy of the solver measured 6.62 seconds in
element-wise matrix products out of 8.21 seconds total for both fitted cages,
six simulated seconds at 120 Hz and two nonlinear passes. All 2,880 linear solves
hit the unchanged 128-iteration CG cap. Hardware `perf` was unavailable because
the host restricts performance events; no host settings were modified.

The runtime now assembles the same symmetric 3x3 stiffness blocks once per
nonlinear pass. Shared tet edges accumulate into one sparse coupling, and fixed
degrees of freedom are eliminated. Material energy, gradients, preconditioner,
CG tolerance/cap, line search and time integration remain unchanged. An independent
element-wise product verifies the assembly, in addition to the existing energy
gradient, symmetry and residual checks.

A paired optimized-kernel comparison used both actual fitted cages for 720 steps
each: three seconds stationary followed by three seconds of analytic translation
and rotation. This is a numerical fixture, not generated-motion evidence. The
reference took 9.022 seconds and sparse assembly took 4.159 seconds (2.17x faster),
with a maximum per-node difference of 2.98e-8 metres across the trajectory.
This is one paired run, with other compilation activity on the host, not a
statistical latency guarantee. All 15 core tests pass. The unchanged iteration cap
still limits convergence; sparse assembly does not solve that separate issue.

Reproduction code, the original kernel snapshot, raw timing counters, numerical
results and check logs are in `artifacts/soft-tissue/sparse-solver/`. Run
`python3 artifacts/soft-tissue/sparse-solver/run-comparison.py` after building the
worktree dependencies to compare the preserved kernel with current source.

The faster 180-frame GPU attempt (`capture-04`) ended after 7.43 seconds and only
154/436 source packets; it is explicitly partial evidence. The extended run
(`capture-05`) completed all 436 byte-identical source packets through 21,800 ms
without a tissue failure: 900 frames in 34.48 seconds, approximately 26.1 capture
fps including polling/encoding. This is a functional check in the full condo
scene, not an isolated renderer benchmark or certified normal-speed recording.
All 22 focused tissue/adapter/timeline tests pass. The Python policy failure and
the nine pre-existing-worse broad quality findings remain disclosed.

`capture-06-zero-displacement` is a surface diagnostic using the same fitted
asset with every embedding influence set to zero, while still simulating tissue
and excluding the old breast springs. Both final screenshots hold source pose
21,800 ms; their recorded pelvis, feet, torso direction and heading match exactly,
and their source packet files are byte-identical. The bright narrow top seam
visible in `capture-05/frame-0899.png` is absent in this diagnostic. This confirms
a displacement-dependent surface defect, so the current visible result is not
accepted. An audit found no active coincident-vertex groups at 1 micrometre
precision in either bound mesh; simple duplicate-vertex welding is not supported
by that evidence. Per-surface face alignment diagnostics are being used to
distinguish geometric folding from shading transport before changing the mapping.
The partial `capture-07-surface-diagnostic` run logs minimum old/new triangle
normal alignment per surface. The logged top alignments remained positive
(minimum 0.945); this rules out a face reversal at those samples, but does not
rule out layer crossing, body/garment clearance errors or shading defects.
The existing Bevy 0.15.3 skin-normal shader uses the inverse-transpose transform
already represented by the adapter. The visible defect remains unresolved.

Contact geometry also needs calibration. The v13 native JSON loads zero authored
body shapes, so the renderer uses the bone-length-based fallback capsules. A
bind-space audit against the actual normalized skin weights found that the
forearm capsules lie 5.8–29.3 mm outside their dominant skin vertices; roughly 14%
of dominant upper-arm vertices are outside their capsule, by up to 21.1 mm. The
chest footprint's signed gap to its capsule spans about -5.7 to +82.8 mm. These
are proxy/skin mismatches, not tissue penetration measurements. Connecting those
proxies directly would produce premature contact or miss the visible surface.
The audit reconstructs joint transforms from the GLB hierarchy and checks them
against its inverse-bind matrices; its script and JSON are preserved alongside
the performance evidence. Next contact work must fit/validate torso, arm and hand
geometry against the actual skin, then sample those read-only shapes at physical
substeps and couple their contact response into the tissue solve.

### Surface transfer and streaming initialization checkpoint

`beautiful_assistant.body-transfer-prototype.json` keeps the fitted tissue and
body embedding unchanged. The top now receives the body surface's displacement:
offline closest-triangle coordinates are composed with the three body vertices'
existing volumetric bindings. Its original geometry, skinning weights and
clearance are retained. Several tet contributions can belong to one garment
vertex; the loader rejects duplicate contributions and summed influence above
unity. The authoring script now produces this transfer for garment surfaces.
The new asset binds 4,371 top vertices with 7,534 tet contributions. It remains
a displacement embedding, not a garment force/support model.

Read-only snapshots capture the source timestamp, original/deformed world
geometry, affected triangles and skinning matrices. Request one with
`CC_AVATAR_TISSUE_SNAPSHOT=<new-file.json>` and
`CC_AVATAR_TISSUE_SNAPSHOT_PTS_MS=<source timestamp>`. The snapshot is written
once without overwriting an existing file. These diagnostics never write an
articulated transform or change the source motion.

The geometry audit at source pose 21,800 ms found no body penetration at the
visible seam (minimum queried clearance approximately 13.8 mm). The original
transfer introduced five additional edge/triangle crossings in the affected
garment region. Composing the body transfer introduced none in the same-pose
prediction or actual `capture-09-body-transfer` snapshot. The maximum measured
garment vertex-normal transport angle fell from approximately 6.16 to 0.96 degrees.
These are bounded checks at one pose, not a collision-free guarantee.

The bright seam nevertheless persists. `capture-10-imported-shading` used a
temporary diagnostic binary with imported normals/tangents and the same tissue
displacement. Its world positions differ from capture 09 by at most 0.127
micrometres, and the seam remains visible. The normal runtime source was restored
and rebuilt; imported shading is not a runtime fallback. The raw GLB bind mesh
itself has 159 detected edge/triangle crossings in the queried region; the posed
undeformed top has 125 with the expanded binding region. These counts include
multiple witnesses per intersecting face pair. The next asset task is to repair
this overlapping geometry and validate it under captured skinning transforms.
The offline `garment-repair/probe.py` experiment is not a deployed asset or
rendered acceptance result. Eight localized fairing cycles moved 1,692 vertices
by at most 6.25 mm but increased posed crossings from 125 to 258, including 230
new witnesses. That candidate is rejected and was not written into a GLB or
runtime asset. Further repair must treat the fabric layers coherently and verify
bind and posed geometry, rather than independently smoothing their vertices.

A separate streaming regression exposed a 0.818 mm difference after 100 ms when
only the first source knot was initially available. Session setup now waits for
the adjacent committed knot before initializing rigid tissue velocity, once and
before advancing physics. The regression passes with identical results for both
delivery patterns. This does not extrapolate, rewrite or smooth body motion.

All 24 focused tests pass after these changes. Captures 08–11 each complete all
436 byte-identical source packets through 21,800 ms; capture 11 uses the corrected
startup and normal shading path. They remain functional evidence with unresolved
garment geometry and no certified source-correlated video timing. Raw snapshots,
audit scripts, passing and failing regression logs, and quality reports are in
`artifacts/soft-tissue/`. The original Python policy failure and nine broad
pre-existing-worse quality findings remain unchanged.

Remaining work for the owner-selected base avatar:

1. Continue reducing runtime cost while matching the coupled reference;
   consider a smaller cage and stronger preconditioning. Preserve numerical
   error measurements rather than merely reducing iteration counts. Refine
   attachment/cage quality and volume preservation through tilted/moving poses.
2. Add calibrated torso/arm/hand, breast-to-breast and relevant environment
   contact. Garment support and repair are outside this base-avatar task.
3. Reduce the observed source-time sampling gaps. Render-correlated timestamps
   and source-paced encoding are implemented in capture 13. Verify resets,
   handoffs and stalls on actual motion.
4. Validate base-skin material/morph/shadow behavior and enable the validated
   replacement by default for the calibrated base beautiful_assistant asset.
5. Complete rendered acceptance on unchanged model motion: front/side/three-quarter
   walking, stopping, turning, bending, sit/stand and supported contact. Run the
   full required checks and resolve new maintainability issues before landing.

This remains a prototype, not a merge-readiness pass. Review and simplify the
constructor, authoring and implicit-solver boundaries as integration stabilizes.
Do not modify other agents' code or weaken ownership tests to satisfy a gate.

## Recommendation

Replace the breast-specific translation springs with a small volumetric tissue
simulation, attached across the chest and embedded into the rendered skin. Keep
ARDY's root and articulated skeleton unchanged. First fix the sampling of moving
attachments: the present frame-to-substep conversion introduces false velocity
pulses even during constant-speed movement.

This is a solver, asset, and rendering change. Adjusting spring constants or
increasing breast-bone skin weights cannot provide local compression, volume
preservation, posture-dependent gravity, or garment support.

## What the current implementation does

The relevant path is:

1. `physics/mod.rs::install` installs Rapier and configures a 120 Hz outer
   timestep with four internal substeps by default: an internal dt of 1/480 s.
2. `physics/secondary.rs::auto_detect_chains` matches `.*Breast.*`. Each matched
   bone gets a kinematic anchor under its animated parent and an unparented
   dynamic sphere. Keeping the dynamic body unparented correctly avoids
   inheriting animated transforms a second time.
3. A generic joint locks angular axes and applies identical force-based linear
   springs on all three translation axes. With the inspected assets, nominal
   additional mass is 2 kg per side, stiffness is 1260 N/m, and the configured
   damping rate becomes 60 N s/m. These imply approximately 4 Hz and damping
   ratio 0.60, ignoring the small collider mass. These are generic parameters,
   not measurements of this avatar.
4. The sphere has no collision partners (`Group::NONE`), zero gravity, disabled
   sleeping, and no `TransformInterpolation` component. Rotation-mode chains
   do receive interpolation.
5. `write_jiggle_bones` reads the simulated offset, clamps its length to 3 cm
   by default, transforms it into parent space, and writes bind translation plus
   that offset. It resets the breast bone rotation to the anchor's bind rotation.
6. Ordering is after Rapier writeback and before final transform propagation.
   Rapier also propagates transforms before syncing its backend, so the current
   code is not simply missing pre-physics transform propagation.

## Confirmed problems

### 1. Attachment motion is delivered as a pulse

`apply_skeletal_pose` runs before `PhysicsSet::SyncBackend`. The pinned Bevy
Rapier adapter submits one `set_next_kinematic_position` per changed render pose.
Rapier consumes that target during the first internal step. Subsequent steps
have no intermediate target. The cloth-specific backend in
`physics/substeps.rs::advance_context` also advances repeatedly without
resampling the attachment trajectory.

A standalone probe using the pinned Rapier library and the breast joint's
parameters reproduced this with a constant 0.5 m/s translation:

| Render fps | Anchor velocity during a frame's internal steps | Breast offset at the end of 4 s |
|---|---|---:|
| 30 | 8 m/s, then 15 steps at 0 | +6.225 mm |
| 60 | 4 m/s, then 7 steps at 0 | +3.000 mm |
| 120 | 2 m/s, then 3 steps at 0 | +1.129 mm |
| All three, attachment resampled each step | approximately 0.5 m/s throughout | -0.389 mm |

The offset is dynamic body minus anchor along travel, sampled at render-frame
end. This is a native solver reproducer, not an end-to-end Bevy recording. It
establishes the mechanism and frame-rate dependence; it does not quantify its
contribution to a particular observed animation. The local reproducer is
`/tmp/cc-breast-kinematic-probe.rs`.

More solver substeps alone do not fix an attachment that advances its entire
frame distance in the first substep. Output interpolation alone also cannot
repair the forces already generated by that pulse.

### 2. The deformation model has only three degrees of freedom per side

All breast motion is one translation weighted across the skin. It cannot
independently represent upper attachment, lower tissue, tip motion, compression,
shear, or changing shape at roughly constant volume. The translation may produce
stretching through ordinary linear blend skinning, but there is no tissue strain
or volume constraint controlling that stretching.

The actual GLBs contain `CC_Base_L_Breast` and `CC_Base_R_Breast`, under their
respective `CC_Base_*_RibsTwist` parents. Each breast bone has an approximately
2 cm local offset. Bone length is therefore not an estimate of tissue depth or
mass. No breast/chest/bust-named morph targets were found.

### 3. Render clipping conceals solver state

The joint limits form an axis-aligned box of +/-3 cm per axis; the visible
writeback uses a 3 cm sphere. A diagonal solver displacement can be valid for
the joint yet clipped for rendering. The clipped position is not reconciled
with the solver position or velocity. The visible mesh can stick to a limit
while the hidden state continues moving.

The existing discontinuity test checks this bounded output. It does not prove
that the underlying state, contact response, or acceleration is physically
reasonable.

### 4. Gravity, contact, and support are absent

Disabling gravity avoids extra sag in the authored standing shape, but also
eliminates changes caused by leaning, reclining, or lying sideways. The proper
problem to solve is the reference shape under load.

The sphere cannot collide with the torso, arms, opposite breast, clothing, or
props. Simply enabling its collision mask would collide a small sphere at a
bone location; that is not a substitute for a deforming tissue surface.

### 5. Asset settings do not reach this avatar; overrides are inconsistent

The CC5 JSON resolver only checks `<glb-stem>.json` beside the GLB. The inspected
processed base GLBs have neither that JSON nor `<glb>.physics.json`.
`avatars/beautiful_assistant.json` exists one directory above, but has empty
breast collision entries and no soft-physics or breast-tuning block anyway.

The condo outfit v13 has a sibling `.json`, but it is an outfit export report,
not CC5 physics. The loader accepts it as an empty `Cc5Physics`. Thus the
inspected paths resolve to generic breast defaults, absent a caller-provided
physics sidecar.

`chains_override` reads per-bone stiffness and damping, but the translation
branch uses `breast.stiffness` and `breast.damping` instead. Per-bone mass and
radius do apply. This contradicts the stated override precedence. Old sizing
tables in `cc5_json.rs` also disagree with current defaults and the 5 cm cap.

## Asset and clothing findings

Inspected the adjacent office SDK's processed base GLB and
`scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb`. The adjacent
main SDK copy has the same breast-node structure and absent sidecar paths.

For the body, after normalizing `WEIGHTS_0` as the renderer does, each side has
751 primitive vertex entries with breast weight above 0.001; 209 have weight
at least 0.99. The median affected weight is about 0.653. These counts include
exported vertex entries, not a welded anatomical surface. The two inspected
body assets give the same statistics.

The v13 knit top has 1,502 affected entries per side and 418 at weight >=0.99,
with essentially the same weight distribution. Its builder explicitly exports
static skinned clothing. It follows the bone displacement without simulating
fabric tension or supporting the underlying tissue.

Mixed torso/breast weights are not inherently wrong: the breast attaches across
a footprint rather than sliding as an isolated object. Do not apply the legacy
`reweight_jiggle_bones.py` boost as a realism fix. Audit weights with rendered
deformation before changing them. Keep the existing retained-weight normalization;
it addresses a separate Bevy four-influence import issue.

## Proposed physical representation

Author a closed low-resolution tetrahedral cage for each side, including a chest
attachment footprint and a smooth transition region. Start a feasibility build
around 100-200 vertices per side, then select resolution from deformation error
and measured CPU cost; this is a proposed starting budget, not a benchmark.

Use a volume-based constitutive model with separate resistance to shear and
volume change, solved with XPBD. A constraint-based stable Neo-Hookean model is
a suitable starting formulation. It provides a basis for finite deformation
beyond independent point springs. XPBD's compliance formulation makes material
parameters less entangled with timestep and iteration choices, but finite
resolution and convergence still require measurement. See the authors'
[XPBD paper](https://matthias-research.github.io/pages/publications/XPBD.pdf) and
[stable Neo-Hookean formulation](https://matthias-research.github.io/pages/publications/neohookean.pdf).

The asset should define:

- A distributed attachment patch driven by the chest's unmodified animated
  transform, with a graded compliant transition to mobile tissue.
- Rest tetrahedra, density, shear/bulk parameters, and dissipative parameters in
  explicit units. Derive nodal masses from volume and density rather than a
  universal per-side mass. Do not present visually selected values as measured
  human tissue properties.
- Optional directional support constraints and skin-envelope stiffness, fitted
  to the asset. Left and right simulate independently; shared chest motion and
  actual contacts provide coupling without an invented oscillation phase.
- A gravity-loaded reference configuration. Offline rest-shape/prestress fitting
  should reproduce the authored standing surface under gravity and the chosen
  garment support. Validate it in tilted poses. Permanently disabling gravity
  or merely resetting the deformed state each frame is not the solution.

Integrate in world space with moving chest attachments. Translational and
rotational inertia then arise from attachment motion; do not additionally add
synthetic acceleration forces. A future chest-local implementation would instead
need all relevant inertial-frame terms and equivalent invariance tests.

Use surface contact against a calibrated torso proxy and moving arm/hand shapes,
plus breast-to-breast and relevant environment contact. Solve thickness,
nonpenetration, friction, and velocity response in the simulation. Check swept
motion at the substep rate. Reject invalid/inverted geometry rather than hiding
it with a final vertex clamp. Kinematic skeletal contact remains one-way: this
does not turn ARDY into a force-aware whole-body simulation.

## Clock and Bevy integration

Use one authoritative session/revision/timestamp domain for committed body
motion, tissue, and contact sampling. Do not independently accumulate wall-clock
time while playback is paused. `ClothClock` is an existing pattern for session
and timestamp validation, but it runs at 60 Hz and is not already a general
shared physics clock.

Proposed flow:

```text
Committed adjacent ARDY samples
    -> fixed retargeting and attachment/collider trajectory sampling
    -> fixed physics substeps with targets sampled at every substep
    -> tissue and contact solve; reconstruct consistent velocities
    -> tissue output sampled at the displayed timestamp
    -> ordinary body skinning plus tissue deformation -> render
```

At every internal step, sample attachment translation and orientation from
adjacent committed poses using the permitted playback interpolation. Keep
simulation state separate from visual interpolation. Capture intermediate
trajectories from committed data rather than mutating the displayed skeleton
back and forth to drive the solver. Do not extrapolate unknown future motion
through an underrun. Explicit session resets/reloads initialize a new equilibrium;
ordinary acceleration must not trigger a hidden reset.

Use a fixed substep rate selected by convergence tests, initially investigating
120-240 Hz for the cage. Compare against finer stepping before selecting the
production rate. The current native Rapier defaults produce 480 Hz internal
steps, so moving rigid contacts need explicit interval synchronization if the
tissue rate differs. The [Small Steps paper](https://matthias-research.github.io/pages/publications/smallsteps.pdf)
supports investigating multiple smaller physical steps; it does not establish
the correct frequency or budget for this avatar.

## Rendering and garment support

Prefer a separate surface-deformation channel instead of writing the two breast
bone transforms. Precompute cage embeddings and an influence mask that fades
smoothly to zero at the torso boundary. Apply the simulated-minus-base-skinned
cage displacement to the corresponding base-skinned body surface. Define both
terms in the same space and at the same timestamp to avoid double transforms.

Implement a Bevy 0.15 render extension that preserves existing PBR materials,
facial morphs, skinning, shadows and depth passes. Update normals/tangents from
the deformation gradient or a validated surface reconstruction. Expand bounds
for the allowed deformation. The legacy breast bone writer must not run on the
same region; retaining bind transforms preserves the existing skeleton contract.

For a future clothed-avatar extension, the top needs its own correspondence and clearance treatment. Initially use an
explicit garment-support approximation with authored compliance and coverage,
and a matching garment deformation. Following the skin alone is not support.
Loose cloth and close-up compression ultimately require coupled cloth/tissue
contact with reciprocal force response. Existing bedding cloth code is a source
of scheduling/contact patterns, not an already validated breast or garment solver.

The surface channel is restricted to the reviewed tissue region. It cannot
write root, spine, arms, or model pose/history, and cannot change settings based
on task success. This keeps the proposal within the requested secondary tissue
physics scope and the repository's model-owned motion architecture.

## Implementation sequence and acceptance

| Stage | Code/assets and integration seam | Evidence required |
|---|---|---|
| 1. Timing and baseline | `physics/mod.rs`, `physics/substeps.rs`, committed playback sampling; both native backend variants | Constant-speed and rotation probes; same timestamp gives the same response at 30/60/120 render fps; raw and displayed state logged separately |
| 2. Asset contract | New versioned tissue asset and explicit renderer/package reference; validated cage and body embedding | Exact base-avatar hash, meter scale, positive tet volumes, distributed attachments, equilibrium silhouette and skin correspondence |
| 3. Pure solver | New isolated `soft_tissue` module; reuse proven numerical/contact primitives where suitable | Rigid-motion invariance, settling, shear/volume convergence, energy dissipation, compression and contact tests |
| 4. Bevy/render adapter | New tissue adapter after pose sampling, before extraction; disable breast portion of `secondary` for this asset | Unchanged ARDY skeleton/root, correct timestamp, matching PBR/shadow/depth geometry, no cracks or body penetration |
| 5. Visual acceptance | Recorded unchanged model motion on the base beautiful_assistant avatar | Front/side/three-quarter walking, stops, turns, bend, sit/stand and supported contact where the source motion supports it; normal-speed and slow-motion review |

No change is proposed to hair, skirt, tongue, or glute behavior in this work.
Do not weaken motion-ownership tests or introduce body-pose correction to make
the recordings pass. Synthetic translations/rotations belong in isolated physics
fixtures; live avatar proof uses unchanged model motion.

Record displacement and deformation, per-tet volume/strain, penetration depth,
contact impulses, kinetic/elastic energy, timestamp alignment, and CPU cost.
Use engineering acceptance targets such as cross-frame-rate landmark agreement
within 1 mm, no inverted tetrahedra, and contact residual within the configured
contact tolerance. Treat those as proposed numerical targets to validate, not
measurements of anatomical realism. Fit response and garment behavior against
appropriate motion reference; there is no universal realistic bounce amplitude.

The current tests exercise matching, quaternion diagnostics, bounded output,
settling and discontinuity recovery on a synthetic one-bone rig. They do not
test the real mesh, frame-rate equivalence, tissue volume, collision, gravity,
clothing support, or visual acceptance. Passing them cannot certify this design.

## Historical initial review evidence and limits

This review inspected current source, local pinned dependency source, GLB node
and skin buffers, source metadata and outfit construction. The native Rapier
probe reproduced the attachment timing defect. At the initial review checkpoint,
no new Bevy acceptance video had been produced and the proposed tissue model
had not been implemented. The implementation checkpoints above supersede that status.

Current validation:

- `cargo test --offline --lib cc_render::bevy_app::physics::secondary::tests -- --nocapture`:
  5 passed, 0 failed. Existing compiler warnings remain.
- `python -m unittest discover -s tests -v`: 6 passed, 1 failed.
  `test_root_acceptance_does_not_rewrite_the_candidate` scans the text between
  `resolve_root` and `permits_root`, including other intervening functions, then
  rejects a `for ` substring in that larger range. The test and controller were
  unchanged during this review. This is an existing source-gate failure, not a
  breast-physics result. Log: `/tmp/cc-breast-review-python-tests.log`.
- The native Rapier probe above distinguishes frame-target pulses from
  substep-resampled motion without changing production code.

Only this design document was added during that initial review; the prototype
code and artifacts described above were added afterward in the isolated worktree.
