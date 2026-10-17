"""Create an opt-in runtime package from the audited passive fridge assets."""
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'


def build():
    scene = json.loads((ROOT / 'scene-lightmap.json').read_text())
    source = ROOT / 'interaction-authoring/fridge/mechanism.json'
    authored = json.loads(source.read_text())
    with (ROOT / scene['geometry']).open('rb') as stream:
        stream.read(12)
        size, kind = struct.unpack('<II', stream.read(8))
        assert kind == 0x4E4F534A
        gltf = json.loads(stream.read(size))
    nodes = [node['name'] for node in gltf['nodes']
             if node.get('name', '').startswith(authored['replaces_object_prefix'])]
    assert len(nodes) == 5, 'Review replacement nodes when the original fridge changes'
    mechanism = {key: authored[key] for key in ('id', 'placement', 'parts', 'replaces_solid')}
    mechanism['replaces_nodes'] = nodes
    for part in mechanism['parts']:
        part['geometry'] = str(source.parent.relative_to(ROOT) / part['geometry'])
        if 'joint' in part:
            assert part['joint'].pop('motor') is None, 'Only passive joints are supported'
    scene['id'] += '-articulated'
    scene['mechanisms'] = [mechanism]
    target = ROOT / 'scene-articulated.json'
    target.write_text(json.dumps(scene, indent=2) + '\n')
    print(target)


if __name__ == '__main__':
    build()
