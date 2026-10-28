# Bounded contact pressure

Owner-selected after the compliant rim showed unbounded penalty escalation
(up to ~20 kN) crushing local volume under the cupping hands. ARDY motion, poses,
material, attachments and the 100 µm residual for uncapped samples are
unchanged. Nothing is default-enabled outside the opt-in tissue path.

## Change

- `PlaneContact::max_force_n` caps each sample's reaction at
  `max pressure × represented area` (split across witnesses like stiffness).
  Beyond the cap the energy is linear (C¹, convex); tangent 0. `f64::INFINITY`
  keeps the previous quadratic penalty.
- `CC_AVATAR_TISSUE_CONTACT_PRESSURE_PA` (default 10 kPa; `inf` disables).
  The value is an engineering bound far below the 500 kPa bulk modulus, not a
  calibration. Capped overlap is permitted and reported per frame
  (`force_limited_contact_samples`, `maximum_force_limited_penetration_m`),
  warn-by-default like seat overlap; `CC_AVATAR_TISSUE_STRICT_CONTACT=1`
  rejects it. Penalty escalation now applies only to uncapped samples.
- Two contact-query defects exposed once capped samples rest near the hand:
  - Conservative advancement on grazing motion exhausted 256 iterations
    1.4–3.8e-8 m short of its 1e-8 m tolerance (`sweep_replay.py`
    reproduces the logged inputs). The tolerance is now 1e-6 m (1 % of
    RESIDUAL); the sweep test's hit bound was widened to that tolerance.
  - The inner-clearance check is swept, so a sample that started a step in
    permitted overlap was flagged even while receding. Only planes whose
    sample starts inside and ends outside are excused; entering or remaining
    overlap must still be at the cap.

## Results (seed-739 chest rub, `summary.ndjson`)

Every configuration completes all 478 frames through 8000 ms (first ever
for this clip). Contact starts at 7683 ms.

| Cap | Asset | Peak surface | Right volume ratio | Max overlap* | Hand force | Subdivisions |
|---|---|---|---|---|---|---|
| 2 kPa | fixed rim | 6.8 mm | 0.88–1.15 | 2.6 mm | 2.5 N | 5 |
| 10 kPa | fixed rim | 20.5 mm | 0.62–1.30 | 2.7 mm | 12.4 N | 7 |
| 50 kPa | fixed rim | 32.8 mm | 0.56–1.98 | 2.9 mm | 54.5 N | 33 |
| 2 kPa | rim 1e-5 | 7.0 mm | 0.88–1.15 | 2.6 mm | 2.6 N | 7 |
| 10 kPa | rim 1e-5 | 18.3 mm | 0.73–1.37 | 3.0 mm | 12.7 N | 9 |
| 50 kPa | rim 1e-5 | 32.5 mm | 0.55–1.75 | 3.0 mm | 55.2 N | 36 |

\*Into the 0.5 mm clearance band, for simulated breast skin only; hand overlap
with the rest of the body is not measured. The default-cap jump is identical to
the baseline in every value (`jump-comparison.json`).

`../pressure-cap-01` is the first sweep before the two query fixes: two runs
completed, four stopped on the grazing sweep or the inherited-overlap fixed
point (gzipped logs and `summary.ndjson` retained).

## Review and caveats

`review/cap-comparison-{real-time,quarter-speed}.mp4`: chest crop, 7300–8000 ms,
2 / 10 / 50 kPa on the selected fixed-rim asset. **The hands visibly pass into
the breasts**; this is the permitted overlap, not repaired motion. The owner
has asked about stopping the hand at the body instead; that is a motion
change governed by the model-owned-motion exception process.

Rendered and trace-only runs are not bit-identical: a 5.6e-9 m anchor
difference appears at 1583 ms and decays to 3e-15 m by 7600 ms, then contact
amplifies roundoff to ~2.7 mm by 8000 ms. Both are valid trajectories; contact
is strongly sensitive to roundoff. Not investigated further.

Not done: live 120 Hz performance check, anatomical calibration of the cap.
Tests: 94 tissue, 5 `tissue_lab` example; Python 15 pass + the pre-existing
`test_root_acceptance_does_not_rewrite_the_candidate` failure.
