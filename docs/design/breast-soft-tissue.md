# Base-avatar volumetric tissue

**Paused at the user's request. Visible bounce remains unsatisfactory.**
See the [paused checkpoint](breast-soft-tissue-paused.md) for the latest skin-volume
candidate, retained evidence, test limitations and temporary-file preservation.
The simulation description below records the preceding viscosity checkpoint;
its cage dimensions and asset selection are superseded by that checkpoint note.

Status: isolated, opt-in Bevy prototype on `feat/breast-soft-tissue`, based on
`ea65ebb68`. The replacement is **not enabled by default or production-ready**.
The current implementation and evidence are described here; the
[development history](breast-soft-tissue-history.md) preserves earlier designs,
failed experiments and superseded checkpoints.

## Scope and ownership

Replace the base avatar's breast-bone springs with a small volumetric tissue
simulation. ARDY remains the only body/root motion author. The renderer samples
unchanged committed motion and deforms the mesh; it does not add joint targets,
IK, root corrections or pose clamps. See [motion ownership](model-owned-motion.md).

The user's base-avatar request resolves to:

- `../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb`
- `../remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json`
- GLB SHA-256: `23060843839654603f26d7aae6a955cf09c86bb2b39de8bf70bfc5e70d1062a9`

No separate `beautiful_avatar` asset was found. The current lab uses this base
mesh, with no clothing or condo geometry. Other agents' worktrees and renderers
are not changed by this work.

## Current simulation

Each side has 256 world-space nodes; the left/right cages contain 828/819
tetrahedra. Distributed fixed attachments follow the sampled chest frame; the
remaining 192 nodes per side respond to inertia, gravity, elastic forces and
viscosity. Fitted unloaded reference coordinates preserve the authored loaded
shape under gravity. All solver quantities use f64 and SI units.

The current asset is
`assets/tissue/beautiful_assistant.base-viscous-prototype.json`. Its parameters
are engineering choices, **not measured anatomical calibration**:

| Parameter | Value |
| --- | ---: |
| Density | 950 kg/m³ |
| Shear modulus | 1,500 Pa |
| Bulk modulus | 500,000 Pa |
| Quartic corotated-strain coefficient | 8 |
| Shear / bulk viscosity | 15 / 5,000 Pa s |
| Integration | Variable-step BDF2; backward-Euler startup |
| Maximum nonlinear iterations | 32 |

The current small-strain linearization has Young's modulus 4.496 kPa, Poisson's
ratio 0.49850, and shear retardation time `eta / mu = 0.01 s`. Its shear storage
modulus is constant, while its loss modulus is `2*pi*f*eta`; the loss/storage
ratio is 0.063 at 1 Hz and 0.314 at 5 Hz. These are consequences of the chosen
model, not measurements of this avatar. The analytical audit and plot are in
`artifacts/soft-tissue/avatar-contact/material-response.{json,png}`.

