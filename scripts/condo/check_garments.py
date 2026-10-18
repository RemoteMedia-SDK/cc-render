"""Read-only audit of the garment GLBs split from a condo outfit GLB.

    python scripts/condo/check_garments.py \
        ../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb \
        scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb \
        scenes/toronto_condo/garments report.json \
        [--khronos-validator DIR]

For each garment it checks (a) GLB/glTF structure, (b) byte identity of every
mesh attribute and index with the outfit's garment, joints compared by name,
(c) joint names, inverse bind matrices and bone rest transforms against both the
outfit and the clothing-free avatar, (d) bind-pose skinned world positions in
both files, (e) that the original avatar plus these garments accounts for every
entry and byte the outfit added, plus the sidecar, determinism and layer order.
``--khronos-validator DIR`` additionally runs the Khronos glTF validator from
``DIR/node_modules/gltf-validator`` (``npm install gltf-validator``) under node.
Exits non-zero on the first failed check.
"""
import argparse
import hashlib
from io import BytesIO
import json
from pathlib import Path
import struct
import subprocess
import sys

import numpy as np
from PIL import Image

from outfit_package import read_glb, verify_preservation
from split_outfit import COLUMNS, COMPONENT, GARMENTS, accessor_array, accessor_bytes, extract_garment, parents_of

BIND_TOLERANCE_M = 1e-6
ARRAYS = ['accessors', 'animations', 'bufferViews', 'cameras', 'images', 'materials', 'meshes',
          'nodes', 'samplers', 'scenes', 'skins', 'textures']


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def quaternion_matrix(x, y, z, w):
    return np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                     [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                     [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])


def local_matrix(node):
    if 'matrix' in node:
        return np.array(node['matrix'], dtype=np.float64).reshape(4, 4).T
    matrix = np.eye(4)
    matrix[:3, :3] = quaternion_matrix(*node.get('rotation', [0, 0, 0, 1])) * np.array(node.get('scale', [1, 1, 1]))
    matrix[:3, 3] = node.get('translation', [0, 0, 0])
    return matrix


def world_matrices(document):
    nodes, parent, world = document['nodes'], parents_of(document['nodes']), {}

    def solve(index):
        if index not in world:
            local = local_matrix(nodes[index])
            world[index] = solve(parent[index]) @ local if index in parent else local
        return world[index]
    for index in range(len(nodes)):
        solve(index)
    return world


def bind_pose(document, binary, node_index):
    """World-space bind-pose positions and normals from joint transforms and skinning."""
    node = document['nodes'][node_index]
    skin = document['skins'][node['skin']]
    world = world_matrices(document)
    ibm = accessor_array(document, binary, skin['inverseBindMatrices']).astype(np.float64)
    joint_matrices = np.stack([world[j] for j in skin['joints']]) @ ibm.reshape(-1, 4, 4).transpose(0, 2, 1)
    positions, normals, unskinned = [], [], []
    for primitive in document['meshes'][node['mesh']]['primitives']:
        attrs = primitive['attributes']
        p = accessor_array(document, binary, attrs['POSITION']).astype(np.float64)
        n = accessor_array(document, binary, attrs['NORMAL']).astype(np.float64)
        j = accessor_array(document, binary, attrs['JOINTS_0']).astype(np.int64)
        w = accessor_array(document, binary, attrs['WEIGHTS_0']).astype(np.float64)
        blended = np.einsum('vk,vkij->vij', w, joint_matrices[j])
        positions.append(np.einsum('vij,vj->vi', blended[:, :3, :3], p) + blended[:, :3, 3])
        skinned_normals = np.einsum('vij,vj->vi', blended[:, :3, :3], n)
        normals.append(skinned_normals / np.linalg.norm(skinned_normals, axis=1, keepdims=True))
        unskinned.append(p @ world[node_index][:3, :3].T + world[node_index][:3, 3])
    return np.concatenate(positions), np.concatenate(normals), np.concatenate(unskinned)


