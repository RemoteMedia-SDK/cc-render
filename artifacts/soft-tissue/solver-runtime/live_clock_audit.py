"""Audit native-clock lab captures without changing their motion or timestamps."""
import hashlib
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent


def lines(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def inspect(name, source):
    folder = ROOT / ("lab-front-" + name)
    config = json.loads((folder / "config.json").read_text())
    capture = json.loads((folder / "capture.json").read_text())
    frames = lines(folder / "frames.ndjson")
    assert config["clock"] == "Bevy realtime" and not config["offline"]
    assert not config["uncapped_scheduler"] and not config["legacy_secondary_bones"]
    assert (folder / "packets.ndjson").read_bytes() == source
    stamps = np.array([r["pts_ms"] for r in frames])
    wall = np.array([r["wall_elapsed_ms"] for r in frames])
    gaps = np.diff(stamps)
    assert len(frames) == capture["frames"] and np.all(gaps > 0)
    assert capture["sent_packets"] == len(source.splitlines())
    assert stamps[-1] == capture["last_pts_ms"] == json.loads(source.splitlines()[-1])["pts_ms"]
    assert all(r["image"] is None or (folder / r["image"]).exists() for r in frames)
    trace = lines(folder / "tissue.ndjson") if config["tissue"] else None
    if trace:
        assert [r["pts_ms"] for r in trace] == stamps.tolist()
        assert max(r["skin_error_m"] for r in trace) < 0.0002
        assert min(region["metrics"]["minimum_volume_ratio"] for r in trace for region in r["regions"]) > 0
    drift = (wall - wall[0]) - (stamps - stamps[0])
    result = {
        "directory": folder.name, "capture": capture, "fps_target": config["fps"],
        "tissue": config["tissue"], "first_source_ms": int(stamps[0]),
        "source_span_seconds": float((stamps[-1] - stamps[0]) / 1000),
        "playback_wall_seconds": float((wall[-1] - wall[0]) / 1000),
        "presentations_per_source_second": float((len(stamps) - 1) * 1000 / (stamps[-1] - stamps[0])),
        "source_gap_ms_p50_p95_p99_max": np.quantile(gaps, [.5, .95, .99, 1]).tolist(),
        "consumer_gap_ms_p50_p95_p99_max": np.quantile(np.diff(wall), [.5, .95, .99, 1]).tolist(),
        "source_gaps_above_50ms": int(np.sum(gaps > 50)),
        "relative_clock_drift_ms_min_max_end": [float(drift.min()), float(drift.max()), float(drift[-1])],
        "maximum_skin_error_mm": max(r["skin_error_m"] for r in trace) * 1000 if trace else None,
        "peak_surface_displacement_mm": max(r["maximum_surface_displacement_m"] for r in trace) * 1000 if trace else None,
        "minimum_element_volume_ratio": min(region["metrics"]["minimum_volume_ratio"] for r in trace for region in r["regions"]) if trace else None,
        "maximum_active_contacts": max(r["integration"]["active_contact_samples"] for r in trace) if trace else None,
    }
    return result, stamps, gaps, trace


def main():
    source = (ROOT / "lab-jump-01/packets.ndjson").read_bytes()
    responses = lines(ROOT / "lab-jump-01/responses.ndjson")
    assert [json.loads(row) for row in source.splitlines()] == [frame for r in responses for frame in r["frames"]]
    assert all(r["audit"] == dict(policy="model-owned-v1", future_nonzero=0,
                                  future_channels=[], exceptions=[]) for r in responses)
    launches = json.loads((HERE / "live-clock-launches.json").read_text())
    assert len(launches) == 3 and all(run["exit_code"] == 0 for run in launches)
    names = [run["name"] for run in launches]
    captures = {name: inspect(name, source) for name in names}
    video = inspect("live-video-01", source)
    fig, axes = plt.subplots(3, 1, figsize=(11, 9), sharex=True, layout="constrained")
    colors = {"live-on-01": "#e45756", "live-off-01": "#4f83cc", "live-on-02": "#17a398"}
    for name, (report, stamps, gaps, trace) in captures.items():
        label = name.replace("live-", "tissue ")
        axes[0].plot(stamps[1:] / 1000, gaps, label=label, color=colors[name], alpha=.75)
        if trace:
            axes[1].plot(stamps / 1000, [r["integration"]["elasticity_ms"] for r in trace], label=label, color=colors[name])
            axes[2].plot(stamps / 1000, [r["maximum_surface_displacement_m"] * 1000 for r in trace], label=label, color=colors[name])
    axes[0].axhline(1000 / 30, color="gray", linestyle="--", linewidth=1, label="30 FPS interval")
    axes[0].set_ylabel("Source frame gap (ms)")
    axes[1].set_ylabel("Tissue solve / frame (ms)")
    axes[2].set_ylabel("Skin displacement (mm)")
    axes[2].set_xlabel("Native source time (s)")
    for axis in axes:
        axis.grid(alpha=.2)
        axis.legend(loc="upper right")
    fig.suptitle("Live Bevy clock | unchanged native jump | 30 FPS target / 120 Hz tissue\nLocal prerecorded input; timing runs exclude PNG saving")
    fig.savefig(HERE / "live-clock-timing.png", dpi=150)
    plt.close(fig)
    report = {
        "source_sha256": hashlib.sha256(source).hexdigest(),
        "runs": {name: item[0] for name, item in captures.items()}, "visual_capture": video[0],
        "source_packets_unchanged_and_semantic_only": True,
        "body_and_tissue_stamps_correlated_in_gpu_readback": True,
        "limitations": [
            "Local renderer with prerecorded native motion and 500 ms bounded prefetch, not live model/network/client latency.",
            "Consumer timestamps measure delivery to this capture process, not browser presentation or absolute input-to-image latency.",
            "Clock drift is relative to each run's first captured frame; startup latency is separate.",
            "Variable first observations and sample times prevent byte-identical mechanical comparisons across live runs.",
            "Source frame gaps are reported, not hidden by resampling or pose repair.",
            "The image-saving run has separate timings and is not pooled into the trace-only performance measurements.",
            "All runs have zero active contact; this is not anatomical or compression acceptance.",
        ],
    }
    (HERE / "live-clock-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
