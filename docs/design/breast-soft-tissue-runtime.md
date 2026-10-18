# Runtime optimization after the visual baseline, 2026-10-18

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