def check_structure(document, binary, raw):
    """Careful structural glTF check of a garment GLB (independent of the Khronos validator)."""
    magic, version, length = struct.unpack_from('<4sII', raw)
    check((magic, version, length) == (b'glTF', 2, len(raw)), 'Bad GLB header')
    check(document['asset']['version'] == '2.0', 'Not glTF 2.0')
    check(not document.get('extensionsUsed') and not document.get('extensionsRequired'), 'Unexpected extensions')
    check(len(document['buffers']) == 1 and 'uri' not in document['buffers'][0], 'Expected one GLB buffer')
    check(document['buffers'][0]['byteLength'] <= len(binary) < document['buffers'][0]['byteLength'] + 4,
          'Buffer length does not match BIN chunk')
    size = lambda key: len(document.get(key, []))
    for view in document['bufferViews']:
        check(view['buffer'] == 0 and view['byteOffset'] % 4 == 0, 'Misaligned buffer view')
        check(view['byteOffset'] + view['byteLength'] <= document['buffers'][0]['byteLength'], 'View overruns buffer')
    for index, spec in enumerate(document['accessors']):
        check(spec['bufferView'] < size('bufferViews'), f'Accessor {index} view out of range')
        component = np.dtype(COMPONENT[spec['componentType']]).itemsize
        check(spec.get('byteOffset', 0) % component == 0, f'Accessor {index} misaligned')
        values = accessor_array(document, binary, index)  # also checks bounds
        if 'min' in spec:
            check(values.min(axis=0).tolist() == spec['min'] and values.max(axis=0).tolist() == spec['max'],
                  f'Accessor {index} min/max mismatch')
    check(size('meshes') == 1 and size('skins') == 1 and size('scenes') == 1, 'Expected one mesh, skin and scene')
    check(not document.get('animations'), 'Garment must not carry animations')
    nodes = document['nodes']
    parent = parents_of(nodes)  # raises on a second parent
    check(all(c < len(nodes) for n in nodes for c in n.get('children', [])), 'Child out of range')
    roots = document['scenes'][document['scene']]['nodes']
    check(sorted(roots) == [i for i in range(len(nodes)) if i not in parent], 'Scene roots are not the tree roots')
    world_matrices(document)  # recursion terminates only for an acyclic tree
    mesh_nodes = [i for i, n in enumerate(nodes) if 'mesh' in n]
    check(len(mesh_nodes) == 1 and nodes[mesh_nodes[0]].get('skin') == 0, 'Expected one skinned mesh node')
    skin = document['skins'][0]
    check(document['accessors'][skin['inverseBindMatrices']]['count'] == len(skin['joints']), 'IBM count')
    check(len(set(skin['joints'])) == len(skin['joints']) and max(skin['joints']) < len(nodes), 'Bad skin joints')
    ancestors = lambda j: {j} | (ancestors(parent[j]) if j in parent else set())
    check(all(skin['skeleton'] in ancestors(j) for j in skin['joints']), 'Skeleton is not a common root')
    for primitive in document['meshes'][0]['primitives']:
        attrs = primitive['attributes']
        count = {document['accessors'][a]['count'] for a in attrs.values()}
        check(len(count) == 1, 'Attribute counts differ')
        joints, weights = (accessor_array(document, binary, attrs[k]) for k in ('JOINTS_0', 'WEIGHTS_0'))
        check(joints.max() < len(skin['joints']), 'Joint index out of range')
        check(np.isfinite(weights).all() and (weights >= 0).all(), 'Bad weights')
        check(abs(weights.sum(axis=1) - 1).max() < 1e-6, 'Weights not normalized')
        check(accessor_array(document, binary, primitive['indices']).max() < count.pop(), 'Index out of range')
        check(primitive['material'] < size('materials'), 'Material out of range')
    for texture in document.get('textures', []):
        check(texture['source'] < size('images') and texture.get('sampler', 0) < max(1, size('samplers')),
              'Texture reference out of range')
    for image in document.get('images', []):
        view = document['bufferViews'][image['bufferView']]
        with Image.open(BytesIO(binary[view['byteOffset']:view['byteOffset'] + view['byteLength']])) as decoded:
            decoded.load()
            check(decoded.format.lower() == image['mimeType'].split('/')[1], 'Image MIME mismatch')
    return {'nodes': len(nodes), 'joints': len(skin['joints']), 'images': size('images'),
            'max_nonzero_influences': max(int((accessor_array(document, binary, p['attributes']['WEIGHTS_0']) > 0)
                                              .sum(axis=1).max()) for p in document['meshes'][0]['primitives'])}


