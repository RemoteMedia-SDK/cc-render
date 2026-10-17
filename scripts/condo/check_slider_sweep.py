"""Exact swept-box envelope for an axis-aligned passive rail, with 1 mm reserve."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np

from audit_mechanism_trace import gaps
from check_fridge_mechanism import obstacles, yaw


def check(scene_path, mechanism_id, output):
    scene = json.loads(scene_path.read_text())
    mechanism = next(m for m in scene['mechanisms'] if m['id'] == mechanism_id)
    fixed = obstacles(mechanism, scene)
    centers = np.array([b['position'] for b in fixed])
    halves = np.array([b['half_extents'] for b in fixed])
    orientations = np.array([yaw(np.deg2rad(b.get('yaw_deg', 0))) for b in fixed])
    results = []
    for part in mechanism['parts']:
        assert hashlib.sha256((scene_path.parent / part['geometry']).read_bytes()).hexdigest() == part['sha256']
        if 'joint' not in part:
            continue
        joint = part['joint']
        assert joint['kind'] == 'prismatic'
        axis = np.array(joint['axis'])
        assert np.count_nonzero(axis) == 1 and np.linalg.norm(axis) == 1
        lo, hi = joint['limits_m']
        for shape in part['colliders']:
            center = np.array(mechanism['placement']) + part['position'] + np.array(shape['position']) + axis*(lo+hi)/2
            half = np.array(shape['half_extents']) + abs(axis)*(hi-lo)/2 + .001
            values = gaps(center, half, np.eye(3), centers, halves, orientations)
            index = int(np.argmin(values))
            assert gaps(center, half, np.eye(3), center[None, :], half[None, :], np.eye(3)[None, :, :])[0] < 0
            results.append({'part':part['id'], 'shape':shape['id'],
                            'gap_m':float(values[index]), 'nearest':fixed[index]['id']})
    assert results
    report = {'scope':__doc__, 'passes':all(r['gap_m'] > 0 for r in results),
              'shapes':results, 'pair_checks':len(results)*len(fixed),
              'negative_control_rejected':True,
              'scene_sha256':hashlib.sha256(scene_path.read_bytes()).hexdigest(),
              'limits':'Authored travel against fixed collision boxes; excludes dynamic peers, solver drift and avatar motion.'}
    with output.open('x') as stream:
        json.dump(report, stream, indent=2); stream.write('\n')
    print(json.dumps(report, indent=2))
    assert report['passes'], report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scene', type=Path)
    parser.add_argument('mechanism')
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    check(args.scene, args.mechanism, args.output)
