"""Reproduce the contact fixture report and scientific plot from saved traces."""
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

folder = Path(__file__).resolve().parent
root = folder.parents[2]
asset = json.loads((root / "assets/tissue/beautiful_assistant.base-fitted-prototype.json").read_text())


def read(name):
    rows = [json.loads(line) for line in (folder / name).read_text().splitlines()]
    if len(rows) != 720:
        raise ValueError(f"Incomplete numerical run: {name}")
    return rows


def volume(points, region):
    tetrahedra = np.asarray(points)[region["tetrahedra"]]
    edges = (tetrahedra[:, 1:] - tetrahedra[:, :1]).transpose(0, 2, 1)
    return float(np.linalg.det(edges).sum() / 6)


cases = {
    "1e8 Pa/m": read("base-8passes-120hz.jsonl"),
    "1e9 Pa/m": read("base-8passes-120hz-k1e9.jsonl"),
    "1e10 Pa/m": read("base-8passes-120hz-k1e10.jsonl"),
}
reference = read("base-32passes-120hz-k1e10.jsonl")
runtime = read("base-2passes-120hz-k1e10.jsonl")
report = {"scope": "Numerical planar indentation; not live avatar contact acceptance", "regions": []}
fig, axes = plt.subplots(2, 2, figsize=(12, 7), sharex=True, sharey="row", constrained_layout=True)
for column, region in enumerate(asset["regions"]):
    name = region["name"]
    for label, rows in cases.items():
        selected = [r for r in rows if r["region"] == name]
        time = [r["seconds"] for r in selected]
        axes[0, column].plot(time, [r["contact"]["normal_force_n"] for r in selected], label=label)
        axes[1, column].plot(time, [1000*r["contact"]["maximum_penetration_m"] for r in selected])
    axes[0, column].set_title(name.replace("_", " ").capitalize())
    axes[1, column].axhline(0.1, color="#666666", linestyle="--", linewidth=1, label="0.1 mm target")
    axes[1, column].set_xlabel("Source simulation time (s)")
    for ax in axes[:, column]:
        ax.grid(alpha=0.2)
        ax.spines[["top", "right"]].set_visible(False)
    selected = [r for r in cases["1e10 Pa/m"] if r["region"] == name]
    high = [r for r in reference if r["region"] == name]
    low = [r for r in runtime if r["region"] == name]
    peak = next(r for r in selected if r["seconds"] == 1.5)
    ref_peak = next(r for r in high if r["seconds"] == 1.5)
    low_peak = next(r for r in low if r["seconds"] == 1.5)
    def maximum_distance(a, b):
        return float(np.linalg.norm(np.asarray(a) - np.asarray(b), axis=1).max())
    report["regions"].append({
        "region": name,
        "maximum_penetration_m": max(r["contact"]["maximum_penetration_m"] for r in selected),
        "peak_normal_force_n": max(r["contact"]["normal_force_n"] for r in selected),
        "peak_volume_ratio_vs_loaded": volume(peak["node_positions"], region) / volume(region["initial_positions"], region),
        "8_vs_32_peak_node_difference_m": maximum_distance(peak["node_positions"], ref_peak["node_positions"]),
        "2_vs_32_peak_node_difference_m": maximum_distance(low_peak["node_positions"], ref_peak["node_positions"]),
        "8_vs_32_max_force_difference_n": max(abs(a["contact"]["normal_force_n"]-b["contact"]["normal_force_n"]) for a,b in zip(selected, high)),
        "release_shape_error_vs_loaded_m": maximum_distance(selected[-1]["node_positions"], region["initial_positions"]),
        "release_normal_force_n": selected[-1]["contact"]["normal_force_n"],
        "8_pass_wall_seconds": selected[-1]["wall_seconds"],
    })
axes[0, 0].set_ylabel("Total compressive force (N)")
axes[1, 0].set_ylabel("Maximum skin overlap (mm)")
axes[0, 0].legend(title="Penalty pressure stiffness", fontsize=9)
axes[1, 0].legend(fontsize=9)
fig.suptitle("Base-avatar tissue: numerical compression and release\n120 Hz, 8 nonlinear passes; no skeleton animation", fontsize=15)
fig.savefig(folder / "compression-response.png", dpi=150)
plt.close(fig)
(folder / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
