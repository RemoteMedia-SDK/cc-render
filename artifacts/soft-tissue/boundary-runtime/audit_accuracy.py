"""Numerical-accuracy study; the existing 1 um equivalence gate is unchanged."""
from pathlib import Path
import json
import sys
import numpy as np

HERE = Path(__file__).resolve().parent
ARTIFACTS = HERE.parent
label = sys.argv[1]
assert label.replace('-', '').isalnum()
OUT = HERE / ('bevy-' + label)
sys.path.insert(0, str(ARTIFACTS/'solver-runtime'))
from startup_cadence_audit import inspect as fixed
from live_clock_audit import inspect as live

# Engineering screen for this experiment, declared before its Bevy capture.
# This is a trajectory-error budget, not a material or anatomical tolerance.
MAXIMUM_NODE_ERROR_M = 0.0001
packets = (ARTIFACTS/'lab-jump-01/packets.ndjson').read_bytes()
reference, old = fixed('lab-boundary-energy-cache240-01', packets)
current, new = fixed('lab-boundary-'+label+'240-01', packets)
assert old.keys() == new.keys()
errors, anchors, rotations, envelopes = [], [], [], []
for time in old:
    a, b = old[time], new[time]
    envelopes.append(abs(a['maximum_surface_displacement_m'] - b['maximum_surface_displacement_m']))
    for x, y in zip(a['regions'], b['regions'], strict=True):
        left, right = np.asarray(x['nodes_world_m']), np.asarray(y['nodes_world_m'])
        assert left.shape == right.shape
        errors.extend(np.linalg.norm(left-right, axis=1))
        anchors.append(np.linalg.norm(np.asarray(x['anchor_world_m'])-y['anchor_world_m']))
        rotations.append(np.linalg.norm(np.asarray(x['anchor_rotation_xyzw'])-y['anchor_rotation_xyzw']))
report = {
    'fixed_reference': reference, 'fixed_current': current,
    'live_before': live('lab-boundary-energy-cache-live120-01', packets)[0],
    'live_current': live('lab-boundary-'+label+'-live120-01', packets)[0],
    'accuracy': {'frames': len(old), 'node_samples': len(errors),
        'maximum_node_error_m': float(max(errors)),
        'rms_node_error_m': float(np.sqrt(np.mean(np.asarray(errors)**2))),
        'maximum_attachment_translation_error_m': float(max(anchors)),
        'maximum_attachment_quaternion_chord_error': float(max(rotations)),
        'maximum_surface_envelope_error_m': max(envelopes),
        'maximum_allowed_node_error_m': MAXIMUM_NODE_ERROR_M},
    'limits': ['This comparison allows bounded numerical error; it does not prove bitwise equivalence.',
        'The reference uses the preceding strict solver, not an exact continuous solution.',
        'Live captures differ in source timestamps and cannot establish trajectory equivalence.',
        'Single shared-host timing runs; readbacks are not browser presentations.',
        'The jump contains no active contacts.'],
}
(OUT/'accuracy-comparison.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report, indent=2))
assert max(errors) < MAXIMUM_NODE_ERROR_M
assert max(envelopes) < MAXIMUM_NODE_ERROR_M
assert max(anchors) == 0.0 and max(rotations) == 0.0, 'Attachment sampling changed'
