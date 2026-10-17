"""Bind an additive outfit to independently checked, read-only FK/skin geometry.

Keeps the original contact calibration only after byte-preservation and numeric
parity checks. Exports the actual extended hierarchy and both garment surfaces.
No motion, avatar files or input profiles are modified.
"""
import argparse
import json
from pathlib import Path
import shutil
import sys

import numpy as np
from scipy.spatial.transform import Rotation

from check_outfit import check_outfit

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT.parent / 'remotemedia-sdk-office-main'
sys.path.insert(0, str(SDK / 'scripts/avatars'))
from heel_contact_foundation import Rig, sha
from ardy_adapter import CORE_NAMES, SMPL_PARENTS
from scene_conditioning.profile import AvatarProfile
from motion_service import digest
from export_renderer_calibration import export


def skin_arrays(rig, node, primitive):
    skin = rig.g.skins[node.skin]
    positions = rig.accessor(primitive.attributes.POSITION).astype(float)
    joints = rig.accessor(primitive.attributes.JOINTS_0).astype(int)
    weights = rig.accessor(primitive.attributes.WEIGHTS_0).astype(float)
    assert np.isfinite(weights).all() and (weights >= 0).all() and (weights.sum(1) > 0).all()
    weights /= weights.sum(1)[:, None]
    inverse = rig.accessor(skin.inverseBindMatrices).reshape(-1, 4, 4).transpose(0, 2, 1)
    weighted = np.einsum('vnij,vj->vni', inverse[joints], np.c_[positions, np.ones(len(positions))])
    return np.array(skin.joints)[joints], weighted * weights[:, :, None]


def export_arrays(rig, base, bind):
    arrays = dict(local=np.asarray(rig.local), rest=np.asarray(rig.rest),
                  parents=np.array([rig.parents.get(i, -1) for i in range(len(rig.local))]),
                  mapped=np.array(rig.mapped), smpl_parents=np.array(SMPL_PARENTS))
    names = bind['rig_joint_names'].tolist()
    indices = np.array([names.index(name) for name in CORE_NAMES])
    arrays['core_indices'] = indices
    arrays['core_rest_inverse'] = np.linalg.inv(bind['bind_rig_transform'][indices, :3, :3])
    world = rig.retarget(np.tile([0., 0., 0., 1.], (22, 1)), rig.rest[rig.mapped[0]][:3, 3])
    rot = lambda m: Rotation.from_matrix(m[:3, :3]).as_matrix()
    arrays['neutral'] = np.array([rot(world[n]) for n in rig.mapped])
    parents = [rot(rig.rest[rig.parents[n]]) for n in rig.mapped]
    arrays['inter'] = np.array([np.eye(3) if p < 0 else rot(rig.rest[rig.mapped[p]]).T @ parents[i]
                               for i, p in enumerate(SMPL_PARENTS)])
    arrays['pelvis_parent'] = parents[0]
    surfaces = list(rig.meshes)
    surfaces += [(rig.g.nodes[i].name, j) for i in range(len(base['local']), len(rig.local))
                 for j in range(len(rig.g.meshes[rig.g.nodes[i].mesh].primitives))]
    for i, (name, primitive) in enumerate(surfaces):
        node = rig.g.nodes[rig.nodes[name]]
        bones, weighted = skin_arrays(rig, node, rig.g.meshes[node.mesh].primitives[primitive])
        arrays[f'mesh_{i}_bones'], arrays[f'mesh_{i}_local_weighted'] = bones, weighted
    for key, previous in base.items():
        actual = arrays[key][:len(previous)] if key in ('local', 'rest', 'parents') else arrays[key]
        assert np.allclose(actual, previous, rtol=0, atol=1e-12), key
    return arrays, surfaces


def package(source, avatar, variant, core_bind, destination):
    if destination.exists():
        raise ValueError('Output must be a new directory')
    preservation = check_outfit(avatar, variant)
    manifest = json.loads((source / 'profile.json').read_text())
    profile = AvatarProfile.load(source / 'profile.json', avatar, manifest['core_bind_id'])
    rig = Rig(variant)
    with np.load(core_bind, allow_pickle=False) as bind:
        assert digest(bind['bind_rig_transform']) == profile.core_bind_id
        arrays, surfaces = export_arrays(rig, profile.arrays('geometry'), bind)
    # Retained seat patches must be actual vertices of the unchanged body skin.
    body = {b.tobytes()+w.tobytes() for i in range(1, 7)
            for b, w in zip(arrays[f'mesh_{i}_bones'], arrays[f'mesh_{i}_local_weighted'])}
    patches = profile.arrays('seat_patches')
    for side in ('left', 'right'):
        assert all(b.tobytes()+w.tobytes() in body
                   for b, w in zip(patches[side+'_bones'], patches[side+'_weighted']))
    destination.mkdir(parents=True, exist_ok=False)
    np.savez_compressed(destination / 'geometry.npz', **arrays)
    shutil.copy2(profile.seat_patches, destination / 'seat-patches.npz')
    manifest['avatar_sha256'] = sha(variant)
    manifest['files']['geometry']['sha256'] = sha(destination / 'geometry.npz')
    output = destination / 'profile.json'
    output.write_text(json.dumps(manifest, indent=2)+'\n')
    bound = AvatarProfile.load(output, variant, manifest['core_bind_id'])
    export(bound, destination / 'renderer-geometry.json')
    evidence = dict(scope=__doc__, preservation=preservation, profile_identity=bound.identity,
                    hierarchy_nodes=len(rig.local), surfaces=surfaces, original_arrays_verified=True,
                    seat_patch_vertices_verified=True, inputs={str(p.resolve()):sha(p) for p in
                    (avatar, variant, core_bind, source/'profile.json', profile.geometry, profile.seat_patches,
                     Path(__file__), SDK/'scripts/avatars/heel_contact_foundation.py')},
                    limits='Body, heels and added garments only; other existing meshes and morph deformation are excluded. '
                           'Static calibration is not manipulation, physical support or live playback certification.')
    (destination/'export-evidence.json').write_text(json.dumps(evidence, indent=2)+'\n')
    print(json.dumps({'profile':str(output), 'identity':bound.identity, 'surfaces':len(surfaces)}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'avatar', 'variant', 'core_bind', 'destination'):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    package(args.source, args.avatar, args.variant, args.core_bind, args.destination)
