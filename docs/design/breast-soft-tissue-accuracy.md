# BDF2 solve accuracy and playback, 2026-10-18

After `2c8bd05`, BDF2 nonlinear stopping limits depend on the effective
differentiation interval. This improves the selected avatar's 120 Hz throughput
while preserving the reviewed response within measured numerical error. The
prototype remains opt-in; live playback still misses its target.

## Policy and rejected candidates

The preceding solver used a fixed 0.1 mN global free-force limit and 0.1 um
maximum node correction. A tenfold relaxation reduced the 120 Hz CPU replay from
9.09 to 7.15 seconds and changed nodes by at most 8.6 um. However, it failed the
independent nonlinear oscillator's time-convergence test: state errors at
240/480/960 Hz were 0.021734/0.011672/0.012570 mm. That candidate was rejected.

For BDF2, let `h` be the effective interval already used to recover velocity from
position history, and `s = min(180*h, 1)`. The new limits are:

```
global free-force norm: max(1e-3 * s^2, 1e-8) N
maximum node correction: max(1e-6 * s^4, 1e-12) m
```

At steady 120 Hz BDF2, `h = 1/180 s`, giving 1 mN and 1 um. Smaller steps tighten
both limits. A position correction contributes correction/h to velocity, so the
fourth-power position budget gives a third-power velocity budget. The caps and
numerical floors are engineering choices. This is a stopping policy, not an
a-posteriori global error bound, a material parameter or anatomical calibration.

Relating nonlinear error to integration accuracy follows the general principle
described in [CVODE's nonlinear convergence discussion](https://sundials.readthedocs.io/en/v7.5.0/cvode/Mathematics_link.html#nonlinear-solve).
This implementation does not reproduce CVODE's adaptive local-error estimator
or claim that its documentation prescribes these constants or powers.

Applying the fourth-power rule to implicit Euler broke its existing one-iteration
retry fixture. That candidate is also retained and rejected. Implicit Euler keeps
its original 0.1 mN / 0.1 um limits. Offline equilibrium, material laws, linear
solver tolerances, iteration/retry caps, source motion and all test assertions
remain unchanged.

## Evidence

All 76 tissue tests pass. The independent oscillator again measures
0.021734/0.006057/0.001595 mm at 240/480/960 Hz, passing the original convergence
ratio and finest-step error checks. Compression/release, rollback, source
ownership and subdivision checks also pass. The Python suite remains 15 passes
and the existing ownership source-slice failure.

The selected 120 Hz CPU replay takes 7.58 seconds versus 9.09 before (about 17%
lower in these shared-host runs). Both use 213 subdivisions. Across 452,196 node
samples, maximum/RMS position differences are 8.615/0.736 um. These recorded
attachment interpolations are not an exact continuous reference or live rendering.

Bevy's 240 Hz fixed-clock capture retains all 478 frames and exactly the same
attachment transforms. Against the preceding strict solver, maximum/RMS node
differences are 0.751/0.041 um; the maximum surface-envelope difference is
0.467 um. Peak displacement remains 36.151 mm. The separate accuracy study uses
a declared 0.1 mm trajectory screen; the existing 1 um equivalence gate is not
modified. The reviewed video remains the visual reference; no replacement images
were generated.

Live 120 Hz playback produces 141 readbacks, up from 110. It takes 8.259 wall
seconds for 7.934 source seconds, with 0.325 seconds of end drift. The p95
delivery gap falls from 310 to 183 ms. This remains visibly uneven, not smooth
real-time playback; readbacks are not browser presentations. The jump activates
no contacts, so actual-avatar compression remains unverified.

Sources, failed test logs, CPU trajectories and launch details are preserved under
`artifacts/soft-tissue/boundary-runtime/`: `cadence120-*`, `accuracy-*` and
`bevy-accuracy/`. Full-repository structural review retains two findings for
recent solver edits and a duplicated helper in the preserved experiment script;
the focused production-source delta has no findings. No structural findings or
test failures were suppressed.
