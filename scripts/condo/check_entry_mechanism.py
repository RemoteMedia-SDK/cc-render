"""Conservative cabinet/room-door sweep; read-only geometry evidence."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np

from check_fridge_mechanism import door_samples, obstacles, sweep


def independent_door_gap(mechanism):
    # These opposing hinges keep their leaves in separate X half-spaces.
    # Enclose every corner at every sample, expanded by the same continuous
    # angular movement bound used for the room sweep, plus its 1 mm reserve.
    ranges = {}
    for identity, (position, half, rotation), margin in door_samples(mechanism):
        radius = np.dot(abs(rotation[0]), half) + margin
        lo, hi = ranges.get(identity['part'], (float('inf'), -float('inf')))
        ranges[identity['part']] = (min(lo, position[0]-radius), max(hi, position[0]+radius))
    gap = ranges['entry_right_door'][0] - ranges['entry_left_door'][1]
    return {'pass':bool(gap > 0), 'continuous_x_gap_m':gap, 'expanded_x_ranges':ranges,
            'scope':'All independent door angles, using disjoint swept X envelopes.'}


def check(scene_path, output, mechanism_id='entry_cabinet'):
    scene = json.loads(scene_path.read_text())
    mechanism = next(m for m in scene['mechanisms'] if m['id'] == mechanism_id)
    for part in mechanism['parts']:
        assert hashlib.sha256((scene_path.parent / part['geometry']).read_bytes()).hexdigest() == part['sha256']
    report = sweep(mechanism, obstacles(mechanism, scene))
    if mechanism_id == 'entry_cabinet':
        report['independent_doors'] = independent_door_gap(mechanism)
    else:
        assert sum('joint' in p for p in mechanism['parts']) == 1
        report['independent_doors'] = {'pass':True, 'scope':'Single leaf; no independent moving pair.'}
    identity, (position, _, _), _ = next(door_samples(mechanism))
    negative = sweep(mechanism, [{'id':'negative_control', 'position':position.tolist(),
                                 'half_extents':[.06, .06, .06]}])
    report['negative_control_rejected'] = not negative['pass']
    report['scene_sha256'] = hashlib.sha256(scene_path.read_bytes()).hexdigest()
    report['passes'] = report['pass'] and report['independent_doors']['pass'] and not negative['pass']
    with output.open('x') as stream:
        json.dump(report, stream, indent=2); stream.write('\n')
    print(json.dumps(report, indent=2))
    assert report['passes'], report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scene', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--mechanism', choices=('entry_cabinet', 'bedroom_door'), default='entry_cabinet')
    args = parser.parse_args()
    check(args.scene, args.output, args.mechanism)