def khronos(directory, paths):
    script = ("const fs=require('fs'),p=require('path'),v=require(p.join(process.argv[1],"
              "'node_modules/gltf-validator'));(async()=>{for(const f of process.argv.slice(2)){"
              "const r=await v.validateBytes(new Uint8Array(fs.readFileSync(f)),{maxIssues:0});const c={};"
              "for(const m of r.issues.messages)c[m.code]=(c[m.code]||0)+1;console.log(JSON.stringify({"
              "validator:r.validatorVersion,errors:r.issues.numErrors,warnings:r.issues.numWarnings,"
              "infos:r.issues.numInfos,hints:r.issues.numHints,codes:c}))}})()")
    result = subprocess.run(['node', '-e', script, str(directory), *map(str, paths)],
                            check=True, capture_output=True, text=True)
    return [json.loads(line) for line in result.stdout.splitlines()]


def material_without_textures(material):
    material = json.loads(json.dumps(material))
    for holder in (material.get('pbrMetallicRoughness', {}), material):
        for value in holder.values():
            if isinstance(value, dict):
                value.pop('index', None)
    return material


def view_bytes(document, binary, index):
    view = document['bufferViews'][index]
    return binary[view.get('byteOffset', 0):view.get('byteOffset', 0) + view['byteLength']]


