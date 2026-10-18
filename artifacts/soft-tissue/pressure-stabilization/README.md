# Rejected uniform pressure-share experiment

Baseline: `6f3c815`. Production source and the reviewed asset are unchanged.
The experimental patch is preserved in `experimental-source.patch`; the exact
compiled kernels, including tests and fixture data, are in the study archives.

The reviewed mesh has extreme local volume changes despite small total-volume
change. Several worst cells are thin: right elements 282 and 435 have fitted
mean-ratio quality 0.086 and 0.127. This experiment asks whether a small local
volume penalty can control those extremes without suppressing broad movement.

## Formulation and checks

For the existing nodal-pressure partition, assign an element bulk share `s`
and a nodal bulk share `K-s`. The fixed-edge partition retains its full `K`.
Relative to the existing nodal energy, this adds

`s/2 * sum_nodes sum_incident_tets (V_t/4) * (J_t - J_node)^2`.

The added energy is nonnegative and vanishes for uniform volume change. The
existing force and positive-semidefinite tangent assembly can use the two
nonnegative shares directly. Viscosity is unchanged. This is an engineering
regularization experiment, not an anatomically measured material parameter or
a formula prescribed by the [ANP paper](https://pmc.ncbi.nlm.nih.gov/articles/PMC4477870/).

All 79 focused tissue tests passed with the experimental code. Three new tests
check the independent variance identity, rigid-motion invariance, uniform bulk
energy, the full-local endpoint, numerical force derivatives with viscosity,
invalid shares and backward-compatible zero defaults. Existing assertions were
retained. Python remained at 15 passes and the existing ownership source-slice
failure. The structural report flags recent source churn, three Rust tests as
unreferenced, and two one-line fixture-size increases; nothing was suppressed.
After removing the experiment from production, the retained report flags only
four large generated JSON arrays as new verbosity findings, with no gating rows.

Each nonzero candidate was refitted to the unchanged authored initial shape
using the existing 0.25 mm equilibrium criterion. Attachment positions, mesh
topology, embeddings, motion, shear parameters and viscosity were unchanged.
The predeclared screen required every sampled volume ratio in [0.5, 2.0], total
volume change below 1%, and at least 80% of both peak and time-RMS surface
movement. A passing screen would still require Bevy and visual validation.

## Results

These are 240 Hz CPU replays interpolating the recorded attachment frames.
They omit contact and fractional display solves; their extrema differ from the
478-frame reviewed render. Surface statistics weight embedded vertices equally.

| Element share, Pa | Left volume range | Right volume range | Peak retained | RMS retained |
| --- | --- | --- | ---: | ---: |
| 0 | 0.421–2.931 | 0.277–2.747 | 100.0% | 100.0% |
| 500 | 0.433–2.637 | 0.281–2.683 | 99.7% | 99.6% |
| 2,000 | 0.469–2.300 | 0.280–2.408 | 98.3% | 97.3% |
| 8,000 | 0.583–1.840 | 0.283–2.187 | 93.3% | 90.5% |
| 16,000 | 0.646–1.598 | 0.314–2.026 | 89.5% | 84.7% |
| 32,000 | 0.717–1.424 | 0.321–1.665 | 84.3% | 78.3% |

No candidate passed. The follow-up above 8,000 Pa used the same limits: its basis
is recorded in `followup-plan.json`. Right-side minimum compression shifted from
element 282 to the adjacent element 435 and remained excessive. At 32,000 Pa the
time-RMS movement also failed. Increasing pressure uniformly is not retained.

`mesh_flip_diagnostic.py` additionally checks all internal 2-to-3 flips that
preserve positive volume and cavity volume in both initial and reference
coordinates. Only one left cavity and no right cavity improve minimum quality.
No topology changed. This rules out that narrow repair, not all remeshing.

The next investigation should target the thin elements through a better volume
mesh or a localized pressure treatment, and retain the reviewed surface-motion
reference. The uniform penalty is not enabled and no new video was generated.

## Reproduction and preservation

From the worktree root, reuse the archived experimental kernel:

```sh
python artifacts/soft-tissue/pressure-stabilization/study.py \
  --output artifacts/soft-tissue/pressure-stabilization/reproduction-01 \
  --source-archive artifacts/soft-tissue/pressure-stabilization/study-01/source.tar.gz \
  --shares 0 500 2000 8000
python artifacts/soft-tissue/pressure-stabilization/analyze.py \
  --input artifacts/soft-tissue/pressure-stabilization/reproduction-01
```

Run `--shares 0 16000 32000` in a separate new output directory for the stronger
screen. `study-02` preserves a failed archive-reader attempt that omitted a JSON
fixture; `study-03` is the corrected successful follow-up. No physics result came
from that failed attempt. Study archives, input hashes, exit codes, build/fit logs,
fitted assets, comparison reports and the source patch are committed. Large
position binaries remain durably in these worktree directories outside Git.
The exact archive is authoritative for compiled sources when `--source-archive`
is supplied; manifest workspace hashes then describe the surrounding checkout.
