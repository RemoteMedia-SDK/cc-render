# Hand ribcage clamp and finger conformance (exception-assisted)

Implements the owner-approved exception in
[`hand-ribcage-clamp-exception.md`](../../../../docs/design/hand-ribcage-clamp-exception.md).
Opt-in with `CC_AVATAR_HAND_RIBCAGE_CLAMP=1`. ARDY packets and service history
are untouched; runs with the clamp are labelled exception-assisted.

## What the measurements showed

The ribcage is torso skin dominated by Spine01/02, RibsTwist, Clavicle and
residual Breast weights, outside the simulated patches, closed by each tissue
region's posterior backing cap (`summary.ndjson` has every iteration):

- On seed-739 the **wrist and palm never enter the ribcage** (`clamp-05-palm`).
  What passes through is the **right hand's fingers, up to 23 mm**.
- A rigid whole-hand translation (`clamp-05-1`, kept as `=whole`) needs up to
  49 mm with ~40 mm frame-to-frame jumps and raises tissue overlap to 46 mm:
  rejected.
- Earlier iterations exposed and fixed: cap side walls counted as chest (false
  "penetrations" of 20–170 mm while standing; tetgen wall nodes required a
  coplanarity filter), a greedy push-out that over-corrected (now Dykstra's
  minimal-norm projection), and withheld unconverged corrections that made the
  hand pop (now bounded corrections are always applied, residual reported).

## Final behaviour (`clamp-08-final`, identical to the reviewed `clamp-06-fingers`)

- Wrist/palm guard: no correction needed in this clip.
- Finger conformance: renderer-owned fingers bend at the knuckles (flexion
  within the rig's limits, or up to 0.5 rad knuckle extension) by the smallest
  amount that keeps each finger's spheres 1 mm outside the ribcage. Right hand
  up to 0.24 rad (14°), **zero residual** in every frame; left hand untouched.
- All 478 frames through 8000 ms; the tissue path's per-frame check that its
  source-time FK matches the displayed skeleton passed, fingers included.
- Tissue overlap 5.4 mm (pressure cap), 20 contact frames, 14 subdivisions.
- No-contact jump: clamp off is identical to the baseline in every value
  (`jump-comparison.json`); clamp on reports no penetration and no bend.

## Review

`review/finger-conformance-{real-time,quarter-speed}.mp4`: front chest crop,
generated hand (10 kPa cap) versus conforming fingers. The front view shows
depth poorly. `review/finger-conformance-side-quarter-speed.mp4` uses the only
side camera, which shows the uncorrected left hand; the corrected right hand
is on the far side. Fingers still press into breast tissue, which responds
under the pressure cap; only the ribcage is a hard limit (owner scope).

Not done: live 120 Hz performance with the clamp, anatomical finger limits.
Tests: 94 tissue + 6 clamp (`hand_clamp_tests.rs`), 5 `tissue_lab` example;
Python 15 pass + the pre-existing ownership failure. `traces.tar.gz` holds the
whole, palm, final and clamped-jump traces (SHA-256 in the manifest).