def check_garment(outfit, outfit_binary, avatar, avatar_binary, garment_doc, garment_binary, source_node):
    """Checks (b)-(d) for one garment; returns the numbers."""
    names = lambda document, indices: [document['nodes'][i]['name'] for i in indices]
    o_node, g_node = outfit['nodes'][source_node], next(n for n in garment_doc['nodes'] if 'mesh' in n)
    o_skin, g_skin = outfit['skins'][o_node['skin']], garment_doc['skins'][0]
    o_prims, g_prims = outfit['meshes'][o_node['mesh']]['primitives'], garment_doc['meshes'][0]['primitives']
    check(len(o_prims) == len(g_prims), 'Primitive count differs')
    o_joint_names = np.array(names(outfit, o_skin['joints']))
    g_joint_names = np.array(names(garment_doc, g_skin['joints']))
    compared = {}
    for o_prim, g_prim in zip(o_prims, g_prims):
        check(set(o_prim['attributes']) == set(g_prim['attributes']), 'Attribute set differs')
        for name, o_index in [*o_prim['attributes'].items(), ('indices', o_prim['indices'])]:
            g_index = g_prim['attributes'].get(name, g_prim.get('indices'))
            o_spec, g_spec = outfit['accessors'][o_index], garment_doc['accessors'][g_index]
            check(all(o_spec[k] == g_spec[k] for k in ('componentType', 'count', 'type')), f'{name} layout differs')
            if name.startswith('JOINTS_'):
                o_values = o_joint_names[accessor_array(outfit, outfit_binary, o_index)]
                g_values = g_joint_names[accessor_array(garment_doc, garment_binary, g_index)]
                check((o_values == g_values).all(), f'{name} differs after mapping joints by name')
                compared[name] = 'equal by joint name'
            else:
                check(accessor_bytes(outfit, outfit_binary, o_index)
                      == accessor_bytes(garment_doc, garment_binary, g_index), f'{name} bytes differ')
                check(o_spec.get('min') == g_spec.get('min') and o_spec.get('max') == g_spec.get('max'),
                      f'{name} bounds differ')
                compared[name] = 'byte-identical'
        o_material = outfit['materials'][o_prim['material']]
        g_material = garment_doc['materials'][g_prim['material']]
        check(material_without_textures(o_material) == material_without_textures(g_material), 'Material differs')
        for slot_holder in ('pbrMetallicRoughness', None):
            o_holder = o_material.get(slot_holder, {}) if slot_holder else o_material
            g_holder = g_material.get(slot_holder, {}) if slot_holder else g_material
            for slot, info in o_holder.items():
                if isinstance(info, dict) and 'index' in info:
                    o_tex, g_tex = outfit['textures'][info['index']], garment_doc['textures'][g_holder[slot]['index']]
                    check(view_bytes(outfit, outfit_binary, outfit['images'][o_tex['source']]['bufferView'])
                          == view_bytes(garment_doc, garment_binary, garment_doc['images'][g_tex['source']]['bufferView']),
                          f'{slot} image bytes differ')
                    check(outfit['samplers'][o_tex['sampler']] == garment_doc['samplers'][g_tex['sampler']], 'Sampler')
                    compared[slot] = 'image byte-identical'

    # (c) joint names and inverse bind matrices, against the outfit and the clothing-free avatar.
    o_ibm = accessor_bytes(outfit, outfit_binary, o_skin['inverseBindMatrices'])
    g_ibm = accessor_bytes(garment_doc, garment_binary, g_skin['inverseBindMatrices'])
    o_position = {name: i for i, name in enumerate(o_joint_names)}
    check(sorted(o_position[n] for n in g_joint_names) == [o_position[n] for n in g_joint_names],
          'Joint order differs from outfit skin order')
    for i, name in enumerate(g_joint_names):
        check(g_ibm[64 * i:64 * i + 64] == o_ibm[64 * o_position[name]:64 * o_position[name] + 64], f'IBM {name}')
    a_body = next(n for n in avatar['nodes'] if n.get('name') == 'CC_Base_Body')
    a_skin = avatar['skins'][a_body['skin']]
    a_ibm = accessor_bytes(avatar, avatar_binary, a_skin['inverseBindMatrices'])
    a_position = {name: i for i, name in enumerate(names(avatar, a_skin['joints']))}
    for i, name in enumerate(g_joint_names):
        check(g_ibm[64 * i:64 * i + 64] == a_ibm[64 * a_position[name]:64 * a_position[name] + 64],
              f'IBM {name} differs from the clothing-free avatar')
    # Every garment node (bones and their ancestors) matches the same-named node and parent in both files.
    g_parent = parents_of(garment_doc['nodes'])
    for document in (outfit, avatar):
        by_name = {n.get('name'): i for i, n in enumerate(document['nodes'])}
        parent = parents_of(document['nodes'])
        for index, node in enumerate(garment_doc['nodes']):
            if 'mesh' in node and document is avatar:
                continue
            source = document['nodes'][by_name[node['name']]]
            check(all(node.get(k) == source.get(k) for k in ('translation', 'rotation', 'scale', 'matrix')),
                  f"Rest transform of {node['name']} differs")
            expected = document['nodes'][parent[by_name[node['name']]]]['name'] if by_name[node['name']] in parent else None
            check((garment_doc['nodes'][g_parent[index]]['name'] if index in g_parent else None) == expected,
                  f"Parent of {node['name']} differs")

    # (d) bind pose: world positions from node transforms plus skinning in both files.
    o_pos, o_nrm, o_unskinned = bind_pose(outfit, outfit_binary, source_node)
    g_pos, g_nrm, g_unskinned = bind_pose(garment_doc, garment_binary, garment_doc['nodes'].index(g_node))
    position_error = float(abs(o_pos - g_pos).max())
    check(position_error <= BIND_TOLERANCE_M, f'Bind pose moved by {position_error} m')
    return {
        'attributes': compared,
        'joints': len(g_joint_names),
        'joint_names_match_outfit_and_avatar': True,
        'inverse_bind_matrices_byte_identical_to_outfit_and_avatar': True,
        'bone_rest_transforms_identical': len(garment_doc['nodes']) - 1,
        'bind_pose_max_position_error_m': position_error,
        'bind_pose_max_normal_error': float(abs(o_nrm - g_nrm).max()),
        'unskinned_node_placement_max_error_m': float(abs(o_unskinned - g_unskinned).max()),
        'skinned_vs_node_placement_max_m': float(np.linalg.norm(g_pos - g_unskinned, axis=1).max()),
        'bind_pose_bounds_m': [g_pos.min(axis=0).round(6).tolist(), g_pos.max(axis=0).round(6).tolist()],
    }, g_pos


