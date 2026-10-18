"""Audit nonlinear acceptance separately from linear residuals and visual realism."""
import hashlib
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

from live_clock_audit import inspect as inspect_live, lines
from startup_cadence_audit import inspect as inspect_fixed, compare

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent


def cpu(name):
    folder = HERE / name
    counters = json.loads((folder / "counters.json").read_text())
    values = np.array(counters["nonlinear_force_n_correction_m"])
    passed = (values[:, 0] <= 1e-4) & (values[:, 1] <= 1e-7)
    report = {
        "directory": name, "observed_solves": len(values),
        "met_both_limits": int(passed.sum()),
        "missed_either_limit": int((~passed).sum()),
        "force_norm_n_p50_p95_p99_max": np.quantile(values[:, 0], [.5, .95, .99, 1]).tolist(),
        "correction_um_p50_p95_p99_max": (np.quantile(values[:, 1], [.5, .95, .99, 1]) * 1e6).tolist(),
        "run": json.loads((folder / "run.json").read_text()),
        "timing_claim": False,
    }
    return report, values, np.fromfile(folder / "positions.bin", dtype="<f8").reshape(-1, 3)


def main():
    packets = (ROOT / "lab-jump-01/packets.ndjson").read_bytes()
    responses = lines(ROOT / "lab-jump-01/responses.ndjson")
    assert [json.loads(row) for row in packets.splitlines()] == [f for r in responses for f in r["frames"]]
    assert all(r["audit"] == dict(policy="model-owned-v1", future_nonzero=0,
                                  future_channels=[], exceptions=[]) for r in responses)
    inputs = json.loads((HERE / "nonlinear-strict-inputs.json").read_text())
    assert all(hashlib.sha256(Path(path).read_bytes()).hexdigest() == digest
               for path, digest in inputs["files"].items())
    launches = json.loads((HERE / "nonlinear-strict-launches.json").read_text())
    assert len(launches) == 2 and all(r["exit_code"] == 0 for r in launches)
    assert all(r["binary_sha256"] == inputs["files"]["target/debug/examples/tissue_lab"] for r in launches)
    workloads = {name: cpu(name) for name in ["nonlinear-balance-8-01", "nonlinear-balance-32-01", "nonlinear-strict-32-01"]}
    strict = workloads["nonlinear-strict-32-01"]
    assert strict[0]["missed_either_limit"] == 0
    differences = {name: (np.quantile(np.linalg.norm(value[2] - strict[2], axis=1), [.5, .95, .99, 1]) * 1000).tolist()
                   for name, value in workloads.items() if name != "nonlinear-strict-32-01"}
    before = inspect_fixed("lab-front-main-clock-fixed-01", packets)
    after = inspect_fixed("lab-front-nonlinear-strict-fixed-01", packets)
    assert before[1].keys() == after[1].keys()
    for stamp, old in before[1].items():
        for a, b in zip(old["regions"], after[1][stamp]["regions"], strict=True):
            for key in ["name", "anchor_world_m", "anchor_rotation_xyzw"]:
                assert a[key] == b[key], (stamp, key)
    assert after[0]["minimum_element_volume_ratio"] > 0
    assert after[0]["maximum_skin_error_mm"] < .2
    video = inspect_live("nonlinear-strict-video-01", packets)[0]
    fig, axes = plt.subplots(3, 1, figsize=(11, 9), layout="constrained")
    for name, (_, values, _) in workloads.items():
        for axis, column, scale in [(axes[0], 0, 1000), (axes[1], 1, 1e6)]:
            axis.semilogy(np.linspace(0, 100, len(values)), np.sort(values[:, column]) * scale,
                          label=name.replace("nonlinear-", "").replace("-01", ""))
    for axis in axes[:2]:
        axis.axhline(.1, color="gray", linestyle="--", label="Acceptance limit")
        axis.set_xlabel("Empirical percentile of final solve state")
    axes[0].set_ylabel("Free-force norm (mN)")
    axes[1].set_ylabel("Remaining correction (µm)")
    for label, (_, trace) in [("Previous eight-pass cap", before), ("Strict convergence + retry", after)]:
        axes[2].plot(np.array(list(trace)) / 1000,
                     [r["maximum_surface_displacement_m"] * 1000 for r in trace.values()], label=label)
    axes[2].set_xlabel("Native source time (s)")
    axes[2].set_ylabel("Skin displacement (mm)")
    for axis in axes:
        axis.grid(alpha=.2)
        axis.legend(loc="upper left")
    fig.suptitle("Nonlinear convergence | unchanged material and source motion\nCPU final-state audit above; fixed-clock GPU replay below")
    fig.savefig(HERE / "nonlinear-convergence.png", dpi=150)
    plt.close(fig)
    report = {
        "source_packets_unchanged_and_semantic_only": True,
        "source_timestamps_and_attachment_frames_unchanged": True,
        "cpu": {name: value[0] for name, value in workloads.items()},
        "full_node_difference_from_strict_mm_p50_p95_p99_max": differences,
        "fixed_capture": after[0], "fixed_physics_change": compare(before[1], after[1]),
        "visual_capture": video,
        "limitations": [
            "Numerical acceptance limits are engineering choices, not anatomical calibration.",
            "CPU replay interpolates saved attachment observations; it is not native body/contact acceptance.",
            "GPU captures overlap a separate bedding test; timings are not an isolated performance comparison.",
            "Both GPU captures have zero active contacts; actual-avatar compression remains unverified.",
        ],
    }
    (HERE / "nonlinear-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
