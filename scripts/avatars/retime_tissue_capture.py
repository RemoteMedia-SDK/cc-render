#!/usr/bin/env python3
"""Encode a diagnostic tissue capture using its GPU-correlated source times.

Keeps the original view.mp4 and frame-times.ndjson. Removes unstamped warmup
and repeated source times, without interpolating pixels or changing motion.
The resulting VFR file is checked frame-by-frame with ffprobe. Input video is
already compressed, so the output is a second-generation visual review copy.
"""
import argparse
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

# Match condo_capture's codec-enabled system FFmpeg, not a model environment's
# potentially minimal conda build.
FFMPEG = "/usr/bin/ffmpeg"
FFPROBE = "/usr/bin/ffprobe"


def source_frames(rows):
    selected = []
    for index, row in enumerate(rows):
        if row["frame"] != index:
            raise ValueError("Capture frame indices must be contiguous")
        pts = row["tissue_pts_ms"]
        if pts is None:
            if selected:
                raise ValueError("Tissue timestamp disappeared during playback")
            continue
        if type(pts) is not int or pts < 0:
            raise ValueError("Invalid tissue source timestamp")
        if selected and pts < selected[-1][1]:
            raise ValueError("Tissue source clock moved backwards")
        if not selected or pts > selected[-1][1]:
            selected.append((index, pts))
    if len(selected) < 2:
        raise ValueError("Capture has no advancing tissue motion")
    return selected


def probe_timestamps(path):
    result = subprocess.run([
        FFPROBE, "-v", "error", "-select_streams", "v:0", "-show_frames",
        "-show_entries", "frame=best_effort_timestamp_time", "-of", "json", str(path),
    ], check=True, capture_output=True, text=True)
    return [Decimal(frame["best_effort_timestamp_time"]) * 1000
            for frame in json.loads(result.stdout)["frames"]]


def encode(video, rows, selected, output):
    with tempfile.TemporaryDirectory(prefix="cc-tissue-retime-") as temporary:
        folder = Path(temporary)
        subprocess.run([
            FFMPEG, "-v", "error", "-i", str(video), "-map", "0:v:0",
            "-fps_mode", "passthrough", "-start_number", "0", str(folder / "%06d.png"),
        ], check=True)
        if len(list(folder.glob("*.png"))) != len(rows):
            raise ValueError("Encoded input frame count differs from timing metadata")
        entries = ["ffconcat version 1.0"]
        for ordinal, (index, pts) in enumerate(selected):
            duration = selected[ordinal + 1][1] - pts if ordinal + 1 < len(selected) else 1
            entries.extend([f"file '{index:06d}.png'", "option framerate 1000",
                            f"duration {duration / 1000:.3f}"])
        manifest = folder / "frames.ffconcat"
        manifest.write_text("\n".join(entries) + "\n")
        subprocess.run([
            FFMPEG, "-v", "error", "-n", "-f", "concat", "-safe", "0",
            "-i", str(manifest), "-an", "-c:v", "libx264", "-preset", "fast",
            "-crf", "18", "-pix_fmt", "yuv420p", "-fps_mode", "vfr",
            "-enc_time_base", "1:1000", "-video_track_timescale", "1000",
            "-bf", "0", "-movflags", "+faststart", str(output),
        ], check=True)
    expected = [pts - selected[0][1] for _, pts in selected]
    observed = probe_timestamps(output)
    if observed != expected:
        raise ValueError("Output presentation timestamps do not match source times")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    capture = args.capture.resolve()
    output = capture / "source-time.mp4"
    report_path = capture / "source-time.json"
    if output.exists() or report_path.exists():
        raise ValueError("Preserve previous timing evidence; use a new capture")
    timing_path = capture / "frame-times.ndjson"
    rows = [json.loads(line) for line in timing_path.read_text().splitlines()]
    selected = source_frames(rows)
    video = capture / "view.mp4"
    encode(video, rows, selected, output)
    gaps = [b[1] - a[1] for a, b in zip(selected, selected[1:])]
    report = {
        "input_video_sha256": hashlib.sha256(video.read_bytes()).hexdigest(),
        "timing_metadata_sha256": hashlib.sha256(timing_path.read_bytes()).hexdigest(),
        "output_video_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
        "input_frames": len(rows), "output_frames": len(selected),
        "first_source_pts_ms": selected[0][1], "last_source_pts_ms": selected[-1][1],
        "source_span_ms": selected[-1][1] - selected[0][1],
        "maximum_source_gap_ms": max(gaps), "final_frame_duration_ms": 1,
        "every_output_timestamp_matches_source": True,
        "frame_mapping": [{"output_frame": i, "capture_frame": index, "source_pts_ms": pts}
                          for i, (index, pts) in enumerate(selected)],
        "limitations": "Timing is preserved; sampling gaps remain visible. No pixel interpolation. Second-generation encoding.",
    }
    with report_path.open("x") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")
    print(json.dumps({key: value for key, value in report.items() if key != "frame_mapping"}, indent=2))


if __name__ == "__main__":
    main()
