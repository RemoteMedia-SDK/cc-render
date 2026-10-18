# Runtime optimization after the visual baseline, 2026-10-18

The latest [BDF2 accuracy follow-up](breast-soft-tissue-accuracy.md) improves
120 Hz throughput with bounded measured trajectory error and unchanged tests.
It supersedes the fixed nonlinear stopping limits described below.

The reviewed visual baseline remains commit `12d54a8` and its
[boundary-pressure video](../../artifacts/soft-tissue/volume-audit/tracked-off-on-boundary.mp4).
This change improves solver throughput without changing the material, timestep,
source motion or deformation. The goal remains active: live performance and
local distortion still need work, and rendered avatar contact is not validated.

## Measured change

The CPU profile replays recorded chest attachment frames on the selected asset.
It is a diagnostic workload, not source-motion or rendered acceptance evidence.
At 240 Hz, the baseline took 16.19 seconds for 7.95 seconds of input. Matrix
factorization accounted for 5.14 seconds; elastic/viscous assembly for 5.74
seconds; line search for 3.20 seconds. Both regions are serial in this CPU harness.

`System::direction_cached` now reuses the first SPD factor within one physical
timestep as a preconditioner. Every nonlinear iteration still assembles the
current matrix and gradient, solves that current system, and checks its true
residual. The cache does not cross timesteps or retries. If the old factor cannot
converge, the solver rebuilds from the current matrix and retries before reporting
failure. Material laws, convergence tolerances and motion ownership are unchanged.

The isolated experiment took 13.95 seconds; the production implementation with
the fresh-factor fallback took 14.92 seconds. Both produced a maximum node
difference of 1.28e-14 m against the baseline. All three runs took the same 85
subdivisions and 18,520 nonlinear linearizations. Factor time fell to 1.09–1.29
seconds, while CG iterations rose from 18,520 to 117,531. These are single runs
on a shared host; the measured CPU reduction was 8–14%, not a guaranteed speedup.
See `artifacts/soft-tissue/boundary-runtime/cpu-comparison.json`.

## Bevy validation

The rebuilt lab completed both captures with the unchanged base avatar, fitted
boundary asset and 160-packet ARDY jump. It saved numeric traces, not new images;
the baseline video remains the review artifact.

At fixed 60 output frames/s and 240 physics steps/s, all 478 frames have:

- Identical attachment positions and rotations.
- Identical surface-displacement envelopes, including the 36.15 mm peak.
- Maximum volume-node difference 9.05e-15 m and RMS difference 8.84e-16 m.
- Unchanged minimum element volume ratio to roundoff (0.3098244).

Live 30 FPS output with 120 Hz simulation improved from 37 to 72 readbacks,
taking 10.16 wall seconds for 7.934 source seconds after the first frame
(previously 14.95 seconds). There are still long source gaps and 2.22 seconds
of accumulated clock drift. This fails the interactive target. Readback delivery
is not browser presentation, and shared-host variation prevents attributing the
entire timing change to factor reuse. See `bevy-comparison.json` and
`bevy-launches.json` in the same artifact folder.

## Contact check and tests

73 tissue tests pass, including the Bevy ownership/parallelism tests and a new
compression/recovery fixture using the selected pressure mode and material.
The new fixture exercises both element-pressure and nodal-pressure partitions,
BDF2 and continuum viscosity. It checks positive contact force, penetration under
0.1 mm, recovery under 0.5 mm, positive volume, and unchanged fixed attachments.

The first run failed the penetration limit: its 250,000 N/m penalty plane allowed
0.627 mm penetration under 158.57 N total contact force. That failure remains in
`tests-01.log`. The new nearly incompressible, coarse clamped cube uses a
2,500,000 N/m plane; the old fixtures retain their original stiffness and all
assertions. The passing run is `tests-02.log`. This numerical penalty choice is
explicit; it does not establish anatomical force calibration or change the
production contact settings. The jump still has no active contact.

