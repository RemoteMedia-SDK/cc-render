"""Independent read-only mesh-distance audit of the existing Bevy snapshot."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import numpy as np
import trimesh
sys.path.insert(0, str(Path.cwd() / 'scripts/avatars'))
from build_tissue_asset import read_glb, accessor

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('snapshot', type=Path)
parser.add_argument('asset', type=Path)
parser.add_argument('glb', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
snapshot = json.loads(args.snapshot.read_text())
asset = json.loads(args.asset.read_text())
document, binary = read_glb(args.glb)
assert hashlib.sha256(args.glb.read_bytes()).hexdigest() in asset['source_sha256']
primitives = {}
for mesh in document['meshes']:
    for primitive in mesh['primitives']:
        positions = accessor(document, binary, primitive['attributes']['POSITION'])
        fingerprint = hashlib.sha256(positions.astype('<f4').tobytes()).hexdigest()
        if fingerprint in primitives:
            raise ValueError('Ambiguous primitive fingerprint')
        primitives[fingerprint] = (accessor(document, binary, primitive['indices']).reshape(-1, 3).astype(int), document['materials'][primitive['material']]['name'])

def fingerprint(surface):
    return hashlib.sha256(np.asarray(surface['base_positions'], dtype='<f4').tobytes()).hexdigest()

surfaces = {fingerprint(s): s for s in snapshot['surfaces']}
binding = asset['surfaces'][0]
body = surfaces[binding['positions_sha256']]
sample_ids = np.asarray(sorted({v['vertex'] for v in binding['vertices']}))
body_faces = primitives[binding['positions_sha256']][0]
active_faces = np.asarray(body['affected_triangles'])
edges = np.unique(np.sort(np.concatenate([active_faces[:, [0, 1]], active_faces[:, [1, 2]], active_faces[:, [2, 0]]]), axis=1), axis=0)
report = {'source_pts_ms': snapshot['source_pts_ms'], 'snapshot_sha256': hashlib.sha256(args.snapshot.read_bytes()).hexdigest(), 'asset_sha256': hashlib.sha256(args.asset.read_bytes()).hexdigest(), 'source_glb_sha256': hashlib.sha256(args.glb.read_bytes()).hexdigest(), 'trimesh_version': trimesh.__version__, 'sample_vertices': len(sample_ids), 'comparisons': [], 'scope': 'Static sample-to-triangle distances and tissue-edge to arm-face crossings at one unchanged displayed frame; not continuous collision certification.'}
for digest, surface in surfaces.items():
    faces, material = primitives[digest]
    if digest == binding['positions_sha256']:
        faces = faces[np.all(~np.isin(faces, sample_ids), axis=1)]
    for state in ['before_world', 'after_world']:
        vertices = np.asarray(surface[state]); points = np.asarray(body[state])[sample_ids]
        mesh = trimesh.Trimesh(vertices=vertices, faces=faces, process=False)
        nearest, distance, face_ids = trimesh.proximity.closest_point(mesh, points)
        best = int(np.argmin(distance)); point = points[best]
        # Exhaustive triangle projection for the reported closest sample,
        # independent of the acceleration structure used above.
        exact = trimesh.triangles.closest_point(vertices[faces], np.repeat(point[None], len(faces), axis=0))
        exhaustive = float(np.linalg.norm(exact - point, axis=1).min())
        assert abs(exhaustive - distance[best]) < 1e-10
        item = {'material': material, 'state': state, 'triangles': len(faces), 'minimum_gap_mm': float(distance[best] * 1000), 'exhaustive_minimum_gap_mm': exhaustive * 1000, 'nearest_sample_vertex': int(sample_ids[best]), 'nearest_triangle_vertices': faces[face_ids[best]].tolist(), 'gap_percentiles_mm': (np.percentile(distance, [0, 1, 50, 99, 100]) * 1000).tolist()}
        if digest != binding['positions_sha256']:
            bp = np.asarray(body[state]); start = bp[edges[:, 0]]; segment = bp[edges[:, 1]] - start; length = np.linalg.norm(segment, axis=1)
            valid = length > 1e-10; tested = edges[valid]; start = start[valid]; length = length[valid]; direction = segment[valid] / length[:, None]
            locations, rays, hit_faces = mesh.ray.intersects_location(start, direction, multiple_hits=True)
            along = np.sum((locations - start[rays]) * direction[rays], axis=1)
            crossings = (along > 1e-7) & (along < length[rays] - 1e-7)
            item['tissue_edge_arm_face_crossings'] = [{'edge': tested[r].tolist(), 'face': faces[f].tolist()} for r, f in zip(rays[crossings], hit_faces[crossings])]
        report['comparisons'].append(item)
with args.output.open('x') as file:
    json.dump(report, file, indent=2); file.write('\n')
print(json.dumps(report, indent=2))
