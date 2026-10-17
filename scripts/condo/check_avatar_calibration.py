"""Compare a bound SDK profile with calibrated FK using unchanged native packets."""
import argparse
import json
from pathlib import Path
import sys

import numpy as np
import torch

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT.parent / 'remotemedia-sdk-office-main'
sys.path.insert(0, str(SDK / 'scripts/avatars'))
from heel_contact_foundation import Rig, ASSET, sha
from scene_conditioning.profile import AvatarProfile
from scene_conditioning.avatar_geometry import AvatarGeometry


def check(profile_path, packets_path, output):
    output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads(profile_path.read_text())
    profile = AvatarProfile.load(profile_path, ASSET, manifest['core_bind_id'])
    geometry = AvatarGeometry(profile, dtype=torch.float64)
    packets = [json.loads(line) for line in packets_path.read_text().splitlines()]
    rotations = torch.tensor([p['source_global_rot_mats'] for p in packets], dtype=torch.float64)
    roots = torch.tensor([p['root_pos'] for p in packets], dtype=torch.float64)
    with torch.inference_mode():
        world = geometry.world(rotations, roots).numpy()
    rig = Rig(ASSET)
    patches = profile.arrays('seat_patches')
    maximum = 0.
    fixtures = []
    for i, packet in enumerate(packets):
        mirrored = np.asarray(rig.retarget(np.asarray(packet['joint_quats_xyzw']), np.asarray(packet['root_pos'])))
        maximum = max(maximum, float(np.max(np.abs(world[i] - mirrored))))
        assert np.max(np.abs(world[i, rig.mapped[0], :3, 3] - packet['root_pos'])) < 1e-9
        if i % 50 == 0 or i + 1 == len(packets):
            seat_minima = [float(np.einsum('vnij,vnj->vi', world[i, patches[side + '_bones']],
                                          patches[side + '_weighted'])[:, 1].min())
                           for side in ('left', 'right')]
            fixtures.append({'quaternions': packet['joint_quats_xyzw'], 'root': packet['root_pos'],
                             'seat_minima': seat_minima,
                             'world': world[i].transpose(0, 2, 1).reshape(-1, 16).tolist()})
    assert maximum < 5e-6, maximum
    asset = profile_path.parent / 'renderer-geometry.json'
    fixture = {'asset': str(asset.resolve()), 'avatar': profile.avatar_sha256,
               'profile': profile.identity, 'frames': fixtures}
    (output / 'renderer-parity-fixture.json').write_text(json.dumps(fixture) + '\n')
    result = {'packets': len(packets), 'maximum_world_matrix_difference': maximum,
              'root_coordinates_preserved': True, 'avatar_sha256': sha(ASSET),
              'packets_sha256': sha(packets_path), 'profile_identity': profile.identity,
              'renderer_geometry_sha256': sha(asset), 'rust_fixture_frames': len(fixtures)}
    (output / 'profile-parity.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('profile', type=Path)
    parser.add_argument('packets', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    check(args.profile, args.packets, args.output)