The Python suite remains 15 passes and the existing ownership source-slice
failure; protected tests were not changed. Structural delta reports no worsening
of existing symbols; it reports the moved PCG loop's complexity/length under its
new method name. Earlier structural debt remains documented at the visual
checkpoint. Source whitespace checks pass.

## Reproduction and remaining work

`boundary-runtime/profile.py` preserves instrumented kernel sources and input
hashes for each CPU run. `run_bevy.py` records source/binary/input hashes and exact
launch settings; `audit_bevy.py` compares every captured volume node, verifies
unchanged input packets and checks source-clock delivery. `evidence-manifest.json`
inventories local evidence; `bevy-traces.tar.gz` preserves the new render traces.
All necessary work is under this worktree, not `/tmp`.

The following change targets elastic/viscous assembly. Continue checking against
the reviewed video before accepting any change. Local element distortion, active
avatar compression/contact and material calibration remain separate unfinished work.

## Fixed-node assembly follow-up

After `c8b427b`, matrix assembly omits tangent blocks involving fixed nodes and
skips fully fixed tetrahedra. About 44% of the upper-triangular element blocks in
the selected mesh are later discarded by constraint elimination; computing them
does not contribute to the free-node system. Forces at active nodes, energy,
volume checks, compliant attachments, source motion and material laws are unchanged.
The active mask is computed from the same inverse masses used by elimination.
Shared `System::add_element` and `eliminate_fixed` methods consolidate the assembly
logic. Existing derivative tests request the complete tangent and retain all
their assertions.

The CPU replay before this change took 15.77 seconds. The first version took
13.57 seconds and the consolidated version 12.63 seconds. All 1,908 steps across
both regions produce **byte-identical position files**, with unchanged subdivision
and iteration counts. Elastic/viscous assembly time fell from 6.21 to 4.33 seconds.
These are single shared-host runs; the measured CPU reduction is not a guaranteed
end-to-end speedup. See `boundary-runtime/assembly-cpu-comparison.json`.

73 tissue tests pass again. The structural delta is clean with no acknowledgements;
the changed internal viscosity method has no incompatible indexed callers.
Python remains at 15 passes and the existing ownership source-slice failure.
The initial structural findings and intermediate performance result are retained.

Bevy completed all 478 fixed-clock frames. Compared with the reviewed video,
attachments and displacement envelopes are identical; node differences remain
below 9.05e-15 m. The new trace is also byte-identical in node coordinates to the
preceding cached-factor capture. Live playback produced 91 readbacks in 9.88 wall
seconds for 7.934 source seconds, with 1.95 seconds of accumulated drift. This
still fails the interactive target. Fixed-clock capture took 19.36 seconds,
versus 18.06 previously; host variation prevents claiming a reliable renderer
speedup from these runs alone. No active contacts occurred.

Launches, source archives, trace archive, comparison report and input hashes are
in `boundary-runtime/bevy-free-assembly/`. CPU source snapshots and positions are
in the adjacent `assembly-before-01`, `assembly-after-01` and `assembly-after-02`
folders; position binaries remain locally outside Git. The reviewed video stays
unchanged. The next candidate is repeated body/skin sampling at shared physical
step boundaries, which currently repeats read-only work within the same frame.

## Pose-cache follow-up

After `2a496fc`, each presentation creates a fresh eight-entry `ContactPoseCache`.
The key is the exact f64 timestamp bits; there is no rounding, resampling,
smoothing or motion modification. The cache sees one frozen motion/morph window
and is discarded before the next presentation, including paused timestamps and
session changes. Immutable `Arc<ContactPose>` values share the sampled skin and
attachments without copying large vertex arrays. Failed samples are not cached.

