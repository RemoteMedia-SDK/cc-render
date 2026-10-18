"""Compare measured render-clock feedback with the headless main-clock fix."""
import hashlib
import json
import re
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

from live_clock_audit import inspect, lines
from startup_cadence_audit import inspect as inspect_fixed, compare

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent


def clock_samples(log):
    samples = [json.loads(re.search(r"clock=(\{.*\})", line).group(1))
               for line in log.read_text().splitlines() if "headless frame clock" in line]
    return [r for r in samples if r["previous_body_pts_ms"] is not None
            and r["previous_body_pts_ms"] < 8000]


def quantiles(values):
    return np.quantile(values, [.5, .95, .99, 1]).tolist()


def clock_report(samples):
    age = np.array([r["selected_instant_age_ms"] for r in samples])
    wall = np.diff([r["wall_elapsed_ms"] for r in samples])
    selected = np.array([r["real_delta_ms"] for r in samples[1:]])
    residual = np.abs(selected - (wall - np.diff(age)))
    assert max(residual) < 1e-6
    return {
        "samples": len(samples),
        "selected_instant_age_ms_p50_p95_p99_max": quantiles(age),
        "main_loop_interval_ms_p50_p95_p99_max": quantiles(wall),
        "selected_delta_ms_p50_p95_p99_max": quantiles(selected),
        "selected_delta_lag1_correlation": float(np.corrcoef(selected[:-1], selected[1:])[0, 1]),
        "main_loop_delta_lag1_correlation": float(np.corrcoef(wall[:-1], wall[1:])[0, 1]),
        "clock_identity_residual_max_ms": float(max(residual)),
    }


def main():
    packets = (ROOT / "lab-jump-01/packets.ndjson").read_bytes()
    responses = lines(ROOT / "lab-jump-01/responses.ndjson")
    assert [json.loads(row) for row in packets.splitlines()] == [f for r in responses for f in r["frames"]]
    assert all(r["audit"] == dict(policy="model-owned-v1", future_nonzero=0,
                                  future_channels=[], exceptions=[]) for r in responses)
    launches = json.loads((HERE / "main-clock-launches.json").read_text())
    assert len(launches) == 4 and all(r["exit_code"] == 0 for r in launches)
    manifest = json.loads((HERE / "main-clock-inputs.json").read_text())
    assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest() == digest
               for p, digest in manifest["files"].items())
    assert all(r["binary_sha256"] == manifest["files"]["target/debug/examples/tissue_lab"] for r in launches)
    baseline = inspect("render-clock-probe-01", packets)
    baseline_clock = clock_samples(HERE / "render-clock-probe-01.log")
    captures = {r["name"]: inspect(r["name"], packets) for r in launches if r["clock"] == "live"}
    clocks = {r["name"]: clock_samples(HERE / r["log"]) for r in launches if r["clock"] == "live"}
    old_fixed = inspect_fixed("lab-front-full-factor-60fps-240hz-01", packets)
    new_fixed = inspect_fixed("lab-front-main-clock-fixed-01", packets)
    fixed_comparison = compare(old_fixed[1], new_fixed[1])
    assert fixed_comparison["identical_traced_physics_and_source"]

    fig, axes = plt.subplots(3, 1, figsize=(11, 9), layout="constrained")
    all_captures = {"render-clock baseline": baseline, **captures}
    all_clocks = {"render-clock baseline": baseline_clock, **clocks}
    for name, (report, stamps, gaps, trace) in all_captures.items():
        label = name.replace("main-clock-", "main clock ")
        axes[0].plot(stamps[1:] / 1000, gaps, label=label, alpha=.75)
        samples = all_clocks[name]
        axes[1].plot([r["previous_body_pts_ms"] / 1000 for r in samples],
                     [r["selected_instant_age_ms"] for r in samples], label=label)
        if trace:
            axes[2].plot(stamps / 1000, [r["maximum_surface_displacement_m"] * 1000 for r in trace], label=label)
    axes[0].axhline(1000 / 30, color="gray", linestyle="--", linewidth=1)
    axes[0].set_ylabel("Source frame gap (ms)")
    axes[1].set_ylabel("Selected timestamp age (ms)")
    axes[2].set_ylabel("Skin displacement (mm)")
    for axis in axes:
        axis.set_xlabel("Native source time (s)")
        axis.legend(loc="upper right")
        axis.grid(alpha=.2)
    fig.suptitle("Headless clock comparison | unchanged native jump | 30 FPS / 120 Hz tissue")
    fig.savefig(HERE / "main-clock-timing.png", dpi=150)
    plt.close(fig)
    report = {
        "source_sha256": hashlib.sha256(packets).hexdigest(),
        "source_packets_unchanged_and_semantic_only": True,
        "body_and_tissue_stamps_correlated_in_gpu_readback": True,
        "baseline": {**baseline[0], "clock": clock_report(baseline_clock)},
        "runs": {name: {**value[0], "clock": clock_report(clocks[name])} for name, value in captures.items()},
        "fixed_clock_run": new_fixed[0],
        "fixed_clock_physics_regression": fixed_comparison,
        "limitations": [
            "Local prerecorded motion and renderer readback, not live inference/network/browser presentation.",
            "Live runs differ in first observation and sample times; their physics traces are not byte-comparable.",
            "Clock correction does not certify deadlines, anatomical material calibration or active avatar contact.",
            "Displayed volume minima do not bound unsampled extrema.",
        ],
    }
    (HERE / "main-clock-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
