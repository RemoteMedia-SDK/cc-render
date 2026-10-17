"""Read-only shoe audit using the SDK's calibrated FK/LBS mirror.

Diagnostic identity rotations are evaluated offline only. No pose, avatar,
scene package, calibration, or playback stream is modified.
"""
import argparse
import json
from pathlib import Path
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT.parent / 'remotemedia-sdk-office-main'
sys.path.insert(0, str(SDK / 'scripts/avatars'))
from heel_contact_foundation import Rig, ASSET, MAP, sha


def shoe_metrics(vertices, profile):
    return {'minimum_y_m': float(vertices[:, 1].min()),
            'outsole_centroid_y_m': {
                name: float(vertices[patch['vertex_ids'], 1].mean())
                for name, patch in profile['patches'].items()}}


def plot_comparison(rig, profile, variants, destination):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    from matplotlib.collections import LineCollection

    fig, axes = plt.subplots(1, 3, figsize=(15, 5), sharex=True, sharey=True)
    triangles = rig.meshes['High_Heels', 0][-1]
    for axis, (title, vertices) in zip(axes, variants):
        left = vertices[triangles].mean(1)[:, 0] > .03
        edges = vertices[triangles[left]][:, [0, 1, 2, 0]][:, :, [2, 1]]
        axis.add_collection(LineCollection(edges, colors='#59646b', linewidths=.3, alpha=.35))
        for name, colour in [('left_heel_tip', '#147d92'), ('left_forefoot', '#d57935')]:
            v = vertices[profile['patches'][name]['vertex_ids']]
            axis.scatter(v[:, 2], v[:, 1], s=9, c=colour, label=name.replace('left_', ''))
        axis.axhline(0, color='black', linewidth=.8)
        axis.set(title=title, xlabel='Forward Z (m)', xlim=(-.18, .24), ylim=(-.09, .25))
        axis.set_aspect('equal')
        axis.grid(alpha=.2)
    axes[0].set_ylabel('Height Y (m)')
    axes[0].legend(loc='upper left', fontsize=8)
    fig.suptitle('Unchanged beautiful_assistant shoe geometry — read-only FK/LBS audit')
    fig.text(.5, .02, 'Neutral calibration is an offline mathematical probe, not a generated pose or proposed animation.', ha='center', fontsize=10)
    fig.subplots_adjust(left=.06, right=.99, bottom=.20, top=.84, wspace=.08)
    fig.savefig(destination, dpi=140)
    plt.close(fig)


def compare_runtime(frames, capture):
    report = json.loads(capture.read_text())
    avatar = report['final_state']['office']['avatar']
    frame = next(row for row in frames if row['pts_ms'] == avatar['displayed_pts_ms'])
    error = float(np.max(np.abs(np.asarray(avatar['feet']) - frame['mapped_ankles_world_m'])))
    return {'capture': str(capture.resolve()), 'capture_sha256': sha(capture),
            'pts_ms': frame['pts_ms'], 'max_ankle_coordinate_error_m': error,
            'within_5_micrometres': error < 5e-6,
            'scope': 'Final observed ankle positions only; no GPU vertex readback or contact acceptance.'}


def audit(packets, destination, capture=None):
    destination.mkdir(parents=True, exist_ok=False)
    rig = Rig(ASSET)
    profile = rig.profile()
    inputs = [ASSET, MAP, packets, Path(__file__),
              SDK / 'scripts/avatars/heel_contact_foundation.py',
              SDK / 'scripts/avatars/ardy_adapter.py',
              ROOT / 'src/cc_render/bevy_app/pose.rs']
    if capture:
        inputs.append(capture)
    hashes = {str(p.resolve()): sha(p) for p in inputs}
    source = [json.loads(line) for line in packets.read_text().splitlines()]
    bind_pelvis = rig.rest[rig.mapped[0]][:3, 3]
    bind = rig.skin(('High_Heels', 0), rig.rest)
    neutral_world = rig.retarget(np.tile([0., 0., 0., 1.], (22, 1)), bind_pelvis)
    neutral = rig.skin(('High_Heels', 0), neutral_world)
    frames = []
    sample = None
    for packet in source:
        world = rig.retarget(np.asarray(packet['joint_quats_xyzw']), np.asarray(packet['root_pos']))
        vertices = rig.skin(('High_Heels', 0), world)
        frames.append({'pts_ms': packet['pts_ms'], 'root_y_m': packet['root_pos'][1],
                       'mapped_ankles_world_m': [world[rig.mapped[i]][:3, 3].tolist() for i in (7, 8)],
                       **shoe_metrics(vertices, profile)})
        if packet['pts_ms'] == 2500:
            sample = vertices
    if sample is None:
        raise ValueError('Expected the unchanged 2500 ms packet for the capture comparison')
    bone_pitch = []
    for ankle, toe in [(7, 10), (8, 11)]:
        vector = rig.rest[rig.mapped[toe]][:3, 3] - rig.rest[rig.mapped[ankle]][:3, 3]
        bone_pitch.append(float(np.degrees(np.arctan2(vector[1], np.hypot(vector[0], vector[2])))))
    result = {'input_sha256': hashes, 'source_packets': len(source), 'bind_pelvis_y_m': float(bind_pelvis[1]),
              'bind_ankle_to_toe_pitch_degrees': bone_pitch,
              'bind': shoe_metrics(bind, profile), 'neutral_calibration': shoe_metrics(neutral, profile),
              'frames': frames,
              'limits': ['CPU mirror of default calibrated FK and normalized four-weight skinning; validate against runtime observations.',
                         'Geometry heights are not force/contact evidence. No floor snap, pose correction, asset edit or new motion generation.']}
    assert hashes == {str(p.resolve()): sha(p) for p in inputs}, 'An audited input changed'
    result['inputs_unchanged'] = True
    if capture:
        result['runtime_comparison'] = compare_runtime(frames, capture)
    (destination / 'foot-audit.json').write_text(json.dumps(result, indent=2) + '\n')
    (destination / 'shoe-profile.json').write_text(json.dumps(profile, indent=2) + '\n')
    plot_comparison(rig, profile, [('Original bind', bind), ('Current neutral calibration', neutral),
                                  ('Unchanged native packet at 2.5 s', sample)], destination / 'shoe-calibration-comparison.png')
    print(json.dumps({k: result[k] for k in ['source_packets', 'bind_pelvis_y_m', 'bind_ankle_to_toe_pitch_degrees', 'bind', 'neutral_calibration', 'inputs_unchanged']}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('packets', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--capture', type=Path, help='Compare final runtime ankles at an exact source timestamp')
    args = parser.parse_args()
    audit(args.packets, args.destination, args.capture)