The fixed-clock run issued 5,158 sample requests: 3,415 were computed and 1,743
reused (33.79%). Sampling time fell from 4.42 to 2.47 seconds. Total fixed-clock
capture time fell from 19.36 to 13.46 seconds, but other stage timings also varied;
do not attribute that entire change to caching on this shared host. All 478
frames' node coordinates are identical to the preceding assembly-optimized run.
Attachments and surface-displacement envelopes remain identical to the reviewed
video, with only the previously measured solver roundoff relative to that video.

The live 120 Hz run reused 1,037 of 2,685 requests (38.62%) and produced 111
readbacks in 8.97 wall seconds for 7.934 source seconds. End drift is still 1.03
seconds; p95 delivery gaps are 315 ms. This remains below the interactive target.
The jump again contains no active contacts. The accepted video remains unchanged;
these are trace captures, not replacement image recordings.

75 tissue tests pass. New checks cover distinct adjacent floating-point times,
bounded eviction, retained geometry, new-window invalidation, failed/nonfinite
samples and cached-versus-uncached subdivision/state equality. Existing assertions
are retained; direct fixture callbacks now return shared immutable poses. Python
still has 15 passes and the existing ownership source-slice failure.

The structural report flags recent churn in three adapted test functions and a
short accessor-shape match between unrelated collision and cache counters. It
also notes four extra lines in the existing long `simulate` method. These are
recorded review findings, not suppressed or resolved by combining unrelated APIs.
The Rust compiler and tests validate the callback return-type change; the
name-based contract checker alone does not establish type compatibility.

Sources, launch details, cache counts, comparison report and archived traces are
in `boundary-runtime/bevy-pose-cache/`. The next performance investigation should
measure trial-state copying and the slowest physics steps; elapsed live time is
still unacceptable despite the preserved visual result.

## Accepted-energy reuse

After `ae36df7`, a clone microbenchmark measured 10,000 copies of both regions in
0.338–0.345 seconds. Pressure-patch copying accounts for much of that cost, but
the full copy is only about 34 microseconds: it does not explain the live stalls.
No production cloning or topology representation was changed.

The line search now returns both the accepted positions and their already
evaluated objective energy. Both the physical solve and offline equilibrium
solve retain that value only while their objective is unchanged. New candidates
still undergo the same volume validation and Armijo check. Material parameters,
forces, tangents, integration, convergence limits and source motion are unchanged.

On the recorded 240 Hz CPU workload, objective evaluations fell from 29,238 to
18,520. Solve time was 12.29 seconds before and 11.35 / 11.55 seconds after (6–8%
lower in these shared-host runs). All node-position bytes at all 1,908 steps are
identical, with the same 85 subdivisions and 117,531 CG iterations. The updated
profiler's descent timer excludes the baseline energy evaluation in both runs;
compare wall time and objective counts, not the old descent timer.

Bevy preserved all 478 fixed-clock frames: node coordinates match the preceding
pose-cache capture exactly, while the difference from the reviewed video remains
below 9.05e-15 m. The surface envelope and attachments are unchanged. Fixed-clock
playback took 12.73 seconds versus 13.46 previously; elastic solving fell from
8.13 to 7.39 seconds. Live playback took 8.84 seconds for 7.934 source seconds,
with 110 readbacks and a 310 ms p95 delivery gap. This is still not smooth
real-time playback. No active contacts occurred, and no replacement video was made.

76 tissue tests pass, including a new check of accepted backtrack energy and
failure without input mutation. The Python suite remains 15 passes and the
existing ownership source-slice failure. Structural reports flag growth of the
test module, recent solver edits and matches against preserved historical source
copies; these are retained as reviewed findings, not suppressed or presented as
a clean structural gate. The compiler and tests cover the changed tuple types;
the name-based contract checker alone does not verify them.

`boundary-runtime/energy-cpu-comparison.json`, `clone-before-01/` and
`bevy-energy-cache/` preserve the measurements, input hashes, sources and traces.
The remaining live delays are dominated by elastic solving during catch-up;
local element distortion and rendered active-contact validation also remain open.

