"""Read-only integrity audit of named meshes in an exported condo GLB."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np

from outfit_package import accessor, read_glb


def check(path, prefix):
    assert prefix, 'Select a nonempty mesh-name prefix'
    document, binary = read_glb(path)
    results = []
    for mesh in document['meshes']:
        if not mesh.get('name', '').startswith(prefix):
            continue
        count, minimum, digest = 0, float('inf'), hashlib.sha256()
        for primitive in mesh['primitives']:
            assert primitive.get('mode', 4) == 4, 'Expected exported triangles'
            attributes = primitive['attributes']
            arrays = [accessor(document, binary, attributes[key])
                      for key in ('POSITION', 'NORMAL', 'TEXCOORD_0')]
            positions, normals, uv = arrays
            indices = accessor(document, binary, primitive['indices']).reshape(-1, 3)
            assert all(np.isfinite(array).all() for array in arrays), mesh['name']
            assert len(positions) == len(normals) == len(uv) and indices.max() < len(positions)
            triangles = positions[indices]
            areas = np.linalg.norm(np.cross(triangles[:, 1] - triangles[:, 0],
                                            triangles[:, 2] - triangles[:, 0]), axis=1) * .5
            assert (areas > 1e-12).all(), (mesh['name'], float(areas.min()))
            minimum = min(minimum, float(areas.min()))
            count += len(indices)
            for array in arrays + [indices]:
                digest.update(array.tobytes())
        results.append({'mesh':mesh['name'], 'triangles':count,
                        'minimum_triangle_area':minimum, 'attribute_sha256':digest.hexdigest()})
    assert results, ('No matching exported meshes', prefix)
    return {'glb_sha256':hashlib.sha256(path.read_bytes()).hexdigest(), 'prefix':prefix,
            'meshes':results, 'triangles':sum(row['triangles'] for row in results),
            'limits':'Finite local attributes, valid indices and nondegenerate triangles only. Placement, materials, collision and interactions require separate review.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('glb', type=Path)
    parser.add_argument('report', type=Path)
    parser.add_argument('--prefix', required=True)
    args = parser.parse_args()
    result = check(args.glb, args.prefix)
    args.report.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'meshes':len(result['meshes']), 'triangles':result['triangles'],
                      'glb_sha256':result['glb_sha256']}))
