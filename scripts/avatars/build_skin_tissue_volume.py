#!/usr/bin/env python3
"""Author a tetrahedral volume bounded by source skin and a chest attachment cap.

The posterior cap is an explicit geometric approximation, not medical anatomy.
Only the surrounding chest rim and posterior cap are fixed. Source skin inside
that boundary receives unattenuated volumetric displacement.
"""
import argparse
import copy
import json
from pathlib import Path

import numpy as np
import pyvista as pv
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra
import tetgen
import trimesh

from build_tissue_asset import anchor_space, embed, surfaces


def skin_patch(body, side, collar_m=0.025):
    points, original = np.unique(np.round(body['positions'].astype(float), 7), axis=0, return_inverse=True)
    faces = original[body['triangles']]
    edges = np.unique(np.sort(np.concatenate([faces[:, [0, 1]], faces[:, [1, 2]], faces[:, [2, 0]]]), axis=1), axis=0)
    edges = edges[edges[:, 0] != edges[:, 1]]
    lengths = np.linalg.norm(points[edges[:, 1]] - points[edges[:, 0]], axis=1)
    graph = coo_matrix((lengths, (edges[:, 0], edges[:, 1])), shape=(len(points), len(points))).tocsr()
    seeds = np.unique(original[body['weights'][side] > .001])
    distance = dijkstra(graph, directed=False, indices=seeds, min_only=True)
    # Partition the medial chest at the source sagittal plane; the regions never overlap.
    same_side = points[:, 0] > 0 if side == 0 else points[:, 0] < 0
    morphed = np.zeros(len(points), dtype=bool)
    np.logical_or.at(morphed, original, body['position_morphed'])
    # The runtime supports a fixed skin embedding. Keep the chest collar and
    # its one-ring normal stencil outside position-morphed shoulder vertices.
    forbidden = np.zeros(len(points), dtype=bool)
    forbidden[np.unique(faces[np.any(morphed[faces], axis=1)])] = True
    selected = (distance <= collar_m) & same_side & ~forbidden
    patch_faces = faces[np.all(selected[faces], axis=1)]
    used, remap = np.unique(patch_faces, return_inverse=True)
    patch_faces = remap.reshape(-1, 3)
    membership = np.isin(original, used).astype(float)
    mesh = pv.PolyData(points[used], np.column_stack([np.full(len(patch_faces), 3), patch_faces]))
    return mesh, membership


def boundary_edges(faces):
    edges = np.concatenate([faces[:, [0, 1]], faces[:, [1, 2]], faces[:, [2, 0]]])
    unique, count = np.unique(np.sort(edges, axis=1), axis=0, return_counts=True)
    if np.any(count > 2):
        raise ValueError('Non-manifold skin patch')
    boundary = unique[count == 1]
    vertices, degree = np.unique(boundary, return_counts=True)
    if not len(boundary) or np.any(degree != 2):
        raise ValueError('Skin patch must have closed boundary loops')
    # Check that the boundary is one loop; a cap must not silently cover holes.
    adjacency = {int(v): set() for v in vertices}
    for a, b in boundary:
        adjacency[int(a)].add(int(b)); adjacency[int(b)].add(int(a))
    loop, previous = [int(vertices[0])], None
    while True:
        following = min(adjacency[loop[-1]] - {previous})
        if following == loop[0]:
            break
        previous = loop[-1]
        loop.append(following)
    if len(loop) != len(vertices):
        raise ValueError('Skin patch has multiple boundary loops')
    return boundary, np.asarray(loop)


def chest_cap(points, loop):
    """Close the chest rim behind the skin without a fan across the breast fold.

    The backing plane is an authoring approximation. The surrounding chest skin
    supplies its depth; a 4 mm margin keeps the outermost wedges non-degenerate.
    """
    rim = points[loop]
    backing = rim.copy()
    backing[:, 1] = rim[:, 1].max() + .004
    back_ids = np.arange(len(points), len(points) + len(loop))
    points = np.vstack([points, backing])
    polygon = pv.PolyData(points, np.r_[len(back_ids), back_ids]).triangulate()
    cap = polygon.faces.reshape(-1, 4)[:, 1:]
    walls = []
    for i in range(len(loop)):
        j = (i + 1) % len(loop)
        walls.extend([[loop[i], loop[j], back_ids[j]], [loop[i], back_ids[j], back_ids[i]]])
    return points, np.vstack([cap, walls])


