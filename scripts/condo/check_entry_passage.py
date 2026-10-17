"""Check a doorway clearance envelope against observed open/closed door geometry."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np

from audit_mechanism_trace import gaps, rotation
from check_fridge_mechanism import obstacles, yaw


def check(scene_path, trace_path, output):
    scene = json.loads(scene_path.read_text())
    trace = json.loads(trace_path.read_text())
    mechanism = next(m for m in scene['mechanisms'] if m['id'] == trace['mechanism'])
    assert mechanism['id'] == 'condo_entry_door'
    fixed = obstacles(mechanism, scene)
    parts = {p['id']:p for p in mechanism['parts']}
    # A conservative 70 cm wide, 190 cm high clearance box swept straight
    # through the opening. This is not generated-body or exterior-floor proof.
    center, half = np.array([-.35, 1.1, -4.75]), np.array([.35, .95, .50])+.001
    results = []
    for phase in (0, 3):
        sample = next(f for f in reversed(trace['frames']) if f['phase'] == phase)
        boxes = [(s['id'], s['position'], s['half_extents'], yaw(np.deg2rad(s.get('yaw_deg', 0)))) for s in fixed]
        for state in sample['parts']:
            orient = rotation(state['rotation_xyzw'])
            for shape in parts[state['id']]['colliders']:
                at = np.array(state['position'])+orient@np.array(shape['position'])
                boxes.append((shape['id'], at, shape['half_extents'], orient))
        values = gaps(center, half, np.eye(3), np.array([b[1] for b in boxes]),
                      np.array([b[2] for b in boxes]), np.array([b[3] for b in boxes]))
        index = int(np.argmin(values))
        results.append({'phase':phase, 'minimum_gap_m':float(values[index]),
                        'nearest':boxes[index][0], 'frame':sample['frame']})
    assert results[0]['minimum_gap_m'] < 0 and results[0]['nearest'] == 'entry_door_panel', results
    assert results[1]['minimum_gap_m'] > 0, results
    report = {'passes':True, 'scope':__doc__, 'envelope':{'center':center.tolist(), 'half_extents':half.tolist()},
        'checks':results, 'inputs':[{'path':str(p), 'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
        for p in (scene_path, trace_path, Path(__file__))],
        'limits':['Observed fully open pose; not passage during opening.',
                  'Conservative clearance proxy only; no avatar motion or exterior hallway support.']}
    with output.open('x') as stream:
        json.dump(report, stream, indent=2); stream.write('\n')
    print(json.dumps(report))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scene', type=Path)
    parser.add_argument('trace', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    check(args.scene, args.trace, args.output)
