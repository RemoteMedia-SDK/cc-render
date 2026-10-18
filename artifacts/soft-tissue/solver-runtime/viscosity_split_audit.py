"""Compare the finite-strain damping change at unchanged source timestamps."""
import hashlib
import json
import subprocess
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

from live_clock_audit import inspect as inspect_live
from startup_cadence_audit import inspect as inspect_fixed, compare

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent


def read(path):
    return json.loads(path.read_text())


def cpu(name):
    folder = HERE / name
    run = read(folder / "run.json")
    counter = read(folder / "counters.json")
    balance = np.asarray(counter.pop("nonlinear_force_n_correction_m"))
    assert np.all(balance[:, 0] <= 1e-4) and np.all(balance[:, 1] <= 1e-7)
    assert counter["true_relative_residual"]["maximum"] <= 1e-9
    return {
        "run": run, "counters": counter, "accepted_solves": len(balance),
        "maximum_force_n": float(balance[:, 0].max()),
        "maximum_correction_um": float(balance[:, 1].max() * 1e6),
    }


def main():
    manifest = read(HERE / "viscosity-split-inputs.json")
    assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest() == h
               for p, h in manifest["files"].items())
    launches = read(HERE / "viscosity-split-launches.json")
    assert len(launches) == 2 and all(r["exit_code"] == 0 for r in launches)
    assert all(r["binary_sha256"] == manifest["files"]["target/debug/examples/tissue_lab"]
               for r in launches)
    names = ["viscosity-symmetry-01", "viscosity-split-01"]
    positions = [np.fromfile(HERE / n / "positions.bin", dtype="<f8").reshape(-1, 512, 3)
                 for n in names]
    distance = np.linalg.norm(positions[1] - positions[0], axis=2) * 1000
    source = (ROOT / "lab-jump-01/packets.ndjson").read_bytes()
    responses = [json.loads(r) for r in (ROOT / "lab-jump-01/responses.ndjson").read_text().splitlines()]
    assert [json.loads(r) for r in source.splitlines()] == [f for r in responses for f in r["frames"]]
    assert all(r["audit"] == dict(policy="model-owned-v1", future_nonzero=0,
                                  future_channels=[], exceptions=[]) for r in responses)
    before = inspect_fixed("lab-front-viscosity-symmetry-review-01", source)
    after = inspect_fixed("lab-front-viscosity-split-review-01", source)
    finer = inspect_fixed("lab-front-viscosity-split-480hz-01", source)
    rate_launch = read(HERE / "viscosity-split-rate-launch.json")
    assert rate_launch["exit_code"] == 0 and rate_launch["binary_sha256"] == launches[0]["binary_sha256"]
    assert before[1].keys() == after[1].keys()
    assert after[1].keys() == finer[1].keys()
    for stamp, row in before[1].items():
        for a, b in zip(row["regions"], after[1][stamp]["regions"], strict=True):
            for key in ["anchor_world_m", "anchor_rotation_xyzw", "name"]:
                assert a[key] == b[key]
    # Cross-rate attachment samples are not bit-identical. Report the observed
    # differences rather than certifying identical forcing for this comparison.
    rate_anchor = {
        key: float(max(np.linalg.norm(np.asarray(a[key]) - b[key])
                       for t in after[1]
                       for a, b in zip(after[1][t]["regions"], finer[1][t]["regions"], strict=True)))
        for key in ["anchor_world_m", "anchor_rotation_xyzw"]
    }
    report = {
        "source_manifest": "viscosity-split-inputs.json",
        "source_packets_unchanged_and_semantic_only": True,
        "all_478_displayed_anchor_frames_unchanged": True,
        "cpu_before": cpu(names[0]), "cpu_after": cpu(names[1]),
        "cpu_maximum_node_difference_mm": float(distance.max()),
        "gpu_before": before[0], "gpu_after": after[0],
        "gpu_comparison": compare(before[1], after[1]),
        "gpu_480hz": finer[0],
        "240_vs_480hz": compare(after[1], finer[1]),
        "rate_comparison_maximum_anchor_difference": rate_anchor,
        "live_timing": inspect_live("viscosity-split-live120-01", source)[0],
        "limits": [
            "Constitutive correction, not empirical anatomical calibration.",
            "CPU replay omits body contact and fractional display solves.",
            "One live shared-host timing run; no controlled performance comparison.",
            "Fixed-clock PNG capture is offline visual review, not interactive performance.",
            "The jump activates zero contacts; avatar compression remains unverified.",
            "The 240/480 Hz envelope/centroid comparison does not bound every vertex or establish a converged reference.",
            "Cross-rate attachment samples differ slightly; their measured maximum translation and quaternion-chord differences are reported, not treated as bit-identical forcing.",
        ],
    }
    output = HERE / "viscosity-split-front.mp4"
    probe = json.loads(subprocess.check_output([
        "/usr/bin/ffprobe", "-v", "error", "-select_streams", "v:0", "-show_frames",
        "-show_entries", "frame=best_effort_timestamp_time", "-of", "json", str(output)]))
    encoded = [round(float(f["best_effort_timestamp_time"]) * 1000) for f in probe["frames"]]
    stamps = list(after[1])
    assert encoded == [t - stamps[0] for t in stamps]
    video = {**after[0], "encoded_timestamps_match_every_source_frame": True,
             "video_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
             "inspected_raw_frames_source_ms": [2200, 7633], "performance_claim": False}
    (HERE / "viscosity-split-video-validation.json").write_text(json.dumps(video, indent=2) + "\n")
    (HERE / "viscosity-split-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    fig, axes = plt.subplots(2, 1, figsize=(10, 7), sharex=True, layout="constrained")
    for (label, item) in [("Previous coupled damping", before), ("Separated shape/volume damping", after)]:
        t = np.array(list(item[1])) / 1000
        axes[0].plot(t, [r["maximum_surface_displacement_m"] * 1000 for r in item[1].values()], label=label)
        axes[1].plot(t, [min(s["metrics"]["minimum_volume_ratio"] for s in r["regions"])
                         for r in item[1].values()], label=label)
    axes[0].set_ylabel("Maximum skin displacement (mm)")
    axes[1].set_ylabel("Minimum tetrahedron volume ratio J")
    axes[1].set_xlabel("Unchanged source motion time (s)")
    for ax in axes:
        ax.grid(alpha=.2)
        ax.legend(loc="best")
    fig.suptitle("Finite-strain damping correction | fixed 60 FPS / 240 Hz\nIdentical coefficients, avatar and attachment frames; not an anatomical fit")
    fig.savefig(HERE / "viscosity-split-response.png", dpi=140)
    plt.close(fig)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
