"""Read-only audit of the first-observation startup fix and output cadences."""
import copy
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = Path(__file__).with_name("startup-cadence-comparison.json")


def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def percentile(values, fraction):
    ordered = sorted(values)
    index = (len(ordered) - 1) * fraction
    lo, hi = math.floor(index), math.ceil(index)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (index - lo)


def physics(row):
    result = copy.deepcopy(row)
    for field in ("sampling_ms", "elasticity_ms", "collision_ms"):
        del result["integration"][field]
    return result


def inspect(name, packets):
    folder = ROOT / name
    config = json.loads((folder / "config.json").read_text())
    capture = json.loads((folder / "capture.json").read_text())
    frames = rows(folder / "frames.ndjson")
    trace = rows(folder / "tissue.ndjson")
    stamps = [frame["pts_ms"] for frame in frames]
    assert (folder / "packets.ndjson").read_bytes() == packets
    assert stamps == [row["pts_ms"] for row in trace]
    assert len(stamps) == len(set(stamps)) == capture["frames"]
    assert all(0 < b - a <= math.ceil(1000 / config["fps"])
               for a, b in zip(stamps, stamps[1:]))
    source = [json.loads(line) for line in packets.splitlines()]
    assert capture["sent_packets"] == len(source)
    assert stamps[-1] == capture["last_pts_ms"] == source[-1]["pts_ms"]
    intervals = [b["wall_elapsed_ms"] - a["wall_elapsed_ms"]
                 for a, b in zip(frames, frames[1:])]
    playback = (frames[-1]["wall_elapsed_ms"] - frames[0]["wall_elapsed_ms"]) / 1000
    source_span = (stamps[-1] - stamps[0]) / 1000
    report = {
        "directory": name, "output_fps": config["fps"],
        "tissue_hz": int(config["tissue_hz"]), "capture": capture,
        "first_source_ms": stamps[0], "source_span_seconds": source_span,
        "first_frame_wall_ms": frames[0]["wall_elapsed_ms"],
        "playback_capture_seconds": playback,
        "playback_wall_over_source": playback / source_span,
        "consumer_interval_ms_p50_p95_p99_max":
            [percentile(intervals, p) for p in (.5, .95, .99, 1)],
        "stage_seconds": {key: sum(row["integration"][key + "_ms"] for row in trace) / 1000
                          for key in ("sampling", "elasticity", "collision")},
        "peak_surface_displacement_mm": max(row["maximum_surface_displacement_m"] for row in trace) * 1000,
        "minimum_element_volume_ratio": min(region["metrics"]["minimum_volume_ratio"]
                                             for row in trace for region in row["regions"]),
        "maximum_skin_error_mm": max(row["skin_error_m"] for row in trace) * 1000,
        "maximum_active_contact_samples": max(row["integration"]["active_contact_samples"] for row in trace),
    }
    return report, {row["pts_ms"]: physics(row) for row in trace}


def compare(a, b):
    common = sorted(a.keys() & b.keys())
    return {
        "common_source_timestamps": len(common),
        "identical_traced_physics_and_source": a == b,
        "maximum_surface_envelope_difference_mm": max(
            abs(a[t]["maximum_surface_displacement_m"] - b[t]["maximum_surface_displacement_m"])
            for t in common) * 1000,
        "maximum_centroid_offset_difference_mm": max(
            math.dist(ar["free_node_centroid_offset_m"], br["free_node_centroid_offset_m"])
            for t in common for ar, br in zip(a[t]["regions"], b[t]["regions"], strict=True)) * 1000,
    }


def main():
    packets = (ROOT / "lab-jump-01/packets.ndjson").read_bytes()
    responses = rows(ROOT / "lab-jump-01/responses.ndjson")
    assert [json.loads(line) for line in packets.splitlines()] == [f for r in responses for f in r["frames"]]
    assert all(r["audit"] == dict(policy="model-owned-v1", future_nonzero=0,
                                  future_channels=[], exceptions=[]) for r in responses)
    names = {
        "before_startup_fix_60fps_240hz": "lab-front-full-opt3-240hz-01",
        "after_startup_fix_60fps_240hz": "lab-front-opt3-startup-60fps-240hz-01",
        "after_startup_fix_30fps_240hz": "lab-front-opt3-30fps-240hz-02",
        "after_startup_fix_30fps_120hz": "lab-front-opt3-30fps-120hz-01",
    }
    inspected = {key: inspect(name, packets) for key, name in names.items()}
    traces = {key: value[1] for key, value in inspected.items()}
    regression = compare(traces["before_startup_fix_60fps_240hz"], traces["after_startup_fix_60fps_240hz"])
    assert regression["identical_traced_physics_and_source"]
    report = {
        "source_sha256": hashlib.sha256(packets).hexdigest(),
        "source_packets_unchanged_and_semantic_only": True,
        "runs": {key: value[0] for key, value in inspected.items()},
        "startup_regression_at_60fps": regression,
        "30fps_120_vs_240hz": compare(traces["after_startup_fix_30fps_120hz"], traces["after_startup_fix_30fps_240hz"]),
        "30_vs_60fps_at_240hz": compare(traces["after_startup_fix_30fps_240hz"], traces["after_startup_fix_60fps_240hz"]),
        "limitations": [
            "Offline GPU render/readback, PNG saving disabled, single runs on a shared host; not real-time deadlines.",
            "30 FPS initializes at 66 ms and 60 FPS at 50 ms: physical initial conditions and integration grid phase differ.",
            "Cross-cadence differences are measured only at common source timestamps and do not isolate output-cadence error.",
            "Envelope and centroid comparisons do not bound every vertex or establish anatomical calibration.",
            "All runs have zero active contact samples; actual-avatar compression remains unvalidated.",
        ],
    }
    OUTPUT.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
