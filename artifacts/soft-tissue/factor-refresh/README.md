# Preconditioner refresh evidence

The selected change refreshes the cached factor after two nonlinear updates.
The physical matrix, true-residual checks and material remain unchanged. See
[the runtime record](../../../docs/design/breast-soft-tissue-runtime.md).

- `plan.json`: declared scope and correctness screens.
- `cpu-comparison-final.json`: baseline, three candidates and paired repeat.
- `production-parity.json`: integrated kernel versus selected experiment.
- `bevy-comparison-final.json`: all 478 frames and both live after-runs.
- `bevy-before-01/`, `bevy-after-01/`, `bevy-after-02/`: exact source archives,
  binary hashes, inputs and run logs. The first after-run overlapped the CPU
  replay/compiler; the repeat did not overlap those task workloads.
- `tissue-tests-01.log`: 84 passes. `python-tests-01.log`: 15 passes and the
  existing ownership source-slice failure, without changing protected tests.
- `quality.xml`, `contract.xml`: unsuppressed structural findings and unchanged
  solver signature. These are supplementary to compiler and runtime evidence.

CPU profile directories live beside this folder in `boundary-runtime/` and
contain exact instrumented source archives. The `--refresh-factor` experiment
requires the pre-refresh source; rerun an archived source to reproduce an old
candidate. The current production profile needs no experiment flag:

```sh
python artifacts/soft-tissue/boundary-runtime/profile.py new-production-label --rate=120 --asset=assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json
python artifacts/soft-tissue/quality-pressure/run_bevy.py --trace-only --label new-refresh-label --output artifacts/soft-tissue/factor-refresh/new-capture
```

Use new labels/output directories. The Bevy runner expects the current
`cargo build --example tissue_lab --offline` binary and host GPU access.
These captures save numeric traces and a geometry snapshot, not a new video.
The [existing comparison video](../quality-pressure/reviewed-vs-local-volume.mp4)
remains unchanged. Actual-avatar contact and smooth playback remain unfinished.
