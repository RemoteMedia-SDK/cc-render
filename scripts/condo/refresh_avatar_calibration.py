"""Stage the canonical avatar's fixed foot-reference calibration and fixture.

Preserves all geometry, skin weights, source motion and root coordinates.
Writes a new profile directory; never overwrites the input profile or avatar.
"""
import argparse
import json
from pathlib import Path
import shutil
import sys

import numpy as np
from scipy.spatial.transform import Rotation

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT.parent / 'remotemedia-sdk-office-main'
sys.path.insert(0, str(SDK / 'scripts/avatars'))
from heel_contact_foundation import Rig, ASSET, sha
from scene_conditioning.profile import AvatarProfile
from export_renderer_calibration import export


def refresh(source, destination):
    manifest = json.loads((source / 'profile.json').read_text())
    profile = AvatarProfile.load(source / 'profile.json', ASSET, manifest['core_bind_id'])
    arrays = profile.arrays('geometry')
    rig = Rig(ASSET)
    assert np.array_equal(arrays['mapped'], rig.mapped)
    assert np.allclose(arrays['local'], rig.local, atol=1e-10, rtol=0)
    before = arrays['neutral'].copy()
    world = rig.retarget(np.tile([0., 0., 0., 1.], (22, 1)), rig.rest[rig.mapped[0]][:3, 3])
    arrays['neutral'] = np.asarray([Rotation.from_matrix(world[node][:3, :3]).as_matrix()
                                    for node in rig.mapped])
    changed = np.where(np.max(np.abs(before - arrays['neutral']), axis=(1, 2)) > 1e-8)[0].tolist()
    assert changed == [7, 8, 10, 11], changed
    destination.mkdir(parents=True, exist_ok=False)
    np.savez_compressed(destination / 'geometry.npz', **arrays)
    shutil.copy2(profile.seat_patches, destination / 'seat-patches.npz')
    manifest['files']['geometry']['sha256'] = sha(destination / 'geometry.npz')
    (destination / 'profile.json').write_text(json.dumps(manifest, indent=2) + '\n')
    updated = AvatarProfile.load(destination / 'profile.json', ASSET, manifest['core_bind_id'])
    export(updated, destination / 'renderer-geometry.json')
    fixture_path = ROOT / 'tests/fixtures/cc5-scene-fit-calibration.json'
    fixture = json.loads(fixture_path.read_text())
    for case in fixture['cases']:
        posed = rig.retarget(np.asarray(case['quats']), np.asarray(case['root']))
        case['expected_local'] = [Rotation.from_matrix(
            np.linalg.inv(posed[rig.parents[node]])[:3, :3] @ posed[node][:3, :3]).as_quat().tolist()
            for node in rig.mapped]
        realized = posed[rig.mapped[0]][:3, 3]
        assert np.allclose(realized, case['realized_root'], atol=1e-10, rtol=0)
    (destination / fixture_path.name).write_text(json.dumps(fixture, indent=2) + '\n')
    # Independently check every retained array and content-bound profile after encoding.
    decoded = updated.arrays('geometry')
    for key, value in profile.arrays('geometry').items():
        if key != 'neutral':
            assert np.array_equal(value, decoded[key]), key
    evidence = {'avatar_sha256': sha(ASSET), 'old_profile_identity': profile.identity,
                'new_profile_identity': updated.identity, 'changed_neutral_bones': changed,
                'all_other_geometry_arrays_identical': True, 'fixture_root_and_motion_inputs_preserved': True,
                'source_profile': str(source.resolve()), 'source_fixture_sha256': sha(fixture_path)}
    (destination / 'refresh-evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps(evidence, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    args = parser.parse_args()
    refresh(args.source, args.destination)
