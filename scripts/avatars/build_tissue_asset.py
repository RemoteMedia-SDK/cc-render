#!/usr/bin/env python3
"""Build a small tetrahedral breast cage from beautiful_assistant's actual skin.

Offline authoring only. Never changes the GLB, skeleton or source skin weights.
The planar layered cage is an approximation to the artist-authored tissue volume;
its material values require equilibrium fitting and rendered motion acceptance.
Requires NumPy, SciPy, trimesh and rtree for the body-surface transfer queries.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from scipy.spatial import ConvexHull, Delaunay
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "condo"))
from outfit_package import accessor, read_glb


def matrix_accessor(document, binary, index):
    spec = document["accessors"][index]
    view = document["bufferViews"][spec["bufferView"]]
    if spec["type"] != "MAT4" or spec["componentType"] != 5126 or "byteStride" in view:
        raise ValueError("Expected contiguous float inverse-bind matrices")
    offset = view.get("byteOffset", 0) + spec.get("byteOffset", 0)
    return np.frombuffer(binary, "<f4", count=16 * spec["count"], offset=offset).reshape(-1, 4, 4).transpose(0, 2, 1).astype(float)


def surfaces(path):
    document, binary = read_glb(path)
    for node in document["nodes"]:
        if "mesh" not in node or "skin" not in node:
            continue
        skin = document["skins"][node["skin"]]
        names = [document["nodes"][joint].get("name", "") for joint in skin["joints"]]
        if "CC_Base_Spine02" not in names:
            continue
        anchor_inverse = matrix_accessor(document, binary, skin["inverseBindMatrices"])[names.index("CC_Base_Spine02")]
        mesh = document["meshes"][node["mesh"]]
        for primitive in mesh["primitives"]:
            attributes = primitive["attributes"]
            if "JOINTS_0" not in attributes:
                continue
            joints = accessor(document, binary, attributes["JOINTS_0"])
            weights = accessor(document, binary, attributes["WEIGHTS_0"]).astype(float)
            weights /= weights.sum(axis=1, keepdims=True)
            breast_weights = []
            for side in ("L", "R"):
                name = f"CC_Base_{side}_Breast"
                breast_weights.append(np.where(joints == names.index(name), weights, 0).sum(axis=1) if name in names else np.zeros(len(weights)))
            if not any(np.any(w > 0.001) for w in breast_weights):
                continue
            positions = accessor(document, binary, attributes["POSITION"])
            position_morphed = np.zeros(len(positions), dtype=bool)
            for target in primitive.get("targets", []):
                if "POSITION" in target:
                    position_morphed |= np.any(accessor(document, binary, target["POSITION"]) != 0, axis=1)
            yield {"name": mesh["name"], "positions": positions, "weights": breast_weights,
                   "position_morphed": position_morphed,
                   "triangles": accessor(document, binary, primitive["indices"]).reshape(-1, 3).astype(int),
                   "anchor_inverse": anchor_inverse,
                   "positions_sha256": hashlib.sha256(positions.astype("<f4").tobytes()).hexdigest()}


def farthest_samples(points, count):
    """Keep the footprint hull exactly and distribute interior samples evenly."""
    chosen = list(ConvexHull(points).vertices)
    minimum_distance = np.min(np.sum((points[:, None] - points[chosen]) ** 2, axis=2), axis=1)
    while len(chosen) < min(count, len(points)):
        index = int(np.argmax(minimum_distance))
        if minimum_distance[index] < 1e-14:
            break
        chosen.append(index)
        minimum_distance = np.minimum(minimum_distance, np.sum((points - points[index]) ** 2, axis=1))
    return np.array(chosen)


def anchor_space(points, matrix):
    return points @ matrix[:3, :3].T + matrix[:3, 3]


def layered_cage(body, side):
    # The source FBX mesh is metre-scaled Z-up, with the chest facing -Y.
    # Inverse-bind conversion below makes the exported cage bone-local.
    active = body["weights"][side] > 0.001
    patch = body["positions"][active].astype(float)
    footprint, unique = np.unique(np.round(patch[:, [0, 2]], 7), axis=0, return_index=True)
    patch = patch[unique]
    selected = farthest_samples(footprint, 64)
    surface = patch[selected].copy()
    triangulation = Delaunay(surface[:, [0, 2]])
    # Backing plane comes from the posterior edge of the source skin patch.
    # A 4 mm margin avoids zero-volume wedges at the patch boundary.
    back = float(patch[:, 1].max() + 0.004)
    levels = [0.0, 0.35, 0.7, 1.0]
    count = len(surface)
    layers = []
    for level in levels:
        points = surface.copy()
        points[:, 1] = back + level * (surface[:, 1] - back)
        layers.append(points)
    points = np.concatenate(layers)
    tets = []
    for layer in range(len(levels) - 1):
        for triangle in triangulation.simplices:
            a, b, c = sorted(int(i) + layer * count for i in triangle)
            # A globally ordered prism split agrees on shared rectangular faces.
            tets.extend([[a, b, c, c + count], [a, b, b + count, c + count], [a, a + count, b + count, c + count]])
    local = anchor_space(points, body["anchor_inverse"])
    volumes = []
    for tet in tets:
        p = local[tet]
        volume = float(np.linalg.det((p[1:] - p[0]).T) / 6)
        if volume < 0:
            tet[1], tet[2] = tet[2], tet[1]
            volume = -volume
        if volume <= 1e-12:
            raise ValueError("Degenerate cage tetrahedron")
        volumes.append(volume)
    attachments = [{"kind": "fixed"} if i < count else {"kind": "free"} for i in range(len(points))]
    region = {"name": ["left_breast", "right_breast"][side], "anchor_bone": "CC_Base_Spine02",
              "rest_positions": local.tolist(), "initial_positions": local.tolist(),
              "tetrahedra": tets, "attachments": attachments,
              "material": {"density_kg_m3": 950.0, "shear_pa": 1500.0, "bulk_pa": 30000.0,
                           "strain_damping_per_second": 30.0, "iterations": 8}}
    return region, {"nodes": len(points), "tetrahedra": len(tets), "volume_m3": sum(volumes),
                    "mass_kg": 950.0 * sum(volumes), "minimum_tet_volume_m3": min(volumes)}


def surface_influence(surface, side, transition_m=0.02):
    """Transfer the full volume displacement inside a geometric chest transition.

    Bone weights identify the existing authoring patch only; their magnitudes do
    not attenuate simulated motion. A C1 smoothstep spans the first 20 mm of
    surface distance from the patch boundary. Weld coincident UV-seam vertices
    before measuring distance so seams do not become artificial attachments.
    """
    if not np.isfinite(transition_m) or transition_m <= 0:
        raise ValueError("Surface transition must be a positive distance")
    points, original = np.unique(np.round(surface["positions"].astype(float), 7), axis=0, return_inverse=True)
    active = np.zeros(len(points), dtype=bool)
    np.logical_or.at(active, original, surface["weights"][side] > 0.001)
    triangles = original[surface["triangles"]]
    mixed = triangles[np.any(active[triangles], axis=1) & ~np.all(active[triangles], axis=1)]
    boundary = np.unique(mixed[active[mixed]])
    if not len(boundary):
        raise ValueError("Tissue patch has no surface boundary")
    edges = np.concatenate([triangles[:, [0, 1]], triangles[:, [1, 2]], triangles[:, [2, 0]]])
    edges = np.unique(np.sort(edges, axis=1), axis=0)
    edges = edges[np.all(active[edges], axis=1) & (edges[:, 0] != edges[:, 1])]
    lengths = np.linalg.norm(points[edges[:, 1]] - points[edges[:, 0]], axis=1)
    graph = coo_matrix((lengths, (edges[:, 0], edges[:, 1])), shape=(len(points), len(points))).tocsr()
    distance = dijkstra(graph, directed=False, indices=boundary, min_only=True)
    if not np.all(np.isfinite(distance[active])):
        raise ValueError("Tissue patch contains a component without a boundary")
    influence = np.zeros(len(points))
    t = np.minimum(distance[active] / transition_m, 1.)
    influence[active] = t * t * (3. - 2. * t)
    return influence[original]


def embed(surface, region, side, influence=None):
    """Bind displacements, preserving exact original skin and garment clearance.

    Boundary/outside vertices use the nearest convex barycentric projection. The
    unmodified source position retains its residual offset from that projection.
    """
    initial = np.asarray(region["initial_positions"])
    topology = np.asarray(region["tetrahedra"])
    tetra = initial[topology]
    inverses = np.linalg.inv((tetra[:, 1:] - tetra[:, :1]).transpose(0, 2, 1))
    points = anchor_space(surface["positions"], surface["anchor_inverse"])
    if influence is None:
        influence = surface_influence(surface, side)
    result = []
    maximum_distance = 0.0
    for vertex in np.flatnonzero(influence > 0):
        bary3 = np.einsum("tij,tj->ti", inverses, points[vertex] - tetra[:, 0])
        bary = np.column_stack((1 - bary3.sum(axis=1), bary3))
        inside = np.flatnonzero(np.min(bary, axis=1) >= -1e-8)
        if len(inside):
            tet = int(inside[0])
            weights = np.maximum(bary[tet], 0)
            weights /= weights.sum()
        else:
            candidates = np.maximum(bary, 0)
            candidates /= candidates.sum(axis=1, keepdims=True)
            distances = np.linalg.norm(np.einsum("ti,tij->tj", candidates, tetra) - points[vertex], axis=1)
            tet = int(np.argmin(distances))
            weights = candidates[tet]
            maximum_distance = max(maximum_distance, float(distances[tet]))
        result.append({"vertex": int(vertex), "region": side, "tetrahedron": tet,
                       "barycentric": weights.tolist(), "influence": float(influence[vertex])})
    return result, maximum_distance


def build(paths):
    all_surfaces = {}
    for path in paths:
        for surface in surfaces(path):
            all_surfaces.setdefault(surface["positions_sha256"], surface)
    body = next(s for s in all_surfaces.values() if s["name"] == "CC_Base_Body")
    cages = [layered_cage(body, side) for side in range(2)]
    regions = [cage[0] for cage in cages]
    bindings, summaries = [], []
    body_vertices = [v for side, region in enumerate(regions) for v in embed(body, region, side)[0]]
    for surface in all_surfaces.values():
        if surface["name"] != "CC_Base_Body":
            vertices, distance = transfer_from_body(body, body_vertices, surface)
            summaries.append({"mesh": surface["name"], "bound_vertices": len({v['vertex'] for v in vertices}),
                              "maximum_embedding_residual_m": distance, "transfer": "body surface displacement"})
            bindings.append({"mesh_name": surface["name"], "positions_sha256": surface["positions_sha256"],
                             "vertex_count": len(surface["positions"]), "vertices": vertices})
            continue
        vertices = []
        for side, region in enumerate(regions):
            embedded, distance = embed(surface, region, side)
            vertices.extend(embedded)
            summaries.append({"mesh": surface["name"], "side": side, "bound_vertices": len(embedded),
                              "maximum_embedding_residual_m": distance})
        bindings.append({"mesh_name": surface["name"], "positions_sha256": surface["positions_sha256"],
                         "vertex_count": len(surface["positions"]), "vertices": vertices})
    return {"schema_version": 1, "source_sha256": [hashlib.sha256(path.read_bytes()).hexdigest() for path in paths], "regions": regions, "surfaces": bindings}, {
        "source_sha256": {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths},
        "cages": [cage[1] for cage in cages], "surfaces": summaries,
        "status": "unfitted authoring asset; requires loaded-equilibrium fitting and rendered acceptance"}


def transfer_from_body(body, body_vertices, surface):
    """Compose garment coordinates with the body's existing tissue embedding.

    The garment retains its original positions/clearance. Its old bone weights
    still perform articulated skinning, but do not independently attenuate the
    tissue displacement (which can differ sharply across thin fabric layers).
    """
    import trimesh
    body_points = anchor_space(body["positions"], body["anchor_inverse"])
    garment_points = anchor_space(surface["positions"], surface["anchor_inverse"])
    carrier = trimesh.Trimesh(vertices=body_points, faces=body["triangles"], process=False)
    closest, distances, triangles = trimesh.proximity.closest_point(carrier, garment_points)
    bary = trimesh.triangles.points_to_barycentric(body_points[body["triangles"][triangles]], closest)
    bary = np.maximum(bary, 0)
    bary /= bary.sum(axis=1, keepdims=True)
    by_vertex = {}
    for binding in body_vertices:
        by_vertex.setdefault(binding["vertex"], []).append(binding)
    result = []
    for vertex, (triangle, coordinates) in enumerate(zip(triangles, bary)):
        accumulated = {}
        for body_vertex, weight in zip(body["triangles"][triangle], coordinates):
            for binding in by_vertex.get(int(body_vertex), []):
                key = (binding["region"], binding["tetrahedron"])
                accumulated.setdefault(key, np.zeros(4))
                accumulated[key] += weight * binding["influence"] * np.asarray(binding["barycentric"])
        for (region, tetrahedron), coefficients in sorted(accumulated.items()):
            influence = float(coefficients.sum())
            if influence > 1 + 1e-10:
                raise ValueError("Composed garment binding amplifies body displacement")
            if influence > 1e-12:
                result.append({"vertex": vertex, "region": region, "tetrahedron": tetrahedron,
                               "barycentric": (coefficients / influence).tolist(), "influence": min(influence, 1.0)})
    active = sorted({binding["vertex"] for binding in result})
    return result, float(distances[active].max()) if active else 0.0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("sources", type=Path, nargs="+")
    args = parser.parse_args()
    if args.output.exists():
        raise ValueError("Output exists; preserve the previous authoring artifact")
    asset, report = build(args.sources)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(asset, separators=(",", ":")) + "\n")
    args.output.with_suffix(".report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
