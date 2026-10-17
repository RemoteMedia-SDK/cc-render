"""Author the bathroom's passive inward-opening door; no avatar motion or latch."""
import hashlib
import json
import os
from pathlib import Path
import struct
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from bedroom_door import hinged_leaf
from fridge_mechanism import assembly, panel
from bedside_mechanism import export_parts, inspection_studio, SCENE, SOURCE

OUT = SCENE / 'interaction-authoring/bathroom-door'


def package(assemblies):
    scene = json.loads((SCENE / 'scene-balcony-slider.json').read_text())
    with (SCENE / scene['geometry']).open('rb') as stream:
        stream.read(12)
        size, kind = struct.unpack('<II', stream.read(8))
        assert kind == 0x4E4F534A
        gltf = json.loads(stream.read(size))
    assert sum(n.get('name') == 'Bathroom south wall A' for n in gltf['nodes']) == 1
    mechanism = {'id':'bathroom_door', 'placement':[-4.37, 0, -2.],
                 'replaces_solid':'bathroom_south_wall_a', 'replaces_nodes':['Bathroom south wall A'],
                 'parts':export_parts(assemblies, OUT)}
    scene['id'] += '-bathroom-door'
    scene['mechanisms'].append(mechanism)
    for cloth in scene['cloths']:
        cloth['collision_solids'].remove(mechanism['replaces_solid'])
        cloth['collision_fixed_parts'].append('bathroom_hinge_jamb')
    scene['cameras'].append({'id':'bathroom-door', 'position':[-2.6, 1.55, .4],
                            'look_at':[-3.78, 1.4, -2.05], 'fov_deg':65.})
    (SCENE/'scene-bathroom-door.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT/'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
    return mechanism


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    names = {'oak':'Kitchen light oak veneer', 'dark':'Workspace dark bronze', 'plaster':'Warm plaster'}
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        assert all(name in available.materials for name in names.values())
        requested.materials = list(names.values())
    palette = {key:bpy.data.materials[name] for key, name in names.items()}
    fixed = assembly('bathroom_hinge_jamb', (0, 0, 0), None)
    # Retain exactly the replaced wall; the hinge sits 3 cm into the existing opening.
    panel(fixed, 'bathroom_retained_wall', (-.83, 0, 1.45), (1.6, .14, 2.9), palette['plaster'])
    assemblies = [fixed, hinged_leaf('bathroom', 1.18, palette)]
    bpy.context.view_layer.update()
    mechanism = package(assemblies)
    scene = inspection_studio((3., -5., 3.4), (-.2, .35, 1.45), 5.3, ((1., -2., 4.), 600, 3.))
    scene.render.filepath = str(output/'closed.png')
    bpy.ops.render.render(write_still=True)
    scene['scope'] = __doc__
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'bathroom-door.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    (output/'capture.json').write_text(json.dumps({'scope':__doc__, 'source_sha256':before,
        'mechanism':mechanism, 'limits':['Estimated 20 kg mass, ideal passive hinge; no latch or lever actuation.',
        'Closed asset inspection only; physics and actual avatar handling are separate gates.',
        'Only the hallway-side handle is exposed by the single-region grasp schema.']}, indent=2)+'\n')
    print('BATHROOM_DOOR_EXPORTED', flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
