"""Three hollow upper cabinets with shelves and passive hinges; no task actuators."""
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from fridge_mechanism import assembly, panel, runtime
from bedside_mechanism import export_parts, inspection_studio, SCENE, SOURCE

OUT = SCENE/'interaction-authoring/upper-cabinets'
BASE = (-5.72, .20, 0)


def cabinet_case(palette):
    part = assembly('upper_cabinet_shell', (0, 0, 0), None)
    for column, y in enumerate((-.55, 0., .55)):
        prefix = f'upper_case_{column}'
        panel(part, prefix+'_back', (-.19, y, 2.12), (.02, .54, .72), palette['oak'])
        for side in (-1, 1):
            panel(part, prefix+f'_side_{side}', (.01, y+side*.26, 2.12),
                  (.38, .02, .72), palette['oak'])
        for level, z in enumerate((1.77, 2.12, 2.47)):
            panel(part, prefix+f'_shelf_{level}', (.01, y, z), (.38, .50, .02), palette['oak'])
    return part


def cabinet_leaf(column, y, palette):
    name = f'upper_cabinet_door_{column}'
    # Forward offset leaves clearance for the full thickness at the hinge.
    part = assembly(name, (.23, y+.262, 2.12), 4.5)
    panel(part, name+'_panel', (.23, y, 2.12), (.024, .524, .702), palette['oak'])
    panel(part, name+'_pull', (.254, y, 1.79), (.018, .44, .014), palette['dark'])
    part['joint'] = {'kind':'revolute', 'parent':'upper_cabinet_shell',
        'parent_anchor':runtime(part['origin']), 'child_anchor':[0, 0, 0],
        'axis':[0, 1, 0], 'limits_rad':[0, math.pi/2]}
    return part


def package(parts):
    scene = json.loads((SCENE/'scene-desk-drawer.json').read_text())
    with (SCENE/scene['geometry']).open('rb') as stream:
        stream.read(12)
        length, kind = struct.unpack('<II', stream.read(8))
        assert kind == 0x4E4F534A
        gltf = json.loads(stream.read(length))
    nodes = [n['name'] for n in gltf['nodes'] if n.get('name', '').startswith('Kitchen detail upper ')]
    assert len(nodes) == 9
    mechanism = {'id':'upper_cabinets', 'placement':runtime(BASE),
        'replaces_solid':'kitchen_upper', 'replaces_nodes':nodes, 'parts':export_parts(parts, OUT)}
    scene['id'] += '-upper-cabinets'
    scene['mechanisms'].append(mechanism)
    for cloth in scene['cloths']:
        cloth['collision_solids'].remove('kitchen_upper')
        cloth['collision_fixed_parts'].append('upper_cabinet_shell')
    scene['cameras'].append({'id':'upper-cabinets', 'position':[-3.55, 2.15, 1.4],
        'look_at':[-5.55, 2.1, -.2], 'fov_deg':58.})
    (SCENE/'scene-upper-cabinets.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT/'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
    return mechanism


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    names = {'oak':'Kitchen light oak veneer', 'dark':'Workspace dark bronze'}
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        assert all(n in available.materials for n in names.values())
        requested.materials = list(names.values())
    palette = {key:bpy.data.materials[name] for key, name in names.items()}
    parts = [cabinet_case(palette)]+[cabinet_leaf(i, y, palette) for i, y in enumerate((-.55, 0., .55))]
    bpy.context.view_layer.update()
    mechanism = package(parts)
    scene = inspection_studio((2.5, -3., 3.), (.05, 0, 2.1), 2.4,
                               ((2., -2., 4.), 400, 3.))
    scene.render.filepath = str(output/'closed.png')
    bpy.ops.render.render(write_still=True)
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'upper-cabinets.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    (output/'capture.json').write_text(json.dumps({'scope':__doc__, 'source_sha256':before,
        'mechanism':mechanism, 'limits':['Closed authoring preview only.',
        'Estimated 4.5 kg leaves; ideal hinges without latches or closers.']}, indent=2)+'\n')
    print('UPPER_CABINETS_EXPORTED', flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
