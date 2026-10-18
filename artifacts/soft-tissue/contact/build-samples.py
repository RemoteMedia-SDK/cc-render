"""Reproduce base-skin quadrature for the numerical contact probe."""
import hashlib
import json
from pathlib import Path
import sys

import numpy as np

root = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(root / "scripts/avatars"))
from build_tissue_asset import surfaces, anchor_space

source = root.parent / "remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb"
asset_path = root / "assets/tissue/beautiful_assistant.base-fitted-prototype.json"
asset = json.loads(asset_path.read_text())
body = next(surfaces(source))
points = anchor_space(body["positions"], body["anchor_inverse"])
triangles = body["triangles"]
area = np.linalg.norm(np.cross(points[triangles[:, 1]] - points[triangles[:, 0]],
                               points[triangles[:, 2]] - points[triangles[:, 0]]), axis=1) / 2
weights = np.zeros(len(points))
for corner in range(3):
    np.add.at(weights, triangles[:, corner], area / 3)
samples = [{"binding": binding, "rest_point": points[binding["vertex"]].tolist(),
            "area_m2": float(weights[binding["vertex"]])} for binding in asset["surfaces"][0]["vertices"]]
report = {
    "source_glb_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
    "tissue_asset_sha256": hashlib.sha256(asset_path.read_bytes()).hexdigest(),
    "fixture": "numerical planar indentation of the unchanged base skin embedding; not avatar motion acceptance",
    "samples": samples,
}
with Path(sys.argv[1]).open("x") as output:
    json.dump(report, output, separators=(",", ":"))
    output.write("\n")