def check_accounting(avatar, avatar_binary, outfit, outfit_binary, garment_sources):
    """(e) The original avatar plus the garments' source entries is the whole outfit."""
    report = verify_preservation(avatar, avatar_binary, outfit, outfit_binary)
    added = {key: list(range(len(avatar.get(key, [])), len(outfit.get(key, [])))) for key in ARRAYS}
    claimed = {key: set() for key in ARRAYS}
    for node_index in garment_sources:
        node = outfit['nodes'][node_index]
        claimed['nodes'].add(node_index)
        claimed['meshes'].add(node['mesh'])
        for primitive in outfit['meshes'][node['mesh']]['primitives']:
            accessors = [*primitive['attributes'].values(), primitive['indices']]
            claimed['accessors'].update(accessors)
            claimed['bufferViews'].update(outfit['accessors'][a]['bufferView'] for a in accessors)
            material = outfit['materials'][primitive['material']]
            claimed['materials'].add(primitive['material'])
            for holder in (material.get('pbrMetallicRoughness', {}), material):
                for info in holder.values():
                    if isinstance(info, dict) and 'index' in info:
                        texture = outfit['textures'][info['index']]
                        claimed['textures'].add(info['index'])
                        claimed['images'].add(texture['source'])
                        claimed['samplers'].add(texture['sampler'])
                        claimed['bufferViews'].add(outfit['images'][texture['source']]['bufferView'])
        check(node['skin'] < len(avatar['skins']), 'Garment skin was added rather than reused')
    for key in ARRAYS:
        check(claimed[key] == set(added[key]),
              f'{key}: outfit added {sorted(set(added[key]) - claimed[key])} that no garment carries; '
              f'garments use {sorted(claimed[key] - set(added[key]))} that the outfit did not add')
    # Every appended BIN byte is either inside an added buffer view or zero alignment padding.
    covered = np.zeros(len(outfit_binary) - len(avatar_binary), dtype=bool)
    for index in added['bufferViews']:
        view = outfit['bufferViews'][index]
        start = view['byteOffset'] - len(avatar_binary)
        check(start >= 0, 'Added view points into original bytes')
        covered[start:start + view['byteLength']] = True
    padding = np.frombuffer(outfit_binary[len(avatar_binary):], dtype=np.uint8)[~covered]
    check(not padding.any(), 'Uncovered non-zero appended bytes')
    return {**report, 'added_entries': {k: len(v) for k, v in added.items() if v},
            'added_entries_carried_by_garments': True,
            'appended_binary_bytes': int(len(covered)), 'appended_bytes_in_garment_views': int(covered.sum()),
            'appended_zero_padding_bytes': int(len(padding)),
            'garment_skin_reused_from_avatar': sorted({outfit['nodes'][n]['skin'] for n in garment_sources})}


def check_layers(positions):
    """Top must lie outside the trousers wherever their heights overlap (sidecar layer order)."""
    top, bottom = positions['condo_knit_tank_top'], positions['condo_charcoal_trousers']
    up = 1  # glTF world space is +Y up
    low, high = max(top[:, up].min(), bottom[:, up].min()), min(top[:, up].max(), bottom[:, up].max())
    centre = np.concatenate([top, bottom])[:, [0, 2]].mean(axis=0)
    polar = lambda p: (np.hypot(*(p[:, [0, 2]] - centre).T), np.arctan2(*(p[:, [2, 0]] - centre[::-1]).T))
    (top_r, top_a), (bottom_r, bottom_a) = polar(top), polar(bottom)
    margins = []
    for z in np.arange(low, high, 0.01):
        in_top = (top[:, up] >= z) & (top[:, up] < z + 0.01)
        in_bottom = (bottom[:, up] >= z) & (bottom[:, up] < z + 0.01)
        for a in np.linspace(-np.pi, np.pi, 37)[:-1]:
            t = in_top & (top_a >= a) & (top_a < a + np.pi / 18)
            b = in_bottom & (bottom_a >= a) & (bottom_a < a + np.pi / 18)
            if t.any() and b.any():
                margins.append(top_r[t].max() - bottom_r[b].max())
    margins = np.array(margins)
    check(len(margins) > 0 and (margins > 0).all(), 'Top is not outside the trousers in the overlap band')
    return {'overlap_band_m': [round(float(low), 4), round(float(high), 4)], 'bins': len(margins),
            'top_outside_min_mm': round(float(margins.min()) * 1000, 2),
            'top_outside_median_mm': round(float(np.median(margins)) * 1000, 2)}