def close_and_mesh(patch, shell_path, surface_points=128, max_volume=None, node_budget=512):
    """`surface_points` sets the decimated skin resolution; `max_volume` (m^3)
    bounds tetrahedron size for a more uniform interior. Defaults reproduce
    the original small cage."""
    reduction = min(.88, max(0., 1. - surface_points / patch.n_points))
    coarse = patch.decimate_pro(reduction, preserve_topology=True, splitting=False,
                                boundary_vertex_deletion=False)
    points = np.asarray(coarse.points).copy()
    faces = coarse.faces.reshape(-1, 4)[:, 1:].copy()
    _, loop = boundary_edges(faces)
    points, cap_faces = chest_cap(points, loop)
    closed_faces = np.vstack([faces, cap_faces])
    closed = pv.PolyData(points, np.column_stack([np.full(len(closed_faces), 3), closed_faces]))
    closed = closed.compute_normals(point_normals=False, cell_normals=True,
                                    consistent_normals=True, auto_orient_normals=True)
    shell = trimesh.Trimesh(closed.points, closed.faces.reshape(-1, 4)[:, 1:], process=False)
    if not shell.is_watertight or not shell.is_winding_consistent or shell.volume <= 0:
        raise ValueError('Invalid closed tissue shell')
    shell.export(shell_path)
    mesher = tetgen.TetGen(shell.vertices, shell.faces)
    options = dict(quality=True, minratio=1.8, nobisect=True, quiet=True)
    if max_volume is not None:
        options['maxvolume'] = max_volume
    nodes, cells = mesher.tetrahedralize(**options)
    cap = trimesh.Trimesh(points, cap_faces, process=False)
    _, distance, _ = trimesh.proximity.closest_point(cap, nodes)
    fixed = distance < 1e-7
    if len(nodes) > node_budget:
        raise ValueError(f'Tissue volume exceeds small-solver budget: {len(nodes)} nodes')
    tetra = nodes[cells]
    volume = np.linalg.det((tetra[:, 1:] - tetra[:, :1]).transpose(0, 2, 1)) / 6
    inverted = volume < 0
    cells[inverted, 1], cells[inverted, 2] = cells[inverted, 2].copy(), cells[inverted, 1].copy()
    if np.min(abs(volume)) <= 1e-12:
        raise ValueError('Degenerate tissue element')
    return nodes, cells, fixed, shell, {
        'nodes': len(nodes), 'tetrahedra': len(cells), 'fixed_nodes': int(fixed.sum()),
        'volume_m3': float(abs(volume).sum()), 'minimum_tet_volume_m3': float(abs(volume).min()),
        'source_patch_vertices': patch.n_points, 'coarse_surface_vertices': len(points),
        'cap_inset_m': .004, 'collar_m': .025,
        'surface_points': surface_points, 'max_volume_m3': max_volume,
    }


def build(avatar, material_asset, output, **mesh_options):
    original = json.loads(material_asset.read_text())
    body = next(s for s in surfaces(avatar) if s['name'] == 'CC_Base_Body')
    asset = copy.deepcopy(original)
    bindings, reports = [], []
    for side in range(2):
        patch, membership = skin_patch(body, side)
        nodes, cells, fixed, shell, report = close_and_mesh(
            patch, output.with_name(output.stem + f'-shell-{side}.ply'), **mesh_options)
        local = anchor_space(nodes, body['anchor_inverse'])
        region = asset['regions'][side]
        region.update(rest_positions=local.tolist(), initial_positions=local.tolist(),
                      tetrahedra=cells.tolist(), attachments=[{'kind': 'fixed' if f else 'free'} for f in fixed])
        vertices, residual = embed(body, region, side, influence=membership)
        vertices = [binding for binding in vertices if any(
            weight > 0 and not fixed[node]
            for node, weight in zip(cells[binding['tetrahedron']], binding['barycentric']))]
        bindings.extend(vertices)
        report['maximum_embedding_residual_m'] = residual
        report['bound_vertices'] = len(vertices)
        reports.append(report)
    assert len(asset['surfaces']) == 1 and asset['surfaces'][0]['positions_sha256'] == body['positions_sha256']
    asset['surfaces'][0]['vertices'] = bindings
    with output.open('x') as file:
        json.dump(asset, file, separators=(',', ':'))
    output.with_suffix('.report.json').write_text(json.dumps(reports, indent=2) + '\n')
    return reports


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('avatar', type=Path)
    parser.add_argument('material_asset', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--surface-points', type=int, default=128)
    parser.add_argument('--max-volume', type=float, help='Maximum tetrahedron volume, m^3.')
    parser.add_argument('--node-budget', type=int, default=512)
    args = parser.parse_args()
    print(json.dumps(build(args.avatar, args.material_asset, args.output, surface_points=args.surface_points,
                           max_volume=args.max_volume, node_budget=args.node_budget), indent=2))
