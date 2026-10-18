"""Compare all captured nodes against the reviewed video and report live timing."""
from pathlib import Path
import argparse
import json
import sys
import numpy as np

HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('label', nargs='?', default='cache')
parser.add_argument('--baseline', default='lab-boundary-nodal-front-01')
parser.add_argument('--current')
parser.add_argument('--live-before', default='lab-boundary-live120-01')
parser.add_argument('--live-current')
parser.add_argument('--output', type=Path)
args = parser.parse_args()
label = args.label
assert label.replace('-', '').isalnum()
OUT = args.output if args.output else (HERE if label == 'cache' else HERE / ('bevy-' + label))
ARTIFACTS = HERE.parent
sys.path.insert(0, str(ARTIFACTS/'solver-runtime'))
from startup_cadence_audit import inspect as fixed
from live_clock_audit import inspect as live

packets = (ARTIFACTS/'lab-jump-01/packets.ndjson').read_bytes()
baseline, old = fixed(args.baseline, packets)
current, new = fixed(args.current or 'lab-boundary-'+label+'240-01', packets)
before_live = live(args.live_before, packets)[0]
after_live = live(args.live_current or 'lab-boundary-'+label+'-live120-01', packets)[0]
assert old.keys() == new.keys()
nodes, anchors, rotations, surface = [], [], [], []
for time in old:
    a, b = old[time], new[time]
    surface.append(abs(a['maximum_surface_displacement_m'] - b['maximum_surface_displacement_m']))
    for x, y in zip(a['regions'], b['regions'], strict=True):
        nodes.extend(np.linalg.norm(np.asarray(x['nodes_world_m'])-y['nodes_world_m'], axis=1))
        anchors.append(np.linalg.norm(np.asarray(x['anchor_world_m'])-y['anchor_world_m']))
        rotations.append(np.linalg.norm(np.asarray(x['anchor_rotation_xyzw'])-y['anchor_rotation_xyzw']))
report = {
    'fixed_baseline': baseline, 'fixed_current': current,
    'live_before': before_live, 'live_current': after_live,
    'parity': {'frames': len(old), 'maximum_node_difference_m': float(max(nodes)),
               'rms_node_difference_m': float(np.sqrt(np.mean(np.asarray(nodes)**2))),
               'maximum_attachment_translation_difference_m': float(max(anchors)),
               'maximum_attachment_quaternion_chord_difference': float(max(rotations)),
               'maximum_surface_envelope_difference_m': max(surface)},
    'limits': ['Single runs on a shared host; absolute timing is not a controlled benchmark.',
               'Live captures have different sample times and cannot prove mechanical parity.',
               'Readbacks are not browser presentations. The jump contains no active contacts.'],
}
(OUT/'bevy-comparison.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report, indent=2))
assert max(nodes) < 1e-6, 'Deformation changed by more than one micrometre'
assert max(anchors) < 1e-7 and max(rotations) < 1e-7, 'Attachment sampling changed'
