"""Append garment meshes to a CC5 GLB without re-exporting its original payload."""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import struct

import numpy as np


def read_glb(path):
    raw = Path(path).read_bytes()
    magic, version, size, length, kind = struct.unpack_from('<4sIIII', raw)
    if (magic, version, size, kind) != (b'glTF', 2, len(raw), 0x4E4F534A):
        raise ValueError('Expected a GLB 2 JSON chunk')
    document = json.loads(raw[20:20 + length])
    binary_length, binary_kind = struct.unpack_from('<II', raw, 20 + length)
    if binary_kind != 0x004E4942 or 28 + length + binary_length != len(raw):
        raise ValueError('Expected exactly one embedded BIN chunk')
    return document, raw[28 + length:]


def accessor(document, binary, index):
    spec = document['accessors'][index]
    view = document['bufferViews'][spec['bufferView']]
    dtype = np.dtype({5126: '<f4', 5121: 'u1', 5123: '<u2', 5125: '<u4'}[spec['componentType']])
    columns = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[spec['type']]
    if spec.get('sparse') or view.get('byteStride', columns * dtype.itemsize) != columns * dtype.itemsize:
        raise ValueError('Sparse/interleaved source requires an explicit decoder')
    if view.get('buffer', 0) != 0 or spec.get('normalized', False):
        raise ValueError('Expected embedded, unnormalized accessor')
    return np.frombuffer(binary, dtype=dtype, count=spec['count'] * columns,
                         offset=view.get('byteOffset', 0) + spec.get('byteOffset', 0)).reshape(-1, columns).copy()


def append_accessor(document, binary, values, kind, component):
    values = np.asarray(values, dtype={5126: '<f4', 5123: '<u2', 5125: '<u4'}[component])
    binary.extend(b'\0' * (-len(binary) % 4))
    view = len(document['bufferViews'])
    document['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': values.nbytes})
    binary.extend(values.tobytes())
    index = len(document['accessors'])
    document['accessors'].append({'bufferView': view, 'componentType': component,
                                  'count': len(values), 'type': kind,
                                  'min': values.min(axis=0).reshape(-1).tolist(),
                                  'max': values.max(axis=0).reshape(-1).tolist()})
    return index


def append_texture(document, binary, data):
    binary.extend(b'\0' * (-len(binary) % 4))
    view = len(document['bufferViews'])
    document['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(data)})
    binary.extend(data)
    image = len(document['images'])
    document['images'].append({'bufferView': view, 'mimeType': 'image/png'})
    index = len(document['textures'])
    sampler = len(document['samplers'])
    document['samplers'].append({'magFilter': 9729, 'minFilter': 9987, 'wrapS': 10497, 'wrapT': 10497})
    document['textures'].append({'source': image, 'sampler': sampler})
    return index


def append_garment(document, binary, body_node, garment):
    material = len(document['materials'])
    document['materials'].append({'name': garment['name'], 'doubleSided': True,
        'pbrMetallicRoughness': {'baseColorFactor': garment['colour'] + [1],
                                'metallicFactor': 0, 'roughnessFactor': .9}})
    attrs = {name: append_accessor(document, binary, garment[key], kind, component)
             for name, key, kind, component in [
                 ('POSITION', 'positions', 'VEC3', 5126), ('NORMAL', 'normals', 'VEC3', 5126),
                 ('JOINTS_0', 'joints', 'VEC4', 5123), ('WEIGHTS_0', 'weights', 'VEC4', 5126)]}
    if 'uv' in garment:
        attrs['TEXCOORD_0'] = append_accessor(document, binary, garment['uv'], 'VEC2', 5126)
        textures = garment['textures']
        document['materials'][material]['pbrMetallicRoughness']['baseColorTexture'] = {
            'index': append_texture(document, binary, textures['base'])}
        document['materials'][material]['normalTexture'] = {
            'index': append_texture(document, binary, textures['normal']), 'scale': .6}
    indices = append_accessor(document, binary, np.asarray(garment['triangles']).reshape(-1, 1), 'SCALAR', 5125)
    node = deepcopy(document['nodes'][body_node])
    node.update(name=garment['name'], mesh=len(document['meshes']))
    document['meshes'].append({'name': garment['name'], 'primitives': [
        {'attributes': attrs, 'indices': indices, 'material': material, 'mode': 4}]})
    index = len(document['nodes'])
    document['nodes'].append(node)
    parent = next(n for n in document['nodes'] if body_node in n.get('children', []))
    parent['children'].append(index)


def verify_preservation(original, original_binary, dressed, dressed_binary):
    """Exact comparison of every original JSON entry and embedded payload byte."""
    restored = deepcopy(dressed)
    for key, value in original.items():
        if isinstance(value, list):
            restored[key] = restored[key][:len(value)]
    for node, old in zip(restored['nodes'], original['nodes']):
        if 'children' in old:
            node['children'] = node['children'][:len(old['children'])]
    restored['buffers'] = deepcopy(original['buffers'])
    if restored != original or dressed_binary[:len(original_binary)] != original_binary:
        raise ValueError('Outfit changed original avatar content')
    return {'original_json_entries_preserved': True, 'original_binary_prefix_preserved': True,
            'original_binary_bytes': len(original_binary),
            'original_binary_sha256': hashlib.sha256(original_binary).hexdigest(),
            'added_meshes': len(dressed['meshes']) - len(original['meshes'])}


def write_outfit(source, destination, garments):
    if Path(destination).exists() or Path(source).resolve() == Path(destination).resolve():
        raise ValueError('Refusing to overwrite an avatar or previous outfit')
    original, original_binary = read_glb(source)
    document, binary = deepcopy(original), bytearray(original_binary)
    body_node = next(i for i, n in enumerate(document['nodes']) if n.get('name') == 'CC_Base_Body')
    for garment in garments:
        append_garment(document, binary, body_node, garment)
    document['buffers'][0]['byteLength'] = len(binary)
    binary.extend(b'\0' * (-len(binary) % 4))
    report = verify_preservation(original, original_binary, document, binary)
    encoded = json.dumps(document, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    result = (struct.pack('<4sIIII', b'glTF', 2, 28 + len(encoded) + len(binary), len(encoded), 0x4E4F534A)
              + encoded + struct.pack('<II', len(binary), 0x004E4942) + binary)
    Path(destination).write_bytes(result)
    # Verify the on-disk asset, not just the pre-serialization object.
    verify_preservation(original, original_binary, *read_glb(destination))
    report.update(source_sha256=hashlib.sha256(Path(source).read_bytes()).hexdigest(),
                  outfit_sha256=hashlib.sha256(result).hexdigest())
    return report
