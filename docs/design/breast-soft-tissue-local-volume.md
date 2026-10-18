# Local volume control without losing broad deformation

The candidate in
`assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json`
reduces the reviewed jump's sampled local volume range from **0.310–2.732** to
**0.562–1.533**, while retaining broad surface movement. The
[comparison video](../../artifacts/soft-tissue/quality-pressure/reviewed-vs-local-volume.mp4)
shows the user-reviewed baseline on the left and the candidate on the right.
The candidate remains opt-in. It has not established smooth live playback,
anatomical calibration or actual-avatar compression/contact acceptance.

## Discretization

The previous uniform stabilization study left thin right-side cells severely
compressed before it suppressed too much overall movement. Selecting only thin
cells improved those extremes while retaining 96% of peak and 98% of time-RMS
movement, but a better-shaped left cell still compressed below the declared
0.5 volume-ratio screen. The combined treatment addresses both observations.

The existing boundary-nodal mode now accepts two zero-default settings:

- `element_pressure_quality_cutoff = 0.15`: select element pressure for cells
  whose mean-ratio quality is below the threshold, in addition to the existing
  fixed-edge cells. This adds eight left and seven right cells. Quality is
  calculated from the unchanged authored initial coordinates, so reference-shape
  fitting, motion and deformation do not change the partition.
- `pressure_stabilization_pa = 8000`: in the remaining nodal partition, use an
  8,000 Pa element share and a 492,000 Pa nodal share. Relative to pure averaging,
  the extra energy is half the share times the volume-weighted variance of
  element Jacobians within each nodal patch. It vanishes for uniform volume
  change and is nonnegative. Both assembly shares remain nonnegative.

Density, shear/bulk moduli, hardening and shear/bulk viscosity coefficients are
unchanged. The geometry selection also uses the existing local bulk-viscosity
path for selected cells; the variance penalty itself is elastic. The initial
shape, fixed attachment positions, mesh topology and surface embeddings are
unchanged. Each candidate's unloaded reference was refitted with the existing
0.25 mm equilibrium criterion. This is an engineering discretization treatment,
not a measured anatomical constitutive law.

`pressure.rs` owns the immutable partition and quality calculation;
`elasticity.rs` and `solver.rs` share the element bulk coefficient;
`implicit.rs` uses the complementary nodal bulk coefficient. The adapter still
reads ARDY motion and updates skin displacement only. Both settings default to
zero, preserving existing assets. Invalid settings and unsupported modes fail
validation. Direct Rust `Material` literals must supply the two new fields.

## Bounded numerical studies

The declared screens required local volume ratios within [0.5, 2.0], total-volume
change below 1%, and at least 80% of baseline peak and time-RMS embedded-surface
movement. These are engineering acceptance screens, not a claim of local tissue
incompressibility at a 50% compression ratio.

| Candidate | Left local volume range | Right local volume range | Peak / RMS retained |
| --- | --- | --- | ---: |
| Original CPU control | 0.421–2.931 | 0.277–2.747 | 100% / 100% |
| Quality cutoff 0.10 | 0.420–2.089 | 0.459–2.723 | 96.1% / 98.3% |
| Quality cutoff 0.15 | 0.452–1.989 | 0.539–1.680 | 96.1% / 97.9% |
| Quality cutoff 0.20 | 0.452–1.989 | 0.588–1.673 | 96.1% / 97.8% |
| Cutoff 0.15 + 2,000 Pa share | 0.496–1.780 | 0.600–1.564 | 94.3% / 94.8% |
| Cutoff 0.15 + 8,000 Pa share | 0.562–1.546 | 0.677–1.473 | 90.7% / 88.3% |

Only the final candidate passes all declared screens. Its integrated production
solver reproduces every 240 Hz prototype node-position byte. The 120, 240 and
480 Hz replays all retain the volume bounds, with total-volume change below
0.47%. Against the finer 480 Hz result, maximum/RMS node differences are
4.382/0.293 mm at 120 Hz and 1.375/0.078 mm at 240 Hz. The reference is a finer
discrete solution, not ground truth. These replays interpolate recorded chest
frames and omit contact and fractional display solves; rendered evidence is
reported separately below.

## Actual Bevy capture

The base avatar and all 160 source packets are unchanged. All 478 output
timestamps match the reviewed capture exactly. Every sampled chest translation
and quaternion, and the undeformed surface at the comparison time, is identical.
Both video panels use the same tracked crop, 2x spatial zoom and no deformation
amplification. Every encoded frame timestamp was verified.

| Rendered measurement | Reviewed baseline | Candidate |
| --- | ---: | ---: |
| Local volume ratio range | 0.310–2.732 | 0.562–1.533 |
| Peak skin displacement | 36.15 mm | 32.56 mm |
| Area-weighted displacement at 7.633 s | 10.93 mm | 9.42 mm |
| Patch area with triangle-mean displacement over 10 mm | 44.85% | 40.77% |
| Minimum face normal alignment at that time | 0.582 | 0.501 |
| Faces rotating over 90 degrees at that time | 0 | 0 |

Candidate total-volume changes stay below 0.26%. Normal alignment is somewhat
worse at the comparison frame despite better volume behavior. Positive alignment
does not prove absence of self-intersection. The jump contains zero active
contacts, so this capture does not establish arm/chest compression behavior.

The node-traced live run delivered 181 readbacks with a 97 ms p95 gap. A separate
run matching the earlier tracing-disabled setup delivered 158 readbacks over
7.934 source seconds, taking 8.162 seconds after its first frame, with a 132 ms
p95 gap and 228 ms end drift. These are single shared-host runs, not an isolated
speed comparison or browser presentation measurements. The inconsistent ordering
is a reason to avoid attributing speed differences to tracing or the material.
The candidate is not smooth 30 FPS playback yet.

## Validation and evidence

The final integrated suite passed 84 focused tests, including the independent
energy/force checks, fixed-node elimination, BDF2 convergence, rollback,
quality invariance, authored-geometry partitioning and affine bulk conservation.
This includes compression/release with the 8,000 Pa share.
The Python suite remains at 15 passes and the existing ownership source-slice
failure; protected assertions were not weakened.

The structural review reports recent solver churn, duplicate preserved study
logging helpers, similar validation tests, and large generated JSON arrays.
These findings are retained, not suppressed or presented as a clean merge gate.
Its name-based graph misses the Rust test registrations; the actual test log is
the evidence for executed tests.

`artifacts/soft-tissue/quality-pressure/` contains the plans, failed candidates,
fit/replay logs, source archives, exact launch inputs, numerical reports, video
and timestamp validation. Production replay sources and results are in
`production-replay-01/`; rendered traces and geometry are in `bevy-01/`, with
tracing-disabled playback in `bevy-timing-01/`. Large position binaries and raw
frames remain durably in the worktree outside Git. Necessary work is not in
`/tmp`. The reviewed original asset and video remain available for comparison.
