# Boundary-pressure visual baseline, 2026-10-18

The user reviewed the [latest comparison video](../../artifacts/soft-tissue/volume-audit/tracked-off-on-boundary.mp4)
and replied, "Looks much much better." Preserve this exact candidate as the
visual baseline. This records positive feedback on visible deformation, not
approval of real-time performance, anatomical calibration or all contact cases.
The goal remains active; the implementation is isolated and opt-in on
`feat/breast-soft-tissue`. The [review page](../../artifacts/soft-tissue/volume-audit/index.html)
contains the video and measured limits.

## What changed

The previous tetrahedral element-pressure formulation resisted nearly all
volume-preserving modes on this mesh. Nodal pressure and isochoric corotated
shear restored broad surface movement, but some tiny chest-boundary elements
expanded severely. The current `average_nodal_boundary` mode uses element
pressure on tetrahedra with at least two fixed vertices (a fixed chest edge),
and average nodal pressure on the remaining volume. The two partitions do not
double-count bulk energy or viscosity. This boundary treatment is an engineering
experiment, not a claim that the referenced ANP paper prescribes this hybrid.

`pressure.rs` supplies nodal energy/derivatives and patch coupling;
`isochoric.rs` separates shear from dilation; `elasticity.rs` shares element
assembly. Fixed attachment nodes still follow unchanged ARDY chest motion.
No body/root/joint correction was introduced. Optional `CC_AVATAR_TISSUE_TRACE_NODES=1`
records actual node positions for volume audits without changing the solve.
Existing assets default to element pressure.

The selected asset is
`assets/tissue/beautiful_assistant.base-boundary-nodal-volume-prototype.json`.
It uses 241/233 nodes, 657/605 tetrahedra and 130/128 fixed nodes on the left/right.
Shear modulus is 500 Pa, bulk modulus 500,000 Pa, shear/bulk viscosity 5/5,000 Pa s,
density 950 kg/m3 and strain-hardening coefficient 8. These are sensitivity-test
parameters, not measured anatomy. The fitted volumes include a chest collar.
An attempted TetGen boundary refinement worsened mesh quality and was rejected;
its inputs and results are retained in `volume-audit/`.

## Controlled visual evidence

Physics is off on the left and on on the right. Both views use the same base
avatar, unchanged 160-packet ARDY jump, camera, source timestamps and tracked
crop. Spatial zoom is 2x; displacement multiplier is 1. All 478 output timestamps
were verified. The capture uses 60 output frames/s and 240 simulation steps/s.
It is offline capture, not evidence of real-time playback.

| Measurement | Current boundary candidate |
| --- | ---: |
| Peak skin displacement | 36.15 mm |
| Area-weighted skin displacement at 7.633 s | 10.93 mm |
| Skin patch area moving over 10 mm at that frame | 44.85% |
| Element volume ratio over the clip | 0.310–2.732 |
| Maximum total volume change | less than 0.28% |
| Maximum reference-volume fraction with over 50% local change, left/right | 0.147% / 0.261% |

The preceding softer nodal candidate reached an element volume ratio of 8.988.
The boundary treatment reduces that extreme while retaining visible movement.
Some local distortion remains excessive. No sampled skin face rotated more than
90 degrees at the comparison frame; this is not a self-intersection guarantee.
The jump has zero active contacts, so it does not validate compression against an arm.

## Verification and performance

- 72 focused Rust tissue tests passed, including energy/gradient consistency,
  BDF2 viscosity, fixed-node elimination and boundary/nodal volume partition.
  Logs: `volume-audit/boundary-tests-02.log` and `optimized-tests.log`.
- The last Python suite had 15 passes and one existing source-slice failure in
  `test_root_acceptance_does_not_rewrite_the_candidate`. Its broad text slice
  matches an unrelated `for ` beyond the function. Protected tests were not changed.
  The checkpoint run is recorded separately in `checkpoint-python-tests.log`.
- The optimized crate reproduced all 478 frames' nodes exactly at 240 Hz.
  Reducing simulation to 120 Hz changed nodes by up to 3.52 mm (0.224 mm RMS).
  This is timestep sensitivity evidence, not a convergence claim.
- Live 120 Hz simulation produced only 37 readbacks over 7.934 source seconds,
  taking 14.949 wall seconds after the first frame. This is unacceptable for
  interactive playback. The implicit elasticity solve dominates measured cost.
  Crate `opt-level=3` did not demonstrate a speedup in these shared-host runs.
- Structural review still reports four gating findings: constructor/assembly
  complexity, constructor length and duplicated small matrix helpers. This is
  an unfinished checkpoint, not a merge-readiness claim. No findings were suppressed.

Next work: profile and reduce solver cost without dropping physical time or
retiming ARDY; validate contact/compression under the new pressure mode; reduce
remaining local volume distortion while preserving this visual baseline.

## Reproduction and preservation

`volume-audit/launch-boundary-01.json` records exact inputs, environment, source
hashes, binary hash and successful exit. `rate-launches.json` records the later
parity/timing runs. Base-avatar identity is documented in
[the original design](breast-soft-tissue.md#scope-and-ownership); the external GLB
is not duplicated. Source motion remains tracked in `lab-jump-01/packets.ndjson`.

All new scripts, source archives, fitted assets, reports and logs are under the
worktree, not `/tmp`. `volume-audit/checkpoint-manifest.json` inventories the
selected evidence. `checkpoint-traces.tar.gz` preserves ignored numeric traces
and source timestamps from the new captures; the archive is verified against
the originals. The four compact review videos are included in Git. Raw PNGs
remain locally in their durable `artifacts/soft-tissue/lab-*` directories and
are excluded from Git. The earlier `/tmp` migration remains documented in
`preserved-tmp/manifest.json` at checkpoint `33bcb3d`.
