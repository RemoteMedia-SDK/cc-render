"""Author a hollow entry cabinet and passive doors; no animated opening or avatar motion."""
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

OUT = SCENE / 'interaction-authoring/entry'


def carcass(palette):
    part = assembly('entry_shell', (0, 0, 0), None)
    for x in (-.678, .678):
        panel(part, f'entry_side_{x}', (x, 0, 1.13), (.024, .48, 2.10), palette['oak'])
    panel(part, 'entry_back', (0, .228, 1.13), (1.332, .024, 2.10), palette['oak'])
    for z in (.092, 2.168):
        panel(part, f'entry_cap_{z}', (0, 0, z), (1.332, .432, .024), palette['oak'])
    panel(part, 'entry_divider', (0, 0, 1.13), (.024, .432, 2.052), palette['oak'])
    for z in (.60, 1.15, 1.70):
        for x in (-.339, .339):
            panel(part, f'entry_shelf_{z}_{x}', (x, 0, z), (.654, .432, .020), palette['oak'])
    panel(part, 'entry_plinth', (0, 0, .04), (1.24, .36, .08), palette['dark'])
    return part


def leaf(side, palette):
    name = 'entry_left_door' if side < 0 else 'entry_right_door'
    # Outboard hinges; opposite signed yaw opens both leaves into the room.
    part = assembly(name, (side*.683, -.265, 1.13), 9.)
    panel(part, name+'_panel', (side*.345, -.265, 1.13), (.676, .026, 2.075), palette['oak'])
    handle_x = side*.125
    pull = panel(part, name+'_pull', (handle_x, -.305, 1.04), (.014, .014, .24), palette['dark'])
    for z in (.94, 1.14):
        panel(part, name+f'_mount_{z}', (handle_x, -.289, z), (.014, .026, .014), palette['dark'])
    part['joint'] = {'kind':'revolute', 'parent':'entry_shell',
                     'parent_anchor':runtime(part['origin']), 'child_anchor':[0, 0, 0],
                     'axis':[0, 1, 0], 'limits_rad':sorted([0, side*math.pi/2])}
    part['handle_region'] = {'mesh':pull.name, 'centre':runtime(pull.location),
                            'half_extents':[.007, .12, .007]}
    return part


def package(parts):
    scene = json.loads((SCENE / 'scene-drawers.json').read_text())
    with (SCENE / scene['geometry']).open('rb') as stream:
        stream.read(12)
        size, kind = struct.unpack('<II', stream.read(8))
        assert kind == 0x4E4F534A
        gltf = json.loads(stream.read(size))
    prefixes = ('Workspace detail entry cabinet', 'Workspace detail cabinet handle')
    nodes = [n['name'] for n in gltf['nodes'] if n.get('name', '').startswith(prefixes)]
    assert len(nodes) == 6, 'Review entry cabinet replacement geometry'
    mechanism = {'id':'entry_cabinet', 'placement':[-1.8, 0, -4.65],
                 'replaces_solid':'entry_storage', 'replaces_nodes':nodes,
                 'parts':export_parts(parts, OUT)}
    scene['id'] += '-cabinet'
    scene['mechanisms'].append(mechanism)
    for cloth in scene['cloths']:
        cloth['collision_solids'].remove('entry_storage')
        cloth['collision_fixed_parts'].append('entry_shell')
    scene['cameras'].append({'id':'entry-mechanics', 'position':[-.5, 1.65, -2.2],
                             'look_at':[-1.8, 1.1, -4.4], 'fov_deg':58.})
    (SCENE / 'scene-cabinet.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT / 'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
    return mechanism


def inspection(output):
    scene = inspection_studio((2.7, -4., 2.8), (0, -.2, 1.1), 3.6, ((1., -2., 4.), 600, 3.))
    scene.render.filepath = str(output / 'closed.png')
    bpy.ops.render.render(write_still=True)


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    names = {'oak':'Kitchen light oak veneer', 'dark':'Workspace dark bronze'}
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        assert all(name in available.materials for name in names.values())
        requested.materials = list(names.values())
    palette = {key:bpy.data.materials[name] for key, name in names.items()}
    parts = [carcass(palette), leaf(-1, palette), leaf(1, palette)]
    bpy.context.view_layer.update()
    mechanism = package(parts)
    inspection(output)
    bpy.context.scene['scope'] = __doc__
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'entry-mechanism.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    (output / 'capture.json').write_text(json.dumps({
        'scope':__doc__, 'source_sha256':before, 'mechanism':mechanism,
        'limits':['Door masses are engineering estimates; ideal hinges, no latch.',
                  'Closed geometry inspection only; no physics or avatar acceptance.']}, indent=2)+'\n')
    print('ENTRY_EXPORTED', len(parts), flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