Published values need their loading and frequency context. [Sridhar and Insana
(2007)](https://ws.engr.illinois.edu/blogs/getfile/151/30849) measured slow creep
in three volunteers, fitting two retardation times (3.2 ± 0.8 s and 42 ± 28 s)
over approximately 0.0016–0.16 Hz. Their unknown internal stress distribution
prevented reporting absolute tissue compliance. [Kumar et al.
(2018)](https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0205717)
fit a Voigt model over 50–400 Hz with observation shorter than 15 ms and explicitly
limit their parameter estimates to that regime. Neither study directly fits the
current jump. A single Kelvin–Voigt term cannot reproduce two creep time scales.
Consequently, copying an elastography viscosity into this asset would not establish
realism; material fitting must match the deformation, loading and frequency range
being reviewed. No material parameter was changed by this literature audit.

Elasticity uses corotated strain with finite volume resistance. Viscous forces
come from objective isochoric Green-strain and log-volume rates, so a rigid
translation or rotation does not create viscous strain. Legacy edge damping is disabled in this
asset. The implicit solve uses an approximate PSD tangent, sparse 3×3 block
assembly, preconditioned conjugate gradients, and energy/positive-volume line search.

The earlier viscosity model used `trace(delta E)` as its bulk coordinate, where
`E = (F^T F - I)/2`. A simple shear of amount `gamma` preserves volume exactly
(`J = det F = 1`) but has `trace(E) = gamma²/2`. Thus the large bulk coefficient
also resisted finite shear. The retained failing regression measured 0.002604 J
of bulk incremental potential at shear 0.1 in its test tetrahedron. This is a
constitutive coupling, not a violation of rigid-rotation objectivity.

The current phenomenological law separates the coordinates:

```
Ebar(F) = (J^(-2/3) F^T F - I) / 2
q(F) = ln J
incremental potential / rest volume =
    (eta_s / dt_eff) ||Ebar - Ebar_history||²
  + (eta_b / (2 dt_eff)) (q - q_history)²
```

Both histories use the same variable-step BDF2 weights as position and velocity,
with the existing backward-Euler startup/growth restart. Forces are exact
derivatives of this potential; the tangent is its Gauss–Newton approximation.
Its positive-semidefinite form is `2 eta_s/dt_eff * A^T A +
eta_b/dt_eff * b b^T`, where `A = dEbar/dx` and `b = d(ln J)/dx`.
For a nodal perturbation `u` with reference shape gradient `g`,
`dEbar = J^(-2/3) [sym(F^T u g^T) - C*(F^-T g · u)/3]`.
This also explains why uniform dilation of an already distorted state generates
no shear damping. The small-strain shear/bulk viscosities remain 15/5,000 Pa s;
elasticity, fitted reference, density, motion and camera are unchanged.

The separation follows the kinematic distinction described by
[Large strain Flory’s decomposition (2022), sections 2.2–2.3](https://www.scielo.br/j/lajss/a/jH7X5bNXqCYH5Gydh89WqHb/).
Our quadratic isochoric-strain/log-volume potential is an implementation choice,
not a reproduction of that paper's invariant-rate law or an anatomical fit.
Tests cover isochoric shear, scaled distorted states, hydrostatic bulk stress,
rigid-motion objectivity, finite-difference forces, PSD tangent and BDF2 history.
The independent axial-oscillator ODE was re-derived for this law; its maximum
state error decreases from 0.021734 to 0.006057 to 0.001595 mm at 240/480/960 Hz,
with the original convergence thresholds unchanged.

The global force and Newton-correction stopping limits are 0.1 mN and 0.1 µm.
The linear solve uses block LDLᵀ preconditioning, with symbolic ordering cached
once per cage and shared immutably across cloned states. Small sparse cages use
a deterministic minimum-degree permutation and complete fill, first attempting
the unshifted system. The left/right cages expand from 903/898 original blocks
to 2,667/2,632 factor blocks. Symbolic construction falls back to the original
incomplete pattern above 512 nodes or 4,096 filled blocks. This bounds additional
ordering/fill work without changing the physical matrix or material.

Positive-pivot checks and bounded diagonal shifts stabilize only the
preconditioner, followed by block Jacobi if factorization fails. The incomplete
path retains its established initial shift of 0.1; see the general
[shifted incomplete-factorization approach](https://libeigen.gitlab.io/eigen/docs-nightly/classEigen_1_1IncompleteCholesky.html).
Complete fill is exact in exact arithmetic; floating-point accuracy is still
checked against the original matrix. The recorded arm-fold regression exercises
both complete and incomplete patterns so the stronger factor cannot mask the
previous residual-restart bug.

PCG checks the true residual against the original matrix before returning,
restarting if the recursive residual has drifted. It requires relative residual
≤1e-9 within 256 iterations or returns `LinearSolveFailed`; failed state is not
committed. The tighter tolerance preserves the existing repeated free-fall
velocity test. The earlier 1e-7 candidate failed that test and was not retained.
Before the viscosity split, the strict-convergence CPU workload completed 19,567
linear solves, each in one CG step, with maximum true relative residual 2.00038e-11.
The corrected material completes 17,027 linear solves with maximum relative
residual 1.22990e-11; all 3,819 accepted states meet both nonlinear limits.
The unchanged replay needs three subdivisions instead of thirteen.

Nonlinear acceptance now checks both force balance and the remaining correction,
including the state after the last permitted descent. Reaching the cap no longer
silently succeeds. `NonlinearSolveFailed` leaves positions, velocities, attachment
frame and derivative history unchanged. The Bevy adapter retries a smaller
source-time interval using its existing depth-six subdivision limit; exhausted
retries fail explicitly. The default cap is 32, with early exit once both limits
are met. Material coefficients are unchanged.

The arm-fold request exposed a wrong-sign residual restart at 2.825 s, before
any contact was active. Residual replacement now uses `b - A*x` consistently
with the initial PCG residual. The captured 576-degree-of-freedom SPD matrix is
retained as an exact-bit regression fixture: the old implementation fails at
8.29e-5 relative residual; the corrected solve reaches 6.63e-10 in 83 iterations.
Decimal round-trip changes of one ULP can hide this failure, so failure diagnostics
also preserve floating-point bits. Neither the 256-iteration limit nor the 1e-9
tolerance was relaxed. Evidence is under `artifacts/soft-tissue/avatar-contact/`.

A new tissue session initializes at its first displayed source timestamp, where
both the body pose and facial contact geometry have been observed. This may be
66 ms for a 30 FPS display even when the first body packet is at 50 ms. It does
not extrapolate facial geometry backward or simulate an unobserved prefix.
After initialization, all intervening committed body knots remain available for
substep sampling. Existing sessions are never reset for ordinary acceleration.

The Bevy adapter advances at fixed source-time steps, with a cloned solve for
fractional display time. Failed inversion/contact/nonlinear steps retry recursively at
smaller intervals without committing failed history. The runtime rate default
remains 120 Hz; the current fixed-clock review uses 240 Hz explicitly. The
separate live capture uses 120 Hz and exposes its missed frame deadlines.

Independent regions solve concurrently on Bevy's existing compute task pool.
Every task operates on trial state and joins before collision checks or commit.
Results are inspected in region order, so simultaneous failures retain the
same first error as sequential execution. A missing pool or single region uses
the sequential path; no worker is created per substep. Contact refreshes remain
sequential. Cross-region contact is not currently modeled, so this concurrency
does not remove any existing mechanical coupling.

## Rendering and contact

The adapter reads the same calibrated rig transforms used for display and
checks sampled skin against displayed skin. Tissue displacement is embedded in
the original surface before ordinary GPU skinning; normals and tangents are
updated for normal PBR, depth and shadow rendering. Requested volumetric tissue
excludes the legacy breast springs, preventing both systems from deforming the
same tissue. The lab disables all legacy secondary chains for its comparison.

Body contact uses 45,940 torso/arm triangles and 1,502 embedded skin samples.
Conservative vertex-face sweeps cover linear endpoint trajectories. Normal
penalties are weighted by represented surface area, with 0.5 mm clearance and
0.1 mm residual allowance. Refreshed contacts re-solve from the original physical
state; they never project the rendered mesh or change body motion.

Swept bounds are computed once per skin vertex and shared between adjacent
triangles. Each contact query builds a local tree containing only triangles whose
bounds overlap the tissue samples' swept region (693 faces on average in the
recorded jump). Dense local tree IDs map back to sorted original face IDs.
The region and tree are rebuilt after every contact re-solve, including retries
and fractional display steps. No tree or physical state is retained between
queries. Exhaustive-query tests cover motion, rewinds and empty regions.

Known contact limits: no edge-edge, mutual breast, friction or environmental
contact; no bound on curved FK trajectories between sampled endpoints. The
current jump activates zero contacts. A moving-triangle compression/release
fixture passes, but broader active contact on the actual avatar is unfinished.
The eight-second ARDY arm-fold request (seed 733) also produces zero active
contacts. After the restart fix it completes all 478 frames and 160 source
packets in 22.14 wall seconds, with verified video timestamps. This is additional
motion/numerical evidence, not successful actual-avatar contact validation.
Two further declared attempts (arm fold, seed 734; self-hug, seed 735) each
complete 478 frames with zero active contacts. At seed 734's crossed-arm frame
at 3.0 s, an independent Trimesh audit finds a 38.98 mm minimum sampled
skin-to-arm gap and zero tissue-edge/arm-face crossings. The closest sample's
distance also matches an exhaustive triangle projection. This is static
geometry evidence at one frame, not whole-sequence CCD certification. Taking
the read-only snapshot preserves every recorded physics value. Videos, source
audits and gap measurements are retained under `artifacts/soft-tissue/avatar-contact/`.

Position-affecting facial morphs are included in sampled contact geometry when
the body clock advances. Changing those morphs at a paused body timestamp still
fails explicitly. Owner clarification about independent tissue settling remains
pending: the existing contract says to “pause the simulation/motion clock” on
underrun, while facial animation is separate. Do not silently change this policy.

## Review evidence

Open the [current review page](../../artifacts/soft-tissue/temporal-integration/index.html)
for the current fixed-clock video, separate live capture, earlier paired front/side video,
normal/quarter-speed playback, rate comparisons and plots.
The current video contains all 478 captured frames through 8,000 ms, at 60 output
FPS and 240 Hz tissue. Encoded timestamps exactly match source timestamps.
Raw frames at 2,200 and 7,633 ms were inspected for framing and surface continuity; this does
not establish anatomical or full-motion visual acceptance.
Its fixed-clock playback is not a real-time claim. The 176-frame strict-solver
live capture and earlier 229/217-frame live videos are retained and labeled.
The unchanged eight-second ARDY recording contains repeated jumps; every source
sample is retained. The earlier fixed-clock front/side captures each have
478 matching display frames, all 160 source
packets, and the final 8,000 ms timestamp. Source and encoded video timestamps
are verified. Front and side traces match exactly.

The following sensitivity table predates the finite-strain viscosity split:

| Check | Maximum measured difference |
| --- | ---: |
| 120 / 240 Hz surface displacement envelope | 2.072 mm |
| 240 / 480 Hz surface displacement envelope | 0.786 mm |
| 240 / 480 Hz tissue centroid | 0.432 mm |
| 8 / 32 nonlinear iterations at 240 Hz, centroid | 0.048 mm |

Whole-sequence peak skin displacement at 240/480 Hz is 37.85/38.17 mm; minimum
element volume ratios are 0.8553/0.8588. These sensitivity checks do not establish
anatomical accuracy or bound every vertex's timestep error. Independent rigid
oscillator and nonlinear viscous-tetrahedron tests also verify temporal response.

The corrected finite-strain viscosity passes **67 focused tissue tests**, including
failed-solve rollback and successful subdivision after nonlinear nonconvergence.
The current lab passes all **five native-stream example tests**. Three headless-clock
tests pass, covering stale render timestamps, exact offline stepping and virtual
pause. The broader 469-test run terminated with exit 143 during its bedding
simulation without a terminal result or assertion failure; it is not a full-suite
pass. Its source checkpoint predates the nonlinear change.
The Python suite has eight
passing tests and one existing root-acceptance source-slice failure. Its scan
extends beyond `resolve_root` into unrelated code; the test has not been weakened.
Ripwire's quality gate remains non-clean at eight major findings.

The final-state CPU audit found that the previous eight-pass cap missed either
the 0.1 mN force limit or 0.1 µm correction limit in 140 of 3,819 successful
returns. A 32-pass cap alone reduced this to ten. Explicit convergence plus
bounded retry meets both limits in all 3,829 successful kernel solves, with
13 subdivisions across the replay. Maximum remaining correction is 0.09364 µm.
Compared at every saved node and step, the eight-pass and strict trajectories
differ by up to 1.791 mm. The older centroid-only comparisons did not bound this
local error. The full native fixed-clock GPU replay retains all 478 timestamps
and attachment transforms; its displacement-envelope/centroid differences are
0.2972 / 0.1389 mm. Neither comparison is anatomical calibration. See
`solver-runtime/nonlinear-comparison.json` and `nonlinear_audit.py`.

An independent volume-distribution audit of the earlier CPU trajectory found
whole-region volume changes within approximately -2% to +1.1%. The worst local
left-tissue element reached a ratio of 0.742 but represented only 0.0031% of
rest volume. This distinguishes a local extreme from whole-region compression;
it does not excuse local error or validate incompressibility. Evidence is in
`solver-runtime/volume-distribution-audit.json`.

## Runtime measurements

The latest split-viscosity live run delivers 95 frames through 8,000 ms,
or 11.85 presentations/source-second, with a 207.2 ms p95 source-frame gap.
This is one shared-host measurement, not an isolated material-cost benchmark;
live performance remains inadequate. The fixed-clock review contains all 478
frames. Its peak displacement is 42.745 mm (previously 37.879 mm), minimum
local volume ratio is 0.8875 (previously 0.8556), and active contacts remain zero.
The same-time envelope/centroid changes reach 32.785/13.493 mm; CPU node
trajectories differ by up to 34.535 mm. This is a substantive change of response,
not a numerically equivalent optimization. All source and attachment frames
are unchanged. See `solver-runtime/viscosity-split-comparison.json`,
`viscosity-split-response.png` and `viscosity-split-video-validation.json`.

A further 480 Hz trace-only replay retains all 478 source timestamps. Compared
with 240 Hz, the maximum envelope/centroid differences are 1.293/0.557 mm;
peak displacement is 42.954 mm. These summary metrics do not establish a
converged reference or bound every vertex. Cross-rate anchor translations and
quaternion chords differ by up to 1.202e-7 m and 1.865e-7, respectively; their
forcing is not bit-identical. The original exact-anchor assertion failed and is
retained in `viscosity-split-rate-anchor-exact-check.log`; the audit now reports
these differences explicitly. The same-rate material comparison remains exact
in source timestamps and anchor transforms.

The following performance comparisons precede the material-law correction.

Enforcing nonlinear convergence exposed a live-throughput problem. A declared
trace-only pair using the strict solver reached 17.53 presentations/source-second
at 120 Hz and 5.80 at 240 Hz. Both preserve all 160 source packets and reach
8,000 ms, but large frame gaps remain. Increasing the tissue rate did not solve
the overload, so the runtime default remains 120 Hz. The library test was
confirmed suspended during these timing captures; its process disappeared
before resume, while both GPU child processes exited successfully. See
`solver-runtime/nonlinear-rate-comparison.json` and the suspension record.

The viscosity tangent now computes its ten diagonal/upper blocks and mirrors
the other six by transpose. A fresh isolated CPU comparison takes 14.54 s with
the archived full calculation and 11.00 s with symmetry reuse. Every saved node
position is bit-identical, with the same 19,567 linear solves and 13 subdivisions.
These are single shared-host CPU measurements, not an interactive guarantee.
The optimized full renderer preserves every traced physical value and source
timestamp across all 478 frames. A fresh fixed-clock trace-only capture takes
15.87 playback seconds versus 10.71 before, with unchanged sampling and collision
stages slower as well. These shared-host runs do not establish a renderer speedup.
The latest optimized live 120 Hz run delivers 128 frames, or 16.01 presentations
per source second, with a 215.6 ms p95 source gap. Live responsiveness remains
inadequate. See `solver-runtime/viscosity-symmetry-comparison.json`,
`viscosity-symmetry-fixed-comparison.json`, and `viscosity_symmetry_audit.py`.

The following clock-correction measurements precede strict nonlinear acceptance.
Headless realtime playback now samples the current monotonic main-loop instant
before Bevy's `TimeSystem`. Bevy 0.15.3's automatic strategy consumed an older
render-thread completion instant: the read-only probe measured a median age of
40.77 ms and a maximum of 77.67 ms. Variation in that age accounted for the
alternating source deltas beyond the actual main-loop interval. This is a clock
selection change for headless realtime renderers, not a tissue-material or motion
change. `TimePlugin` still drains the render timing channel and manages real and
virtual time; explicit offline steps and virtual pause are preserved.

The declared post-fix captures use the unchanged native jump, 30 FPS output and
120 Hz tissue. Only this worktree's verified library-test process was suspended
for the three timing runs, then resumed; the commands and suspension record are
saved alongside the results.

| Main-clock run | Frames | Presentations / source second | Source gap p95 / max |
| --- | ---: | ---: | ---: |
| Tissue on, 1 | 231 | 28.99 | 42 / 69 ms |
| Tissue off | 239 | 30.00 | 34 / 34 ms |
| Tissue on, 2 | 231 | 28.99 | 42 / 67 ms |

All three reach 8,000 ms with all 160 source packets unchanged. The selected
instant is less than 0.019 ms old at the diagnostic observation. The instrumented
pre-fix baseline has a 57.3 ms p95 source gap and 23 gaps above 50 ms; each new
enabled run has four such gaps. This removes the clock-feedback oscillation,
but does not meet every 30 FPS deadline: consumer-delivery p95 gaps remain about
66 ms. These are local renderer/readback measurements, not browser presentation
or live generation latency.

Sampled/displayed skin agreement stays within 0.00136 mm. Observed minimum volume
ratios are 0.854 / 0.831, with zero active contacts. The different live sampling
times prevent byte-comparing physical responses or bounding unsampled extrema.
The fixed-clock 60 FPS / 240 Hz regression does preserve **every traced physical
value and source timestamp exactly across all 478 frames**. Its timing is not
used because it overlapped the bedding test. Evidence, pinned Bevy source,
source/binary hashes and the reproducible audit are in
`solver-runtime/main-clock-*`, `main_clock_audit.py` and
`solver-runtime/bevy-clock-source.*`. Set `CC_RENDER_CLOCK_TRACE=1` for the
optional read-only timing diagnostic.

The following live-clock measurements precede the main-clock correction.
Live-clock validation now uses `offline_frame_step=None` and normal renderer
pacing, while preserving the original native packets and bounded 500 ms
prefetch. It is local replay of model-generated motion, not a live generation,
speech, network or browser-presentation test. Two declared tissue-enabled runs
and one control all deliver 160 source packets and reach 8,000 ms:

| Live run | Frames | Presentations / source second | Source gap p95 / max | Wall / source span |
| --- | ---: | ---: | ---: | ---: |
| Tissue on, 1 | 219 | 27.51 | 63 / 81 ms | 7.938 / 7.923 s |
| Tissue off | 239 | 30.00 | 34 / 56 ms | 7.945 / 7.934 s |
| Tissue on, 2 | 217 | 27.30 | 66 / 98 ms | 7.963 / 7.912 s |

Both enabled runs preserve GPU-correlated body/tissue timestamps and keep
sampled skin within 0.0013 mm of displayed skin. The source clock keeps pace
with wall time, but frame spacing is visibly irregular and the 30 FPS target
is not sustained. Tissue-solve time rises during the jumping intervals. These
results supersede any inference of interactive readiness from average offline
throughput alone. Different first observation and presentation times also mean
the live traces cannot be compared byte-for-byte: their observed minimum
element volume ratios are 0.807 / 0.772, with no inverted displayed elements.
These samples do not bound unsampled extrema or establish material realism.

A separate image-saving run preserves all 217 images and produces
`solver-runtime/live-front.mp4` with verified variable frame timestamps. Its
timings are not pooled into the trace-only runs. Commands, source hashes,
raw frames, timing plots and audits are retained under `solver-runtime/live-*`
and the corresponding `lab-front-live-*` directories.

Complete sparse factorization reduces the same isolated CPU replay from
17.54 to 11.02 s (37.2%). The largest difference across all 512 nodes at all
1,908 steps is 0.00000000627 mm. All 18,132 systems satisfy the unchanged
1e-9 residual threshold, and the integration/subdivision counts match. The
matrix-product count falls from 1,428,408 to 18,132. The independent manufactured
sparse-block test covers nontrivial ordering, nonsymmetric off-diagonal blocks,
fill entries and a fixed node; separate tests cover both symbolic limits.
This benchmark still omits contact, rendering and fractional display solves.
Evidence is under `solver-runtime/full-factor-*`.

The full Bevy replay confirms the improvement at unchanged material and motion:
at 30 FPS / 120 Hz, measured elasticity time falls from 8.63 to 3.96 s and
first-to-last-frame capture time from 10.83 to 8.15 s. At 60 FPS / 240 Hz,
elasticity falls from 12.52 to 6.08 s and playback capture from 16.47 to 10.19 s.
Both comparisons preserve every sampled displacement-envelope value, source
timestamp and attachment transform. The maximum centroid differences are
5.75e-10 / 1.23e-10 mm; these summary values do not bound every skin vertex.

The lab's normal runs retain `ScheduleRunnerPlugin` FPS pacing even with a fixed
offline simulation clock. An explicit 30 FPS / 120 Hz paced/uncapped pair records
the child environment, command, binary hash and exit code. Setting the existing
offline-only `CC_RENDER_FAST=1` removes the scheduler wait and reduces playback
capture from 8.13 to 6.02 s for 7.934 source seconds. All traced physical values
match exactly between those two runs. The uncapped run's observed consumer
interval p50/p95/p99/max is 24.26/37.97/47.04/59.00 ms: average throughput exceeds
source time, but this does not prove frame deadlines or an interactive streaming
pipeline. The earlier timing tables included scheduler waits. See
`solver-runtime/full-factor-comparison.json` and its reproducible audit script;
all five GPU runs and the original comparison sources are retained.

The full-renderer experiments build this package with optimization level
3 through a command-line Cargo profile override; repository build defaults are
unchanged. At 60 FPS / 240 Hz, the matching trace-only capture improves from
25.69 to 22.00 wall seconds, including startup, while preserving every traced
physics value and source timestamp. The corresponding first-to-last-frame
interval improves from 20.06 to 16.56 s for 7.95 source seconds. These are single
shared-host measurements, not a release-build or real-time guarantee.

The first 30 FPS attempt failed because its first displayed pose was at 66 ms,
while tissue tried to start at the unobserved 50 ms facial timestamp. The
first-observation initialization fix above passes the full replay. A fresh
60 FPS regression preserves all 478 traced physics values exactly. The failed
attempt remains in `lab-front-opt3-30fps-240hz-01`; no motion was repaired.

Before complete factorization, the startup-fix checkpoint measured:

| Output / tissue rate | Captured frames | Total wall time | First-to-last-frame wall time | Source span |
| --- | ---: | ---: | ---: | ---: |
| 60 FPS / 240 Hz | 478 | 22.03 s | 16.47 s | 7.950 s |
| 30 FPS / 240 Hz | 240 | 21.34 s | 14.42 s | 7.934 s |
| 30 FPS / 120 Hz | 240 | 17.52 s | 10.83 s | 7.934 s |

These three runs render/read back every output frame, skip PNG writes, and deliver
all 160 unchanged source packets through 8,000 ms. Even 30 FPS / 120 Hz takes
1.37 times the source duration after startup at that checkpoint. The 30 FPS
sessions initialize at 66 ms instead of 50 ms, so their
integration-grid phase and initial conditions differ. At the 160 common source
timestamps, 30/60 FPS at 240 Hz differs by at most 0.484 mm in the displacement
envelope and 0.291 mm in centroid offset. This does not isolate output-cadence
error. Lower output cadence can also miss peaks between captured frames.
At 30 FPS, the 120/240 Hz envelope difference is 1.946 mm. These summary metrics
do not bound every vertex. The reproducible audit is
`artifacts/soft-tissue/solver-runtime/startup_cadence_audit.py`, with results in
`startup-cadence-comparison.json` and build provenance in `startup-inputs.json`.

Connectivity indices are now precomputed once per tetrahedron. Elastic,
viscous and contact assembly reuse these indices without changing summation
order, material laws or solver settings. In an isolated CPU replay of recorded
attachment frames, assembly time fell from 12.18 to 6.10 s and total solve time
from 26.43 to 20.36 s. All 512 nodes at all 1,908 steps are byte-identical to the
baseline. This benchmark omits body contact, rendering and fractional display
solves; it is a profiling workload, not source-motion acceptance evidence.

The full GPU lab replay with this change also matches all 478 recorded tissue
responses exactly. It took **44.56 wall seconds for eight source seconds**.
Shared-host load affects stage timings, and PNG/capture overhead is included.
This is not a real-time performance result. Source archives, input hashes,
counters, full node trajectories and comparisons are under
`artifacts/soft-tissue/solver-runtime/`.

The subsequent collision optimization reduced measured broad-phase work from
8.01 to 1.07 s over the same jump. Candidate faces, all 478 traced tissue
responses, and source timestamps remain exactly equal. The latest GPU replay
took 36.97 wall seconds; it rendered/read back every frame but skipped PNG
writes. Compare against the matching trace-only baseline (42.73 s), not the
PNG capture above. This still does not establish interactive performance.
Evidence and source snapshots are under `artifacts/soft-tissue/collision-search/`.
Set `CC_TISSUE_LAB_TRACE_ONLY=1` for this diagnostic mode; its captures cannot
be encoded as video because they intentionally contain no saved images.

An isolated inexact-solve experiment reduced CPU cost further but differed from
the capped-CG reference by as much as 1.579 mm at a node. It is **not adopted**.
Its forcing-term idea is informed by
[Eisenstat and Walker](https://users.wpi.edu/~walker/Papers/forcing_terms,SISC_17,1996,16-32.pdf);
their convergence theory does not certify this approximate-Hessian solver.

The earlier residual-checked incomplete-factor solver takes 22.13 s on the CPU replay,
versus 20.36 s for the earlier unconverged 128-iteration solve. Its stronger
accuracy therefore costs time relative to that old cap. A separately converged
block-Jacobi reference at 1e-7 takes 27.46 s; all recorded node positions differ
from that reference by at most 0.000000653 mm. The current tolerance is 100×
tighter, so these timings are not a same-tolerance algorithm benchmark.
See `solver-runtime/ic0-integrated-comparison.json` for full-node differences
and residual measurements. Real-time performance remains unfinished.

The integrated GPU captures complete all 478 frames through 8,000 ms, with
unchanged source timestamps and attachment frames. Front/side traces match
exactly; full captures take 38.63 / 39.27 s including PNG saving. Compared with
the earlier capped solver, the maximum envelope/centroid differences are
0.00000373 / 0.00000216 mm. These summaries do not bound every vertex.
At that checkpoint the review page defaulted to `preconditioned-front-side.mp4`; older rate
comparisons are retained and labeled. Both individual videos and the paired
video have verified encoded timestamps. Raw front 850 ms and side 7,216 ms
frames were inspected for framing and surface continuity; broader visual
and active-contact acceptance remains pending.

Concurrent-region execution preserves all traced physical values and source
timestamps in a matching trace-only GPU replay. Elasticity time falls from
27.33 to 14.52 s (46.9%); total capture time falls from 38.46 to 25.66 s.
Sampling/collision stages remain approximately 2.4 / 1.6 s. These are measured
shared-host improvements, not real-time certification. The new tests also
compare every node position and velocity against sequential execution and check
deterministic failure selection and whole-interval rollback. Evidence is in
`solver-runtime/parallel-gpu-comparison.json`; the viewable videos remain the
preceding sequential capture, whose recorded physical response is identical.

## Reproduction

From this worktree, build and validate:

```sh
cargo test --offline --lib tissue -- --nocapture
python -m unittest discover -s tests -v
cargo build --offline --example tissue_lab
```

The measured optimized build uses the following local override:

```sh
CARGO_INCREMENTAL=0 cargo build --offline \
  --config 'profile.dev.package.cc-render-plugin.opt-level=3' --example tissue_lab
```

Run a fresh GPU lab capture with an output directory that does not already exist:

```sh
CC_AVATAR_TISSUE_HZ=240 RUST_LOG=info target/debug/examples/tissue_lab \
  ../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb \
  ../remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json \
  artifacts/soft-tissue/lab-jump-01/packets.ndjson \
  assets/tissue/beautiful_assistant.base-viscous-prototype.json \
  front artifacts/soft-tissue/new-capture
```

Use `side` for the other fixed camera. `CC_TISSUE_LAB_FPS=30` selects the normal
output cadence; the lab defaults to 60 and accepts only 30 or 60. This changes
display sampling, not source packet timestamps. `CC_TISSUE_LAB_CLOCK=live`
uses Bevy's ordinary realtime clock; the default `fixed` mode retains strict
offline frame-completeness checks. Live mode records variable source gaps and
rejects `CC_RENDER_FAST=1`. Neither mode changes native packet timestamps.
`CC_AVATAR_TISSUE_ITERATIONS`
controls the iteration budget. The numerical profiling script accepts a new output label and
optimization level, for example:

```sh
python artifacts/soft-tissue/solver-runtime/profile.py fresh-profile 1 residual
```

Before default replacement: establish interactive runtime performance, validate
broader actual-avatar contact and facial/pause behavior, retain source-motion
fidelity, and complete the final code/visual audit. The existing jump harness
provides a reproducible input; its successful completion alone is insufficient.

## Optimized kernel and contact-attempt checkpoint

The post-restart-fix isolated kernel at compiler optimization level 3 takes
17.57 s for the same workload that took 22.13 s at level 1; all node trajectories
are byte-identical. This does not measure the full renderer or establish a
real-time rate. The node defaults to 30 output FPS; the diagnostic lab defaults
to 60 FPS and also supports 30 FPS. Total wall time includes initialization/readback.

Two copied-kernel experiments add selected preconditioner fill blocks. They
reduce conjugate-gradient work from 1,428,408 to 962,172 / 927,951 iterations,
but total solve time only falls to 17.23 / 17.24 s. All 18,132 linear solves
meet 1e-9 in each variant. That approximately 2% difference from single
shared-host runs does not justify adopting a more expensive preconditioner.
Production code is unchanged. See `artifacts/soft-tissue/solver-runtime/filled-comparison.json`.
