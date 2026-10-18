# Coordinate-centering experiment (not integrated)

The exact saved kernel is compared with a translated Newton solve, preserving
material, contact gaps and all convergence limits. Source copies, compiler argv,
outputs and binary hashes are retained. No model poses were modified.

The first 27-case probe passes both ways. An oblique contact probe, using the
observed witness direction, fails one of 27 cases in world coordinates and none
when centered. Previously successful cases differ by at most 1.2732e-13 m.
The 60 core solver tests pass with the experiment.

However, the actual Bevy replay still fails at 7666.796875..7666.861979 ms at
depth 6, with 1.7243912e-6 N residual and 9.5312478e-14 m correction. This does
not resolve actual-avatar contact. The experiment was removed from production;
`c098357` remains the solver baseline. The exact experimental patch and Bevy
source archive remain here, along with failed rendered frames in the associated
capture folder. `restore-build.log` records rebuilding the baseline binary.

A small Newton correction alone is not evidence of a converged residual; see
[SUNDIALS stopping criteria](https://sundials.readthedocs.io/en/v7.8.0/kinsol/Mathematics_link.html#nonlinear-iteration-stopping-criteria).
No tolerance was relaxed to accept the failed replay. Further work should
measure residual evaluation precision on the actual failed state.
