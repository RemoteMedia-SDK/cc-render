# Seed-739 residual precision and line-search checkpoint

This is **not a completed contact validation**. It resolves the 7666.67 ms
nonlinear failure and exposes the next, different blocker at 7671.35 ms.
No source poses, timing, material, attachments, contact limits or nonlinear
force/correction limits were changed.

## Capture

`CC_TISSUE_FAILED_STATE_DIR` (new, diagnostic only) records every
`NonlinearSolveFailed` step as f64 bits: tissue state, BDF2 history, interval,
end frame, contact planes and the failed iterate. The ignored probe
`failed_state_precision` reloads a dump, **asserts a bit-exact replay of the
recorded error**, then measures it. `failed-states/failed-state-002.json` is
the final 7666.666667..7666.731771 ms, depth-6 failure (the run's residual
was 1.1077148e-6 N; the earlier `bevy-retry-01` run reported 1.1120567e-6 N).

## Measurements at the failed state (`probe-before-fix-002.log`)

| Quantity | Value |
|---|---|
| Interval / previous dt / effective BDF2 dt | 65.104 us / 4.1667 ms / 64.118 us |
| Force limit (unchanged) | 1.3320e-7 N |
| Residual; share on node 110 (contact node) | 1.1077e-6 N; 99.996 % |
| Node-110 terms: contact / viscous / inertia / elastic | 14.3 / 17.3 / 10.7 / 6.9 N |
| Gradient world vs exactly y-shifted evaluation | 3.9e-12 N |
| Representability floor, H * (ulp/2) RMS | 6.6e-9 N |
| Tangent eigenvalues / condition | 7.46e4 .. 6.70e7 N/m / 899 |
| One plain Newton step from the failed iterate | 7.29e-9 N, then a 7.9e-9 N floor |

The residual is **not** precision-limited: the limit is reachable with a 17x
margin and the tangent is benign. The failure is the line search. From
iteration 2 the Newton step's predicted decrease is ~1e-19 J, while each trial
energy carries ~6e-15 J of evaluation error (contact force ~64 N times ~1e-16 m
depth rounding; per-term table in the probe). Armijo with 1e-15 J slack rejects
the full step and accepts random 1/128 steps; the residual falls ~0.6 % per
iteration until the 32-iteration budget is exhausted. Even an exact f64 energy
(rounding ~ eps*|E| = 2e-18 J) could only certify |g| ~ 4e-7 N, above the
limit, which is also why coordinate-centering could not fix the replay.

## Change

`descend` keeps the energy Armijo test (constant 1e-4, slack 1e-15). Only when
it fails and the trial energy is within 1e-6 * |E| (every term is nonnegative)
does it test the approximate Wolfe conditions of Hager and Zhang
(doi:10.1137/030601880): `(2*1e-4 - 1) phi'(0) >= phi'(a) >= 0.9 phi'(0)`,
using the accurate gradient. On a quadratic model the first inequality is the
same Armijo condition. Convergence acceptance is unchanged.

## Results

- The recorded state now converges: `recorded_avatar_contact_step_converges_once_descent_uses_the_gradient`
  (fixture `src/cc_render/soft_tissue/fixtures/contact_line_search_failed_state.json`).
  It and the synthetic noisy-quadratic test both fail with the fallback
  disabled (`regression-before.log`) and pass with it.
- 89 focused tissue tests pass (87 previous + 2); the probe is ignored.
  `tissue_lab` example: 5 pass. Python: 15 pass, the pre-existing
  `test_root_acceptance_does_not_rewrite_the_candidate` failure remains.
- The no-contact jump (`runs/approximate-wolfe-jump-01`) is **identical** to
  the saved baseline in every non-timing value, all 478 frames through 8000 ms,
  peak 32.5599 mm, 66 subdivisions, zero contacts (`jump-comparison.json`).
- Contact replay (`runs/approximate-wolfe-01`, `-02`) passes 7666.67 ms and
  stops at **7671.354167..7671.419271 ms, depth 6: `ContactUnresolved`**. Six
  intermediate nonlinear failures were recovered by subdivision as designed.

## Next blocker: contact on attachment-dominated skin

`approximate-wolfe-02` logs the final planes. All three lie in right-breast
tet 587, nodes [52, 41, 164, 110]; **52, 41 and 164 are Fixed**. The two skin
samples lie on edge 52-110 with only **11.2 % and 22.3 %** of their position on
the single free node. Over eight rounds the penalty rises 198x (to ~20 kN)
while depth moves only 106.7 -> 104.9 um (into the 500 um clearance band; the
actual gap stays ~0.38 mm). The native arm reaches skin the model treats as
mostly chest-kinematic, so no penalty can satisfy the 100 um residual. This is
a modelling/policy decision, not a solver defect: options include a finer or
compliant attachment rim under contactable skin, or an owner-approved rule for
samples whose embedding is mostly fixed. Nothing here selects one.

Probes 000/003/005 (recovered steps, dt 1-4 ms, 1e8-2e9 N/m penalties) show
slow or cycling Newton with these very stiff penalties: tangent quality
(Gauss-Newton terms), not precision. Subdivision handles them today.

`traces.tar.gz` holds the ignored NDJSON inputs/traces of the contact runs
(SHA-256 in `traces-manifest.json`); extract from the worktree root. Per-run
geometry snapshots were not retained; rerun the launch to regenerate them.
No rendered contact video exists yet.
