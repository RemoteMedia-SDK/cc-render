#!/usr/bin/env python3
"""Bind the base-avatar tissue candidate to its torso and arm skin primitives.

This does not modify the GLB, fitted cages, skin weights or vertex embeddings.
Records geometry-affecting morph channels which the current prototype rejects.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from build_tissue_asset import accessor, read_glb


def build(fitted_path, glb_path):
    asset = json.loads(fitted_path.read_text())
    fingerprint = hashlib.sha256(glb_path.read_bytes()).hexdigest()
    if asset["source_sha256"] != [fingerprint]:
        raise ValueError("Expected a single calibrated base-avatar fingerprint")
    document, binary = read_glb(glb_path)
    report = []
    inactive = set()
    arm_hashes = []
    for mesh in document["meshes"]:
        if mesh["name"] != "CC_Base_Body":
            continue
        for primitive in mesh["primitives"]:
            material = document["materials"][primitive["material"]]["name"]
            if material not in ("Std_Skin_Body", "Std_Skin_Arm"):
                continue
            positions = accessor(document, binary, primitive["attributes"]["POSITION"])
            sha = hashlib.sha256(positions.astype("<f4").tobytes()).hexdigest()
            if material == "Std_Skin_Arm":
                arm_hashes.append(sha)
            elif sha not in {s["positions_sha256"] for s in asset["surfaces"]}:
                raise ValueError("Fitted body embedding does not match base torso")
            morphs = []
            for index, target in enumerate(primitive.get("targets", [])):
                delta = accessor(document, binary, target["POSITION"])
                maximum = float(np.linalg.norm(delta, axis=1).max())
                if maximum > 1e-7:
                    inactive.add(index)
                    morphs.append({"index": index, "name": mesh["extras"]["targetNames"][index],
                                   "maximum_delta_m": maximum})
            report.append({"material": material, "vertices": len(positions),
                           "positions_sha256": sha, "nonzero_geometry_morphs": morphs})
    if len(arm_hashes) != 1 or len(report) != 2:
        raise ValueError("Missing or ambiguous base torso/arm primitives")
    asset["contact_surfaces"] = arm_hashes
    asset["contact_inactive_morph_targets"] = sorted(inactive)
    return asset, {"source_sha256": fingerprint,
                   "fitted_asset_sha256": hashlib.sha256(fitted_path.read_bytes()).hexdigest(),
                   "surfaces": report,
                   "scope": "Torso fixed skin and arm/hand skin; no head, legs, nails or mutual breast contact"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fitted_asset", type=Path)
    parser.add_argument("base_glb", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    asset, report = build(args.fitted_asset, args.base_glb)
    encoded = json.dumps(asset, separators=(",", ":")) + "\n"
    report["output_sha256"] = hashlib.sha256(encoded.encode()).hexdigest()
    with args.output.open("x") as handle:
        handle.write(encoded)
    with args.output.with_suffix(".report.json").open("x") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")


if __name__ == "__main__":
    main()
