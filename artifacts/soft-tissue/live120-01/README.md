# Live 120 Hz cost of the accepted tissue candidate

Same protocol as `factor-refresh/` (the last live measurement): the ARDY jump
(`lab-jump-01/packets.ndjson`, unchanged), Bevy realtime clock, 120 Hz tissue,
32 iterations, 30 FPS output, trace only. `run.sh` interleaves four setups and
repeats them; `run-repeat.sh` re-ran the two setups that had a stalled run.
`summarize.py` writes `summary.json`. The host was shared with other sessions
(ARDY daemon, condo capture, a Rust build; load 3-8), so delivery numbers vary
run to run; medians below.

`surface_ms` (new in the tissue trace) times the displayed skin update per
output frame: embedding, smoothing and surface metrics, including the crease
metric.

| setup | runs | readbacks (range) | p95 gap | end drift | skin update p50 / p95 | elastic solve |
|---|---|---|---|---|---|---|
| selected, linear | 5 | 205 (150-211) | 68.5 ms | 61 ms | 0.32 / 0.38 ms | 4.25 s |
| selected, mls + smoothing 4 | 3 | 208 (204-208) | 59.0 ms | 25 ms | 0.47 / 0.52 ms | 4.20 s |
| s40-k32-v4, linear | 3 | 217 (209-219) | 51.5 ms | 28 ms | 0.33 / 0.42 ms | 3.97 s |
| **s40-k32-v4, mls + smoothing 4** | 5 | **210 (153-223)** | **64.2 ms** | **31 ms** | **0.48 / 0.56 ms** | **3.92 s** |

- The smooth skin layer costs about 0.15 ms more per output frame (0.33 to
  0.48 ms), under 0.5% of a 33 ms frame.
- The accepted 4 Pa s material is slightly cheaper to solve than the selected
  asset (3.9-4.0 s vs 4.2 s of elastic solve for the 7.95 s clip).
- Delivery is not measurably different between setups: every setup's best runs
  deliver 210-223 readbacks, and both setups repeated five times had a stalled
  run (150-179 readbacks, 250 ms gaps) with the physics itself slower in that
  run, i.e. host contention. The previous live result was 196 readbacks and a
  73 ms p95 gap.
- Still not smooth 30 FPS: 30 FPS over 7.95 s is about 239 readbacks, and
  every setup has gaps above 50 ms. That limit predates this candidate.

The jump has no contacts; contact cost with the softer material is unmeasured.
