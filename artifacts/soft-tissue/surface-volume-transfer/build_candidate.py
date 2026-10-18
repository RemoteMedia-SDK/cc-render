"""Change surface transfer only; preserve the fitted physical volume and motion."""
import copy
import hashlib
import json
import sys
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / "scripts/avatars"))
from build_tissue_asset import embed, surfaces


def main():
    source = ROOT / "assets/tissue/beautiful_assistant.base-viscous-prototype.json"
    target = ROOT / "assets/tissue/beautiful_assistant.base-geometric-transfer-prototype.json"
    assert not target.exists()
    avatar = ROOT / "../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb"
    body = next(s for s in surfaces(avatar) if s["name"] == "CC_Base_Body")
    original = json.loads(source.read_text())
    candidate = copy.deepcopy(original)
    assert len(candidate["surfaces"]) == 1
    binding = candidate["surfaces"][0]
    assert binding["positions_sha256"] == body["positions_sha256"]
    bindings = [embed(body, region, side) for side, region in enumerate(candidate["regions"])]
    binding["vertices"] = [b for group, _ in bindings for b in group]
    assert candidate["regions"] == original["regions"]
    target.write_text(json.dumps(candidate, separators=(",", ":")) + "\n")
    points = body["positions"].astype(float)
    triangles = body["triangles"]
    reports = []
    fig, axes = plt.subplots(1, 2, figsize=(12, 6), layout="constrained")
    for axis, asset, label in zip(axes, [original, candidate], ["Previous bone-weight attenuation", "Geometric 20 mm boundary transition"]):
        weights = np.zeros(len(points))
        for b in asset["surfaces"][0]["vertices"]:
            weights[b["vertex"]] += b["influence"]
        # Keep the same old patch area in both comparisons, including its rim.
        active = np.logical_or.reduce([w > 0.001 for w in body["weights"]])
        faces = triangles[np.any(active[triangles], axis=1)]
        area = np.linalg.norm(np.cross(points[faces[:, 1]] - points[faces[:, 0]],
                                      points[faces[:, 2]] - points[faces[:, 0]]), axis=1) / 2
        average = weights[faces].mean(axis=1)
        reports.append({"label": label, "bindings": len(asset["surfaces"][0]["vertices"]),
                        "patch_area_cm2": float(area.sum() * 1e4),
                        "area_weighted_influence": float(np.dot(area, average) / area.sum()),
                        "area_fraction_below_quarter": float(area[average < .25].sum() / area.sum()),
                        "area_fraction_above_0_9": float(area[average > .9].sum() / area.sum())})
        plot = axis.tripcolor(points[:, 0] * 100, points[:, 2] * 100, faces,
                              facecolors=average, vmin=0, vmax=1, cmap="viridis", edgecolors="none")
        axis.set(xlim=(-13, 13), ylim=(122, 137), aspect="equal", title=label,
                 xlabel="Source X (cm)", ylabel="Source Z (cm)")
    fig.colorbar(plot, ax=axes, label="Fraction of simulated displacement transferred to skin", shrink=.65)
    fig.suptitle("Surface-transfer audit | identical cage, coefficients and source skin\nGeometry transition is a prototype boundary blend, not anatomical calibration")
    fig.savefig(HERE / "influence-comparison.png", dpi=140)
    plt.close(fig)
    report = {"source_asset_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "candidate_asset_sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
              "builder_sha256": hashlib.sha256((ROOT / "scripts/avatars/build_tissue_asset.py").read_bytes()).hexdigest(),
              "physical_regions_exactly_unchanged": True, "transition_width_m": .02,
              "embedding_maximum_residual_m": [distance for _, distance in bindings],
              "surface_comparison": reports,
              "limitation": "Legacy weights still identify the patch; magnitude is replaced by geometry-based boundary transfer. The blend approximates coupling to kinematic chest skin."}
    (HERE / "asset-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
