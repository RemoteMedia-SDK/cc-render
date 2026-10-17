"""Read-only preservation and skin/texture integrity audit of an additive outfit."""
import argparse
from copy import deepcopy
import hashlib
from io import BytesIO
import json
from pathlib import Path

import numpy as np
from PIL import Image

from outfit_package import accessor, read_glb, verify_preservation


def check_mesh(document, binary, node):
    mesh = document['meshes'][node['mesh']]
    primitive = mesh['primitives'][0]
    attrs = primitive['attributes']
    positions = accessor(document, binary, attrs['POSITION'])
    weights = accessor(document, binary, attrs['WEIGHTS_0'])
    joints = accessor(document, binary, attrs['JOINTS_0'])
    indices = accessor(document, binary, primitive['indices']).reshape(-1, 3)
    assert np.isfinite(positions).all() and np.isfinite(weights).all() and (weights >= 0).all()
    assert joints.max() < len(document['skins'][node['skin']]['joints'])
    assert len(positions) == len(weights) == len(joints) and indices.max() < len(positions)
    error = float(abs(weights.sum(axis=1) - 1).max())
    assert error < 1e-6
    triangles = positions[indices]
    areas = np.linalg.norm(np.cross(triangles[:, 1] - triangles[:, 0],
                                    triangles[:, 2] - triangles[:, 0]), axis=1) / 2
    assert (areas > 1e-10).all(), 'Degenerate garment triangles'
    return {'name': mesh['name'], 'vertices': len(positions), 'triangles': len(indices),
            'maximum_weight_sum_error': error, 'degenerate_triangles': 0}


def check_outfit(source, variant):
    original, original_binary = read_glb(source)
    document, binary = read_glb(variant)
    report = verify_preservation(original, original_binary, document, binary)
    old_body = next(n for n in original['nodes'] if n.get('name') == 'CC_Base_Body')
    added_nodes = document['nodes'][len(original['nodes']):]
    names = [n.get('name') for n in document['nodes'] if n.get('name')]
    assert len(names) == len(set(names)), 'Duplicate node names can affect mapping'
    for node in added_nodes:
        assert node['skin'] == old_body['skin']
        for field in ['translation', 'rotation', 'scale', 'matrix']:
            assert node.get(field) == old_body.get(field)
    report['garments'] = [check_mesh(document, binary, n) for n in added_nodes]
    report['textures'] = []
    for image in document['images'][len(original['images']):]:
        view = document['bufferViews'][image['bufferView']]
        start = view.get('byteOffset', 0)
        with Image.open(BytesIO(binary[start:start + view['byteLength']])) as decoded:
            decoded.load()
            report['textures'].append({'size': decoded.size, 'mode': decoded.mode})
    # Negative controls demonstrate the guard rejects original-content changes.
    changed = deepcopy(document)
    changed['nodes'][0]['name'] = 'preservation-negative-control'
    for candidate, payload in [(changed, binary), (document, bytes([binary[0] ^ 1]) + binary[1:])]:
        try:
            verify_preservation(original, original_binary, candidate, payload)
        except ValueError:
            continue
        raise AssertionError('Preservation guard accepted a mutation')
    report.update(mutation_rejections=2, source_sha256=hashlib.sha256(Path(source).read_bytes()).hexdigest(),
                  variant_sha256=hashlib.sha256(Path(variant).read_bytes()).hexdigest(),
                  limits='Asset integrity only; not visual fit, cloth collision or interaction certification.')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('variant', type=Path)
    parser.add_argument('report', type=Path)
    args = parser.parse_args()
    report = check_outfit(args.source, args.variant)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