def check_garments(avatar_path, outfit_path, garments_dir, validator=None):
    avatar, avatar_binary = read_glb(avatar_path)
    outfit_raw = Path(outfit_path).read_bytes()
    outfit, outfit_binary = read_glb(outfit_path)
    outfit_sha256 = hashlib.sha256(outfit_raw).hexdigest()
    report = {'avatar_sha256': hashlib.sha256(Path(avatar_path).read_bytes()).hexdigest(),
              'outfit_sha256': outfit_sha256, 'garments': {}}
    positions, sources, paths = {}, [], []
    for garment in GARMENTS:
        folder = Path(garments_dir) / garment['id']
        path = folder / f"{garment['id']}.glb"
        raw = path.read_bytes()
        sidecar = json.loads((folder / 'garment.json').read_text())
        document, binary = read_glb(path)
        source_node = sidecar['provenance']['source_node_index']
        check(outfit['nodes'][source_node]['name'] == garment['source_node_name'], 'Sidecar source node mismatch')
        check(sidecar['provenance']['source_outfit_sha256'] == outfit_sha256, 'Sidecar outfit hash mismatch')
        check(sidecar['provenance']['source_avatar_sha256'] == report['avatar_sha256'], 'Sidecar avatar hash mismatch')
        check(sidecar['asset'] == {'glb': path.name, 'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest()},
              'Sidecar asset record mismatch')
        joint_names = [document['nodes'][j]['name'] for j in document['skins'][0]['joints']]
        check(sidecar['binding']['joints'] == joint_names, 'Sidecar joints mismatch')
        check(sidecar['binding']['joints_sha256'] == hashlib.sha256('\n'.join(joint_names).encode()).hexdigest(),
              'Sidecar joint hash mismatch')
        check({k: sidecar[k] for k in ('id', 'slot', 'layer', 'display_name')}
              == {k: garment[k] for k in ('id', 'slot', 'layer', 'display_name')}, 'Sidecar metadata mismatch')
        regenerated, _ = extract_garment(outfit, outfit_binary, source_node, garment, outfit_sha256)
        check(regenerated == raw, 'Re-running the splitter does not reproduce the committed GLB')
        structure = check_structure(document, binary, raw)
        result, positions[garment['id']] = check_garment(outfit, outfit_binary, avatar, avatar_binary,
                                                         document, binary, source_node)
        check(sidecar['geometry']['vertices'] == len(positions[garment['id']]), 'Sidecar vertex count')
        check(sidecar['geometry']['triangles'] == sum(document['accessors'][p['indices']]['count'] // 3
                                                     for p in document['meshes'][0]['primitives']), 'Sidecar triangles')
        report['garments'][garment['id']] = {'glb_bytes': len(raw), 'sha256': sidecar['asset']['sha256'],
                                             'vertices': sidecar['geometry']['vertices'],
                                             'triangles': sidecar['geometry']['triangles'],
                                             'structure': structure, 'deterministic_rebuild': True, **result}
        sources.append(source_node)
        paths.append(path)
    report['accounting'] = check_accounting(avatar, avatar_binary, outfit, outfit_binary, sources)
    report['layer_order'] = check_layers(positions)
    if validator:
        results = khronos(validator, paths)
        for path, result in zip(paths, results):
            check(result['errors'] == 0, f'Khronos validator errors in {path.name}: {result["codes"]}')
        report['khronos_validator'] = dict(zip([p.parent.name for p in paths], results))
    else:
        report['khronos_validator'] = 'not run (pass --khronos-validator DIR)'
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('avatar', type=Path, help='clothing-free source avatar GLB')
    parser.add_argument('outfit', type=Path, help='outfit GLB the garments were split from')
    parser.add_argument('garments', type=Path, help='garments directory')
    parser.add_argument('report', type=Path, nargs='?', help='optional JSON report path')
    parser.add_argument('--khronos-validator', type=Path, help='directory containing node_modules/gltf-validator')
    args = parser.parse_args()
    result = check_garments(args.avatar, args.outfit, args.garments, args.khronos_validator)
    text = json.dumps(result, indent=2) + '\n'
    if args.report:
        args.report.write_text(text)
    sys.stdout.write(text)