## Predictor investigation and dev compiler profile

After `e193d91`, the instrumented 120 Hz replay attributed all 213 subdivisions
to invalid predicted tetrahedra before the nonlinear solve. No nonlinear-limit,
descent or final-position failures occurred. Reducing the nonlinear iteration
cap would not address those retries. The profiler now records those causes in
copied, archived kernels; it does not modify production source.

Two starting-guess experiments were rejected:

- Rigidly transport the preceding valid tissue state when new fixed attachments
  invalidate an otherwise valid velocity prediction. This cut subdivisions from
  213 to 20 and measured solve time from 7.47 to 6.78 seconds. However, maximum
  same-time error against a 480 Hz reference increased from 5.208 to 5.840 mm,
  and RMS error from 0.313 to 0.365 mm. It failed the declared accuracy screen.
- Select the lower-energy valid starting guess while retaining the original
  inversion guard. This reduced CG iterations from 109,948 to 89,283, but added
  objective evaluations. Measured solve time was 7.40 seconds, with unchanged
  213 subdivisions: no convincing net improvement. Maximum node change was
  7.05 um. The extra unconstrained validation in this instrumented version also
  prevents treating the small timing difference as a controlled speedup.

The 480 Hz result is a finer discrete reference, not an exact solution. These
CPU replays interpolate recorded attachments and exclude rendering, contact and
fractional display solves. Plans, counters, source snapshots and comparisons
are in `boundary-runtime/predictor*`, `retry-causes120-01/` and
`energy-start-plan.json`. Both experiments remain confined to profiler flags.

The retained change sets `codegen-units = 1` only in the
`[profile.dev.package.cc-render-plugin]` override, alongside its existing
`opt-level = 3`. Cargo supports package profile overrides; fewer codegen units
can trade compilation parallelism for optimization opportunities. See the
[Cargo profile documentation](https://doc.rust-lang.org/cargo/reference/profiles.html#codegen-units).
No material, force, integrator, acceptance or body-motion code changed.

| Measurement | Prior BDF2-policy build | One unit, run 1 | One unit, repeat |
| --- | ---: | ---: | ---: |
| Fixed 240 Hz capture, seconds after first frame | 12.856 | 10.822 | 10.672 |
| Sampling / elasticity, seconds | 2.433 / 7.445 | 1.344 / 6.819 | 1.306 / 6.760 |
| Live 120 Hz readbacks | 141 | 170 | 174 |
| Live wall seconds for 7.934 source seconds | 8.259 | 8.062 | 8.048 |
| Live p95 delivery gap, ms | 183.45 | 114.42 | 111.73 |
| Live end clock drift, ms | 325.33 | 127.56 | 113.68 |

All 478 fixed-clock frames have identical decoded mechanical values across
these three captures, including every node, attachment, surface envelope and
region metric. Their difference from the reviewed video remains below 0.752 um,
passing the original 1 um gate. The video itself remains unchanged. Live sample
times differ and are not used to claim mechanical parity.

The first build used a command-line profile override and took 109.5 seconds.
The verbose log confirms `-C codegen-units=1` on this package. After adding the
same setting to Cargo.toml, the normal build reused the identical binary. The
repeat capture archives that manifest. This applies when this manifest controls
the dev profile; it does not establish production release performance. Two runs
on a shared host support retaining the optimization, but are not an isolated
benchmark. Playback still has visible timing gaps and zero active contacts.

All 76 tissue tests pass. Python remains at 15 passes and the existing ownership
source-slice failure, with protected assertions unchanged. The structural delta
reports no regressions; its call graph does not establish compiler-profile
safety, which is checked through the build, tests and full-frame comparison.
`codegen-one-comparison.json`, the build/test logs, and `bevy-codegen-one*/`
preserve the evidence. The concise design entrypoint now records current
settings; its earlier contents remain in `breast-soft-tissue-viscosity-checkpoint.md`.
