"""Read-only comparison of the declared baseline and nonlinear material runs."""
import json
from pathlib import Path
import subprocess
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

root = Path(__file__).resolve().parent.parent
output = root / "lab-review-hardening-8"
def trace(name):
    return [json.loads(line) for line in (root / name / "tissue.ndjson").read_text().splitlines()]
old = trace("lab-front-on-01")
new = trace("lab-front-hardening-8")
assert [r["pts_ms"] for r in old] == [r["pts_ms"] for r in new]
fig, axes = plt.subplots(2, 1, figsize=(11, 6), sharex=True, layout="constrained")
report = {}
for rows, label, color in [(old, "Original material", "#d55e00"), (new, "Strain stiffening", "#0072b2")]:
    times = [r["pts_ms"] / 1000 for r in rows]
    maximum = [r["maximum_surface_displacement_m"] * 1000 for r in rows]
    axes[0].plot(times, maximum, label=label, color=color)
    axes[1].plot(times, [r["regions"][0]["free_node_centroid_offset_m"][1] * 1000 for r in rows], label=label, color=color)
    first_landing = [r for r in rows if 1300 <= r["pts_ms"] <= 1900]
    report[label] = {"whole_sequence_peak_mm": max(maximum),
                     "first_landing_peak_mm": max(r["maximum_surface_displacement_m"] for r in first_landing) * 1000}
for axis in axes:
    axis.grid(alpha=.2)
    axis.axvspan(1.3, 1.9, color="gray", alpha=.1)
    axis.legend()
axes[0].set_ylabel("Maximum surface displacement (mm)")
axes[1].set_ylabel("Left centroid local Y offset (mm)")
axes[1].set_xlabel("Native source time (s)")
fig.suptitle("Identical generated jump sequence | material comparison\nShaded first landing window; both remain uncalibrated prototypes")
fig.savefig(output / "material-response.png", dpi=150)
(output / "material-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
command = ["/usr/bin/ffmpeg", "-v", "error", "-n"]
for folder, name in [("lab-review-01", "front-on"), ("lab-review-hardening-8", "front-on"),
                     ("lab-review-01", "side-on"), ("lab-review-hardening-8", "side-on")]:
    command += ["-i", str(root / folder / f"{name}.mp4")]
labels = ["FRONT | original material", "FRONT | strain stiffening", "SIDE | original material", "SIDE | strain stiffening"]
filters = [f"[{i}:v]crop=600:900:60:0,pad=600:940:0:40:color=0x18202c,drawtext=fontcolor=white:fontsize=19:x=12:y=10:text='{label}'[v{i}]" for i, label in enumerate(labels)]
filters += ["[v0][v1][v2][v3]xstack=inputs=4:layout=0_0|600_0|0_940|600_940[v]"]
command += ["-filter_complex", ";".join(filters), "-map", "[v]", "-c:v", "libx264", "-crf", "18",
            "-pix_fmt", "yuv420p", "-fps_mode", "vfr", "-enc_time_base", "1:1000",
            "-video_track_timescale", "1000", "-bf", "0", "-movflags", "+faststart", str(output / "material-comparison.mp4")]
subprocess.run(command, check=True)
result = subprocess.run(["/usr/bin/ffprobe", "-v", "error", "-show_frames", "-select_streams", "v:0",
                         "-show_entries", "frame=best_effort_timestamp_time", "-of", "json", str(output / "material-comparison.mp4")],
                        check=True, capture_output=True, text=True)
stamps = [round(float(f["best_effort_timestamp_time"]) * 1000) for f in json.loads(result.stdout)["frames"]]
assert stamps == [r["pts_ms"] - old[0]["pts_ms"] for r in old]
page = output / "index.html"
html = page.read_text().replace("Left: undeformed mesh. Right: volumetric tissue.", "Left: original material. Right: strain stiffening.")
html = html.replace('src="comparison.mp4"', 'src="material-comparison.mp4"')
html = html.replace('src="response.png"', 'src="material-response.png"')
html = html.replace('<p><a href="measurements.json">', '<p><a href="comparison.mp4">New material versus undeformed mesh</a> · <a href="measurements.json">')
page.write_text(html)
print(json.dumps(report, indent=2))
