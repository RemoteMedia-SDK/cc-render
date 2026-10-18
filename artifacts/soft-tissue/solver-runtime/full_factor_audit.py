"""Compare the adopted bounded complete factor with the converged IC(0) solver."""
import hashlib
import json
from pathlib import Path
import runpy

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
CADENCE = runpy.run_path(str(HERE / "startup_cadence_audit.py"))


def read(path):
    return json.loads(path.read_text())


def cpu_comparison():
    before = HERE / "full-factor-baseline-opt3-01"
    after = HERE / "full-factor-integrated-opt3-01"
    a = np.fromfile(before / "positions.bin", dtype="<f8").reshape(-1, 512, 3)
    b = np.fromfile(after / "positions.bin", dtype="<f8").reshape(a.shape)
    assert np.isfinite(a).all() and np.isfinite(b).all()
    difference = np.linalg.norm(a - b, axis=2) * 1000
    assert difference.max() < 1e-4
    baseline, current = read(before / "run.json"), read(after / "run.json")
    counters = read(after / "counters.json")
    assert counters["true_relative_residual"]["maximum"] <= 1e-9
    assert counters["true_relative_residual"]["count"] == counters["linear_solves"]
    assert all(baseline[key] == current[key] for key in ("steps", "regions", "rate_hz", "subdivisions"))
    return {
        "baseline": baseline, "current": current, "current_counters": counters,
        "maximum_node_difference_mm": float(difference.max()),
        "node_frames_compared": int(difference.size),
        "solve_time_reduction_percent": 100 * (1 - current["solve_seconds"] / baseline["solve_seconds"]),
        "scope": "Same sequential CPU workload; interpolated recorded attachment frames, no contact/render/fractional display solve.",
    }


def gpu_comparison(before, after, packets):
    baseline, a = CADENCE["inspect"](before, packets)
    current, b = CADENCE["inspect"](after, packets)
    assert a.keys() == b.keys()
    assert read(ROOT / before / "config.json") == read(ROOT / after / "config.json")
    assert (ROOT / before / "tissue-asset.json").read_bytes() == (ROOT / after / "tissue-asset.json").read_bytes()
    for time in a:
        assert a[time]["integration"] == b[time]["integration"]
        for ar, br in zip(a[time]["regions"], b[time]["regions"], strict=True):
            for key in ("name", "anchor_world_m", "anchor_rotation_xyzw"):
                assert ar[key] == br[key]
    differences = CADENCE["compare"](a, b)
    assert differences["maximum_surface_envelope_difference_mm"] < 1e-4
    assert differences["maximum_centroid_offset_difference_mm"] < 1e-4
    return {"baseline": baseline, "current": current, "differences": differences,
            "unchanged_source_attachment_frames_and_contact_subdivision_counts": True}


def main():
    packets = (ROOT / "lab-jump-01/packets.ndjson").read_bytes()
    normal = gpu_comparison("lab-front-opt3-30fps-120hz-01", "lab-front-full-factor-30fps-120hz-01", packets)
    review = gpu_comparison("lab-front-opt3-startup-60fps-240hz-01", "lab-front-full-factor-60fps-240hz-01", packets)
    uncapped = gpu_comparison("lab-front-full-factor-paced-30fps-120hz-01", "lab-front-full-factor-fast-30fps-120hz-02", packets)
    assert uncapped["differences"]["identical_traced_physics_and_source"]
    source = read(HERE / "full-factor-inputs.json")
    launches = read(HERE / "full-factor-scheduler-launches.json")
    assert len(launches) == 2
    for launch, mode, flag in zip(launches, ("paced", "uncapped"), ("0", "1"), strict=True):
        assert launch["mode"] == mode and launch["environment"]["CC_RENDER_FAST"] == flag
        assert launch["exit_code"] == 0
        assert launch["binary_sha256"] == source["files"]["target/debug/examples/tissue_lab"]
        assert launch["directory"] == uncapped["baseline" if mode == "paced" else "current"]["directory"]
    workspace = HERE.parents[2]
    assert all(hashlib.sha256((workspace / p).read_bytes()).hexdigest() == h for p, h in source["files"].items())
    report = {
        "source_sha256": hashlib.sha256(packets).hexdigest(),
        "cpu": cpu_comparison(), "30fps_120hz": normal, "60fps_240hz": review,
        "paced_vs_uncapped_30fps_120hz": uncapped,
        "source_manifest": "full-factor-inputs.json",
        "scheduler_launches": "full-factor-scheduler-launches.json",
        "decision": "Retain bounded complete factorization; preserve original incomplete pattern for larger/denser cages.",
        "limitations": [
            "Single runs on a shared host; these are not end-to-end interactive latency or deadline guarantees.",
            "The default offline captures retain ScheduleRunnerPlugin FPS pacing; their wall time includes its wait.",
            "CC_RENDER_FAST=1 removes that wait only for the explicitly uncapped offline capture; fixed source-time sampling is unchanged.",
            "GPU envelope/centroid differences do not bound every skin vertex; the CPU comparison covers all volumetric node positions.",
            "Actual-avatar compression and anatomical material calibration remain unvalidated.",
        ],
    }
    output = HERE / "full-factor-comparison.json"
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"report": str(output), "cpu_reduction_percent": report["cpu"]["solve_time_reduction_percent"],
                      "normal_playback_seconds": normal["current"]["playback_capture_seconds"],
                      "review_playback_seconds": review["current"]["playback_capture_seconds"],
                      "uncapped_playback_seconds": uncapped["current"]["playback_capture_seconds"]}, indent=2))


if __name__ == "__main__":
    main()
