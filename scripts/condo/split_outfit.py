"""Split the garments appended to a CC5 outfit GLB into one standalone garment GLB each.

This is glTF-level surgery, not a re-export: every vertex attribute, index
buffer, inverse bind matrix and embedded image is copied byte for byte from the
outfit GLB. The only re-encoded data is JOINTS_0, whose indices are remapped
from the outfit's 78-joint ``CC_Base_Body`` skin to the garment's own skin,
which lists only the joints the garment references (same names, same inverse
bind matrices, same order). The garment GLB also carries the CC5 bone nodes on
the path from the scene root to those joints, with their unchanged rest
transforms, so it is a valid glTF skin whose bind pose places every vertex where
the outfit places it. A loader binds the garment to an avatar by joint name.

    python scripts/condo/split_outfit.py \
        scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb \
        scenes/toronto_condo/garments

The output is deterministic. Re-running it on the same outfit is a no-op;
an existing file with different content is refused unless ``--overwrite``.
``check_garments.py`` independently verifies the result.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys

import numpy as np

from outfit_package import read_glb

SCRIPT = 'scripts/condo/split_outfit.py'
REPO = Path(__file__).resolve().parents[2]
SCHEMA = 'cc-render.garment/0'

# Display metadata is authored here; everything else is derived from the GLB.
# Layer is a stacking ordinal (higher is worn over lower). The top's hem
# overlaps the trouser waistband and lies outside it in every sampled bin of
# the overlap band (see check_garments.py), so the top is layer 1.
GARMENTS = [
    {'id': 'condo_knit_tank_top', 'source_node_name': 'Condo cream knit top',
     'display_name': 'Cream sleeveless knit top', 'slot': 'top', 'layer': 1},
    {'id': 'condo_charcoal_trousers', 'source_node_name': 'Condo charcoal trousers',
     'display_name': 'Charcoal trousers', 'slot': 'bottom', 'layer': 0},
]

COMPONENT = {5120: 'i1', 5121: 'u1', 5122: '<i2', 5123: '<u2', 5125: '<u4', 5126: '<f4'}
COLUMNS = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}
TEXTURE_SLOTS = ['baseColorTexture', 'metallicRoughnessTexture']
MATERIAL_TEXTURE_SLOTS = ['normalTexture', 'occlusionTexture', 'emissiveTexture']
TRS = ['translation', 'rotation', 'scale', 'matrix']


def accessor_bytes(document, binary, index):
    """Raw, tightly packed bytes of a non-sparse embedded accessor."""
    spec = document['accessors'][index]
    view = document['bufferViews'][spec['bufferView']]
    item = np.dtype(COMPONENT[spec['componentType']]).itemsize * COLUMNS[spec['type']]
    if spec.get('sparse') or view.get('buffer', 0) != 0 or view.get('byteStride', item) != item:
        raise ValueError(f'Accessor {index} is sparse, external or interleaved')
    start = view.get('byteOffset', 0) + spec.get('byteOffset', 0)
    end = start + spec['count'] * item
    if end > view.get('byteOffset', 0) + view['byteLength']:
        raise ValueError(f'Accessor {index} overruns its buffer view')
    return binary[start:end]


def accessor_array(document, binary, index):
    spec = document['accessors'][index]
    return np.frombuffer(accessor_bytes(document, binary, index),
                         dtype=COMPONENT[spec['componentType']]).reshape(-1, COLUMNS[spec['type']])


def parents_of(nodes):
    parent = {}
    for index, node in enumerate(nodes):
        for child in node.get('children', []):
            if child in parent:
                raise ValueError(f'Node {child} has two parents')
            parent[child] = index
    return parent


def with_ancestors(nodes, seeds):
    parent, kept = parents_of(nodes), set()
    for node in seeds:
        while node is not None and node not in kept:
            kept.add(node)
            node = parent.get(node)
    return kept


def common_root(nodes, joints):
    """Deepest node that is a joint or an ancestor of every joint."""
    parent = parents_of(nodes)

    def chain(node):
        path = [node]
        while path[-1] in parent:
            path.append(parent[path[-1]])
        return path[::-1]
    chains = [chain(j) for j in joints]
    root = None
    for level in zip(*chains):
        if len(set(level)) != 1:
            break
        root = level[0]
    if root is None:
        raise ValueError('Joints do not share a root')
    return root


class GarmentWriter:
    """Accumulates the JSON and BIN chunk of one garment GLB."""

    def __init__(self, source, source_binary):
        self.source, self.source_binary = source, source_binary
        self.document = {'asset': {}, 'scene': 0, 'scenes': [], 'nodes': [], 'meshes': [],
                         'skins': [], 'materials': [], 'textures': [], 'images': [],
                         'samplers': [], 'accessors': [], 'bufferViews': [], 'buffers': []}
        self.binary = bytearray()
        self.copied_accessors, self.copied_textures = {}, {}
        self.copied_images, self.copied_samplers = {}, {}

    def view(self, data, target=None):
        self.binary.extend(b'\0' * (-len(self.binary) % 4))
        view = {'buffer': 0, 'byteOffset': len(self.binary), 'byteLength': len(data)}
        if target is not None:
            view['target'] = target
        self.binary.extend(data)
        self.document['bufferViews'].append(view)
        return len(self.document['bufferViews']) - 1

    def accessor(self, spec, data, target=None):
        spec = {key: value for key, value in spec.items() if key not in ('bufferView', 'byteOffset')}
        spec = {'bufferView': self.view(data, target), **spec}
        self.document['accessors'].append(spec)
        return len(self.document['accessors']) - 1

    def copy_accessor(self, index, target=None):
        """Byte-exact copy of an accessor, keeping its JSON (count, type, min/max...)."""
        if index not in self.copied_accessors:
            self.copied_accessors[index] = self.accessor(
                self.source['accessors'][index],
                accessor_bytes(self.source, self.source_binary, index), target)
        return self.copied_accessors[index]

    def remapped_joints(self, index, remap):
        spec = self.source['accessors'][index]
        values = remap[accessor_array(self.source, self.source_binary, index)]
        values = values.astype(COMPONENT[spec['componentType']])
        spec = {**spec, 'min': values.min(axis=0).tolist(), 'max': values.max(axis=0).tolist()}
        return self.accessor(spec, values.tobytes(), 34962)

    def copy_sampler(self, index):
        if index not in self.copied_samplers:
            self.document['samplers'].append(dict(self.source['samplers'][index]))
            self.copied_samplers[index] = len(self.document['samplers']) - 1
        return self.copied_samplers[index]

    def copy_image(self, index):
        if index not in self.copied_images:
            image = dict(self.source['images'][index])
            if 'uri' in image:
                raise ValueError(f'Image {index} is external')
            view = self.source['bufferViews'][image['bufferView']]
            start = view.get('byteOffset', 0)
            image['bufferView'] = self.view(self.source_binary[start:start + view['byteLength']])
            self.document['images'].append(image)
            self.copied_images[index] = len(self.document['images']) - 1
        return self.copied_images[index]

    def copy_texture(self, index):
        if index not in self.copied_textures:
            texture = dict(self.source['textures'][index])
            if texture.get('extensions'):
                raise ValueError(f'Texture {index} uses extensions')
            texture['source'] = self.copy_image(texture['source'])
            if 'sampler' in texture:
                texture['sampler'] = self.copy_sampler(texture['sampler'])
            self.document['textures'].append(texture)
            self.copied_textures[index] = len(self.document['textures']) - 1
        return self.copied_textures[index]

    def copy_material(self, index):
        material = json.loads(json.dumps(self.source['materials'][index]))
        if material.get('extensions'):
            raise ValueError(f'Material {index} uses extensions')
        holders = [(material.get('pbrMetallicRoughness', {}), TEXTURE_SLOTS),
                   (material, MATERIAL_TEXTURE_SLOTS)]
        for holder, slots in holders:
            for slot in slots:
                if slot in holder:
                    holder[slot]['index'] = self.copy_texture(holder[slot]['index'])
        self.document['materials'].append(material)
        return len(self.document['materials']) - 1

    def encode(self):
        self.document = {key: value for key, value in self.document.items() if value != []}
        self.binary.extend(b'\0' * (-len(self.binary) % 4))
        self.document['buffers'] = [{'byteLength': len(self.binary)}]
        encoded = json.dumps(self.document, separators=(',', ':')).encode()
        encoded += b' ' * (-len(encoded) % 4)
        return (struct.pack('<4sIIII', b'glTF', 2, 28 + len(encoded) + len(self.binary),
                            len(encoded), 0x4E4F534A)
                + encoded + struct.pack('<II', len(self.binary), 0x004E4942) + bytes(self.binary))


def material_facts(material):
    pbr = material.get('pbrMetallicRoughness', {})
    return {'name': material.get('name'), 'double_sided': material.get('doubleSided', False),
            'textures': [slot for holder, slots in [(pbr, TEXTURE_SLOTS), (material, MATERIAL_TEXTURE_SLOTS)]
                         for slot in slots if slot in holder]}


def extract_garment(source, source_binary, node_index, garment, source_sha256):
    """Return (glb_bytes, facts) for one skinned mesh node of the outfit."""
    nodes = source['nodes']
    node = nodes[node_index]
    mesh = source['meshes'][node['mesh']]
    skin = source['skins'][node['skin']]
    if 'weights' in mesh or any('targets' in p for p in mesh['primitives']):
        raise ValueError('Garment has morph targets; this splitter does not carry them')

    joint_sets = [name for p in mesh['primitives'] for name in p['attributes'] if name.startswith('JOINTS_')]
    referenced = sorted({int(v) for p in mesh['primitives'] for name in p['attributes']
                         if name.startswith('JOINTS_')
                         for v in np.unique(accessor_array(source, source_binary, p['attributes'][name]))})
    remap = np.full(len(skin['joints']), 0xFFFF, dtype=np.int64)
    remap[referenced] = np.arange(len(referenced))
    joint_nodes = [skin['joints'][i] for i in referenced]

    kept = sorted(with_ancestors(nodes, [node_index, *joint_nodes]))
    new_index = {old: new for new, old in enumerate(kept)}

    writer = GarmentWriter(source, source_binary)
    out = writer.document
    out['asset'] = {'version': '2.0', 'generator': f'cc-render {SCRIPT}',
                    'extras': {'garment_id': garment['id'], 'source_sha256': source_sha256,
                               'source_node': node_index, 'source_node_name': node['name']}}
    for old in kept:
        source_node = nodes[old]
        # Bones and their ancestors must be plain transforms; anything else would be lost.
        allowed = {'name', 'children', *TRS} | ({'mesh', 'skin'} if old == node_index else set())
        if set(source_node) - allowed:
            raise ValueError(f'Kept node {old} carries {sorted(set(source_node) - allowed)}')
        copy = {key: source_node[key] for key in ['name', *TRS] if key in source_node}
        children = [new_index[c] for c in source_node.get('children', []) if c in new_index]
        if children:
            copy['children'] = children
        out['nodes'].append(copy)

    garment_node = out['nodes'][new_index[node_index]]
    garment_node.update(mesh=0, skin=0)
    parent = parents_of(nodes)
    roots = [new_index[n] for n in kept if parent.get(n) not in new_index]
    out['scenes'] = [{'name': garment['id'], 'nodes': roots}]

    ibm_spec = source['accessors'][skin['inverseBindMatrices']]
    ibm = accessor_bytes(source, source_binary, skin['inverseBindMatrices'])
    ibm = b''.join(ibm[64 * i:64 * (i + 1)] for i in referenced)
    new_skin = {'inverseBindMatrices': writer.accessor({**ibm_spec, 'count': len(referenced)}, ibm),
                'joints': [new_index[j] for j in joint_nodes]}
    # The source skin names CC_Base_Pelvis as its skeleton, but CC_Base_Waist is a sibling of
    # the pelvis, so that value is invalid glTF. Use the joints' actual common root instead;
    # `skeleton` is informative and does not change skinning.
    new_skin['skeleton'] = new_index[common_root(nodes, joint_nodes)]
    out['skins'].append(new_skin)

    primitives = []
    for primitive in mesh['primitives']:
        if set(primitive) - {'attributes', 'indices', 'material', 'mode'}:
            raise ValueError('Unsupported primitive fields')
        attributes = {}
        for name, index in primitive['attributes'].items():
            attributes[name] = (writer.remapped_joints(index, remap) if name.startswith('JOINTS_')
                                else writer.copy_accessor(index, 34962))
        copy = {'attributes': attributes}
        if 'indices' in primitive:
            copy['indices'] = writer.copy_accessor(primitive['indices'], 34963)
        if 'material' in primitive:
            copy['material'] = writer.copy_material(primitive['material'])
        copy['mode'] = primitive.get('mode', 4)
        primitives.append(copy)
    out['meshes'].append({'name': mesh['name'], 'primitives': primitives})

    facts = {
        'joint_names': [nodes[j]['name'] for j in joint_nodes],
        'hierarchy_nodes': [nodes[n]['name'] for n in kept if n != node_index],
        'vertices': sum(source['accessors'][p['attributes']['POSITION']]['count'] for p in mesh['primitives']),
        'triangles': sum(source['accessors'][p['indices']]['count'] // 3 for p in mesh['primitives']),
        'attributes': sorted({name for p in mesh['primitives'] for name in p['attributes']}),
        'joint_sets': len(joint_sets),
        'materials': [material_facts(source['materials'][p['material']])
                      for p in mesh['primitives'] if 'material' in p],
        'source_mesh': node['mesh'], 'source_skin': node['skin'],
        'source_skin_joint_count': len(skin['joints']),
    }
    return writer.encode(), facts


def sidecar(garment, glb, facts, outfit_path, outfit_sha256, node_index, avatar_sha256):
    names = facts['joint_names']
    return {
        'schema': SCHEMA,
        'id': garment['id'],
        'display_name': garment['display_name'],
        'slot': garment['slot'],
        'layer': garment['layer'],
        'asset': {'glb': f"{garment['id']}.glb", 'bytes': len(glb),
                  'sha256': hashlib.sha256(glb).hexdigest()},
        'binding': {
            'skeleton': 'CC5',
            'bind_by': 'joint_name',
            'joints': names,
            'joints_sha256': hashlib.sha256('\n'.join(names).encode()).hexdigest(),
            'max_influences': 4 * facts['joint_sets'],
            'bind_pose': 'CC5 rest pose of the source avatar; inverse bind matrices copied from its CC_Base_Body skin',
        },
        'geometry': {'vertices': facts['vertices'], 'triangles': facts['triangles'],
                     'attributes': facts['attributes'], 'morph_targets': 0},
        'materials': facts['materials'],
        'deformation': 'static skinned, no simulation',
        'provenance': {
            'source_outfit': outfit_path,
            'source_outfit_sha256': outfit_sha256,
            'source_avatar_sha256': avatar_sha256,
            'source_node_index': node_index,
            'source_node_name': garment['source_node_name'],
            'source_mesh_index': facts['source_mesh'],
            'source_skin_index': facts['source_skin'],
            'source_skin_joint_count': facts['source_skin_joint_count'],
            'authored_by': 'scripts/condo/build_outfit.py',
            'extracted_by': SCRIPT,
        },
    }


def write_once(path, data, overwrite):
    """Write deterministic output; identical existing content is a no-op."""
    if path.exists():
        if path.read_bytes() == data:
            return 'unchanged'
        if not overwrite:
            raise SystemExit(f'{path} exists with different content; pass --overwrite to replace it')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return 'written'


def split_outfit(outfit, out_dir, overwrite=False):
    outfit, out_dir = Path(outfit), Path(out_dir)
    raw = outfit.read_bytes()
    outfit_sha256 = hashlib.sha256(raw).hexdigest()
    source, source_binary = read_glb(outfit)
    report_path = outfit.with_suffix('.json')
    avatar_sha256 = None
    if report_path.exists():
        report = json.loads(report_path.read_text())
        if report.get('outfit_sha256') == outfit_sha256:
            avatar_sha256 = report.get('source_sha256')
    try:  # Record a repo-relative path so the sidecar does not depend on the checkout location.
        label = outfit.resolve().relative_to(REPO).as_posix()
    except ValueError:
        label = outfit.as_posix()
    results = []
    for garment in GARMENTS:
        matches = [i for i, n in enumerate(source['nodes']) if n.get('name') == garment['source_node_name']]
        if len(matches) != 1:
            raise ValueError(f"Expected one node named {garment['source_node_name']!r}, found {matches}")
        glb, facts = extract_garment(source, source_binary, matches[0], garment, outfit_sha256)
        record = sidecar(garment, glb, facts, label, outfit_sha256, matches[0], avatar_sha256)
        folder = out_dir / garment['id']
        status = [write_once(folder / f"{garment['id']}.glb", glb, overwrite),
                  write_once(folder / 'garment.json', (json.dumps(record, indent=2) + '\n').encode(), overwrite)]
        results.append({'id': garment['id'], 'glb_bytes': len(glb), 'sha256': record['asset']['sha256'],
                        'joints': len(facts['joint_names']), 'vertices': facts['vertices'],
                        'triangles': facts['triangles'], 'files': status})
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('outfit', type=Path, help='outfit GLB produced by build_outfit.py')
    parser.add_argument('out_dir', type=Path, help='garments directory, e.g. scenes/toronto_condo/garments')
    parser.add_argument('--overwrite', action='store_true', help='replace differing existing outputs')
    args = parser.parse_args()
    json.dump(split_outfit(args.outfit, args.out_dir, args.overwrite), sys.stdout, indent=2)
    print()
