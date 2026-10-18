"""Build a source-timed studio comparison and read-only response measurements."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

FFMPEG = "/usr/bin/ffmpeg"
FFPROBE = "/usr/bin/ffprobe"


def lines(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def encode(folder, output):
    rows = lines(folder / "frames.ndjson")
    if not rows or any(not isinstance(row.get("image"), str) for row in rows):
        raise ValueError("Capture did not save images; trace-only runs cannot be encoded")
    entries = ["ffconcat version 1.0"]
    for i, row in enumerate(rows):
        dt = rows[i + 1]["pts_ms"] - row["pts_ms"] if i + 1 < len(rows) else 17
        entries += [f"file '{(folder / row['image']).resolve()}'", "option framerate 1000",
                    f"duration {dt / 1000:.3f}"]
    manifest = output.with_suffix(".ffconcat")
    manifest.write_text("\n".join(entries) + "\n")
    subprocess.run([FFMPEG, "-v", "error", "-n", "-f", "concat", "-safe", "0",
                    "-i", str(manifest), "-c:v", "libx264", "-crf", "18", "-preset", "fast",
                    "-pix_fmt", "yuv420p", "-fps_mode", "vfr", "-enc_time_base", "1:1000",
                    "-video_track_timescale", "1000", "-bf", "0", "-movflags", "+faststart", str(output)], check=True)
    result = subprocess.run([FFPROBE, "-v", "error", "-show_frames", "-select_streams", "v:0",
                             "-show_entries", "frame=best_effort_timestamp_time", "-of", "json", str(output)],
                            check=True, capture_output=True, text=True)
    stamps = [round(float(f["best_effort_timestamp_time"]) * 1000) for f in json.loads(result.stdout)["frames"]]
    expected = [r["pts_ms"] - rows[0]["pts_ms"] for r in rows]
    if stamps != expected:
        raise ValueError("Encoded video did not preserve source timestamps")
    return rows


def comparison(output):
    names = ["front-off", "front-on", "side-off", "side-on"]
    labels = ["FRONT | undeformed", "FRONT | tissue simulation", "SIDE | undeformed", "SIDE | tissue simulation"]
    command = [FFMPEG, "-v", "error", "-n"]
    for name in names:
        command += ["-i", str(output / f"{name}.mp4")]
    filters = [f"[{i}:v]crop=600:900:60:0,pad=600:940:0:40:color=0x18202c,"
               f"drawtext=fontcolor=white:fontsize=20:x=14:y=10:text='{label}'[v{i}]"
               for i, label in enumerate(labels)]
    filters += ["[v0][v1][v2][v3]xstack=inputs=4:layout=0_0|600_0|0_940|600_940[v]"]
    command += ["-filter_complex", ";".join(filters), "-map", "[v]", "-c:v", "libx264",
                "-crf", "18", "-pix_fmt", "yuv420p", "-fps_mode", "vfr", "-enc_time_base", "1:1000",
                "-video_track_timescale", "1000", "-bf", "0", "-movflags", "+faststart", str(output / "comparison.mp4")]
    subprocess.run(command, check=True)


def measure(motion, trace, output):
    times = np.array([p["pts_ms"] for p in motion]) / 1000
    # Source toe heights observe flight; no placement correction or pose edits.
    toes = np.array([[p["source_joints"][i][1] for i in (22, 26)] for p in motion])
    baseline = np.median(toes[:4], axis=0)
    airborne = np.all(toes > baseline + .03, axis=1)
    first = int(np.flatnonzero(airborne)[0])
    landing = first + int(np.flatnonzero(~airborne[first:])[0])
    t = np.array([row["pts_ms"] for row in trace]) / 1000
    offsets = np.array([[r["free_node_centroid_offset_m"] for r in row["regions"]] for row in trace])
    maximum = np.array([row["maximum_surface_displacement_m"] for row in trace])
    anchor = np.array([row["regions"][0]["anchor_world_m"] for row in trace])
    metrics = {"first_source_flight_s": [float(times[first]), float(times[landing])],
               "flight_definition": "Both source toes exceed their first-200ms median by 30mm; 20Hz observation, not sole contact.",
               "peak_surface_displacement_mm": float(maximum.max() * 1000),
               "peak_surface_time_s": float(t[maximum.argmax()]),
               "settling_definition": "Free-node centroid offset stays within 2mm of fitted initial shape for 0.5s after first source landing, before 3.2s.",
               "centroid_settling_s_after_landing": None,
               "minimum_volume_ratio": min(r["metrics"]["minimum_volume_ratio"] for row in trace for r in row["regions"]),
               "maximum_skin_error_mm": max(row["skin_error_m"] for row in trace) * 1000,
               "maximum_active_contacts": max(row["integration"]["active_contact_samples"] for row in trace)}
    below = np.all(np.linalg.norm(offsets, axis=2) < .002, axis=1)
    for i in np.flatnonzero((t >= times[landing]) & (t <= 2.7)):
        window = (t >= t[i]) & (t <= t[i] + .5)
        if below[window].all() and t[window][-1] - t[i] >= .48:
            metrics["centroid_settling_s_after_landing"] = float(t[i] - times[landing])
            break
    fig, axes = plt.subplots(3, 1, figsize=(11, 8), sharex=True, layout="constrained")
    axes[0].plot(t, anchor[:, 1], color="#4f83cc", label="Chest anchor world height")
    axes[0].set_ylabel("Chest height (m)")
    for side, color in enumerate(["#e45756", "#17a398"]):
        axes[1].plot(t, offsets[:, side, 1] * 1000, color=color, label=f"{'Left' if side == 0 else 'Right'} free-node centroid")
    axes[1].axhspan(-2, 2, color="gray", alpha=.12)
    axes[1].set_ylabel("Chest-local Y offset (mm)")
    axes[2].plot(t, maximum * 1000, color="#8e5bb7", label="Maximum rendered surface displacement")
    axes[2].set_ylabel("Displacement (mm)")
    axes[2].set_xlabel("Native source time (s)")
    for axis in axes:
        axis.axvspan(times[first], times[landing], color="#4f83cc", alpha=.12)
        axis.axvline(times[landing], color="gray", linestyle="--", linewidth=1)
        axis.grid(alpha=.2)
        axis.legend(loc="upper right")
    fig.suptitle("Base avatar jump | identical recorded motion | shaded first source flight\nPrototype parameters; centroid response is not anatomical validation")
    fig.savefig(output / "response.png", dpi=150)
    plt.close(fig)
    return metrics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--tissue-suffix", default="on-01", help="Capture suffix after lab-front- / lab-side-")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    motion_dir = args.root / "lab-jump-01"
    packets = motion_dir / "packets.ndjson"
    motion = lines(packets)
    responses = lines(motion_dir / "responses.ndjson")
    if motion != [f for r in responses for f in r["frames"]]:
        raise ValueError("Motion differs from saved service output")
    if any(r["audit"] != dict(policy="model-owned-v1", future_nonzero=0, future_channels=[], exceptions=[]) for r in responses):
        raise ValueError("Source was not semantic-only")
    expected = None
    for name in ["front-off", "front-on", "side-off", "side-on"]:
        view, state = name.split("-")
        suffix = args.tissue_suffix if state == "on" else "off-01"
        folder = args.root / f"lab-{view}-{suffix}"
        if (folder / "packets.ndjson").read_bytes() != packets.read_bytes():
            raise ValueError("Capture source bytes changed")
        rows = encode(folder, args.output / f"{name}.mp4")
        stamps = [r["pts_ms"] for r in rows]
        if stamps[0] != motion[0]["pts_ms"] or stamps[-1] != motion[-1]["pts_ms"] or max(np.diff(stamps)) > 17:
            raise ValueError("Incomplete or undersampled capture")
        if expected is not None and stamps != expected:
            raise ValueError("Comparison source clocks do not match")
        expected = stamps
    front = lines(args.root / f"lab-front-{args.tissue_suffix}/tissue.ndjson")
    side = lines(args.root / f"lab-side-{args.tissue_suffix}/tissue.ndjson")
    for a, b in zip(front, side, strict=True):
        if a["pts_ms"] != b["pts_ms"]:
            raise ValueError("Trace clocks differ")
        if a["regions"] != b["regions"]:
            raise ValueError("Camera changed the tissue trajectory")
    metrics = measure(motion, front, args.output)
    metrics.update(frames_per_view=len(expected), source_start_ms=expected[0], source_end_ms=expected[-1],
                   tissue_capture_suffix=args.tissue_suffix,
                   maximum_frame_gap_ms=int(max(np.diff(expected))), camera_independent_trajectory=True,
                   source_sha256=hashlib.sha256(packets.read_bytes()).hexdigest(),
                   limitations=["Offline render, not real-time performance", "Repeated model-generated jumps after the first landing",
                                "No anatomical material calibration", "Initial settling and continued body movement affect response",
                                "Vertex-face contact only; this sequence is not a compression acceptance test"])
    (args.output / "measurements.json").write_text(json.dumps(metrics, indent=2) + "\n")
    comparison(args.output)
    (args.output / "index.html").write_text('''<!doctype html><meta charset="utf-8"><title>Base avatar tissue lab</title>
<style>body{max-width:1100px;margin:36px auto;padding:0 24px;background:#111923;color:#e8edf4;font:16px system-ui}button{padding:10px 18px;margin-right:8px}video{display:block;max-width:100%;max-height:82vh;margin:20px auto}img{width:100%;background:white}a{color:#9dceff}</style>
<h1>Base avatar · jump response</h1><p>Identical ARDY motion, fixed studio cameras. Left: undeformed mesh. Right: volumetric tissue. No condo or clothing.</p>
<button onclick="v.playbackRate=1">Normal speed</button><button onclick="v.playbackRate=.25">¼ speed</button><button onclick="v.currentTime=0;v.play()">Replay</button>
<video id="v" controls loop playsinline src="comparison.mp4"></video>
<p>Offline source-time capture at 60 frames per simulated second. This is visual response evidence, not a real-time performance result. The generated recording repeats the jump later; all samples are preserved.</p>
<img src="response.png" alt="Chest height and chest-relative tissue displacement over source time">
<p><a href="measurements.json">Measurements and limitations</a> · <a href="front-on.mp4">Full front view</a> · <a href="side-on.mp4">Full side view</a></p>''')
    print(json.dumps(metrics, indent=2))


if __name__ == "__main__":
    main()
