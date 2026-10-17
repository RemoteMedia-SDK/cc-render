"""Check recorded rigid poses against fixed room boxes, including hinge drift/overshoot.

Full 3D separating axes; checks sampled frames, not continuous motion or avatar contact.
Diagnostic obstruction contacts are intentionally excluded from room clearance.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np

from check_fridge_mechanism import obstacles, yaw


def rotation(quaternion):
    q = np.asarray(quaternion, dtype=float)
    assert abs(np.linalg.norm(q)-1) < 1e-5
    x, y, z, w = q / np.linalg.norm(q)
    return np.array([[1-2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)],
                     [2*(x*y+z*w), 1-2*(x*x+z*z), 2*(y*z-x*w)],
                     [2*(x*z-y*w), 2*(y*z+x*w), 1-2*(x*x+y*y)]])


def gaps(center, half, orient, centers, halves, orientations):
    """Largest normalized separating-axis gap for each other oriented box."""
    own = np.broadcast_to(orient.T, orientations.shape)
    other = orientations.transpose(0, 2, 1)
    cross = np.cross(own[:, :, None, :], other[:, None, :, :]).reshape(-1, 9, 3)
    axes = np.concatenate((own, other, cross), axis=1)
    norm = np.linalg.norm(axes, axis=2)
    valid = norm > 1e-10
    axes /= np.where(valid, norm, 1)[:, :, None]
    distance = abs(np.einsum('nki,ni->nk', axes, centers-center))
    a = np.sum(abs(axes@orient)*half, axis=2)
    b = np.sum(abs(np.einsum('nki,nij->nkj', axes, orientations))*halves[:, None, :], axis=2)
    return np.max(np.where(valid, distance-a-b, -np.inf), axis=1)


def audit(scene_path, trace_path, output):
    scene = json.loads(scene_path.read_text())
    trace = json.loads(trace_path.read_text())
    mechanism = next(m for m in scene['mechanisms'] if m['id'] == trace['mechanism'])
    assert np.asarray(mechanism['placement'], dtype=np.float32).astype(float).tolist() == trace['placement']
    parts = {p['id']:p for p in mechanism['parts']}
    fixed = obstacles(mechanism, scene)
    centers = np.array([b['position'] for b in fixed])
    halves = np.array([b['half_extents'] for b in fixed])
    orientations = np.array([yaw(np.deg2rad(b.get('yaw_deg', 0))) for b in fixed])
    minimum, count, nearest = float('inf'), 0, None
    expected = {p['id'] for p in mechanism['parts'] if p.get('joint')}
    assert trace['frames'] and expected and len(fixed)
    for frame in trace['frames']:
        assert {p['id'] for p in frame['parts']} == expected
        for pose in frame['parts']:
            orient = rotation(pose['rotation_xyzw'])
            for shape in parts[pose['id']]['colliders']:
                center = np.array(pose['position']) + orient@np.array(shape['position'])
                half = np.array(shape['half_extents'])
                values = gaps(center, half, orient, centers, halves, orientations)
                assert np.isfinite(values).all(), 'Nonfinite recorded geometry'
                index = int(np.argmin(values)); count += len(values)
                if values[index] < minimum:
                    minimum = float(values[index])
                    nearest = {'frame':frame['frame'], 'part':pose['id'], 'shape':shape['id'], 'obstacle':fixed[index]['id']}
    # Coincident boxes must penetrate; a translated pair must be separated.
    unit = np.eye(3)[None, :, :]
    assert gaps(np.zeros(3), np.ones(3), unit[0], np.zeros((1, 3)), np.ones((1, 3)), unit)[0] < 0
    assert gaps(np.zeros(3), np.ones(3), unit[0], np.array([[3., 0, 0]]), np.ones((1, 3)), unit)[0] > 0
    report = {'scope':__doc__, 'passes':minimum >= 0, 'pair_checks':count,
              'minimum_separating_gap_m':minimum, 'nearest':nearest,
              'inputs':[{'path':str(p), 'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
                        for p in (scene_path, trace_path, Path(__file__))]}
    with output.open('x') as stream:
        json.dump(report, stream, indent=2); stream.write('\n')
    print(json.dumps(report, indent=2))
    assert report['passes'], report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scene', type=Path)
    parser.add_argument('trace', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    audit(args.scene, args.trace, args.output)
