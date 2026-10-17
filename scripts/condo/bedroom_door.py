"""Author a passive full-height bedroom door in the existing opening; no avatar motion."""
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

OUT = SCENE / 'interaction-authoring/bedroom-door'


def parts(palette):
    frame = assembly('bedroom_hinge_jamb', (0, 0, 0), None)
    panel(frame, 'bedroom_retained_jamb', (-.15, 0, 1.45), (.24, .14, 2.9), palette['plaster'])
    leaf = assembly('bedroom_door_leaf', (0, 0, 1.44), 20.)
    panel(leaf, 'bedroom_door_panel', (.56, 0, 1.44), (1.12, .035, 2.82), palette['oak'])
    for side in (-1, 1):
        panel(leaf, f'bedroom_lever_mount_{side}', (.97, side*.0275, 1.04), (.045, .02, .09), palette['dark'])
        handle = panel(leaf, f'bedroom_lever_{side}', (.92, side*.05, 1.04), (.14, .016, .018), palette['dark'])
        if side < 0:
            leaf['handle_region'] = {'mesh':handle.name, 'centre':runtime(handle.location),
                                    'half_extents':[.07, .009, .008]}
    leaf['joint'] = {'kind':'revolute', 'parent':'bedroom_hinge_jamb',
                     'parent_anchor':runtime(leaf['origin']), 'child_anchor':[0, 0, 0],
                     'axis':[0, 1, 0], 'limits_rad':[0, math.pi/2]}
    return [frame, leaf]


def package(assemblies):
    scene = json.loads((SCENE / 'scene-cabinet.json').read_text())
    with (SCENE / scene['geometry']).open('rb') as stream:
        stream.read(12)
        size, kind = struct.unpack('<II', stream.read(8))
        assert kind == 0x4E4F534A
        gltf = json.loads(stream.read(size))
    assert sum(n.get('name') == 'Bedroom door jamb' for n in gltf['nodes']) == 1
    mechanism = {'id':'bedroom_door', 'placement':[2.27, 0, -1.2],
                 'replaces_solid':'bedroom_door_jamb', 'replaces_nodes':['Bedroom door jamb'],
                 'parts':export_parts(assemblies, OUT)}
    scene['id'] += '-doors'
    scene['mechanisms'].append(mechanism)
    for cloth in scene['cloths']:
        cloth['collision_solids'].remove('bedroom_door_jamb')
        cloth['collision_fixed_parts'].append('bedroom_hinge_jamb')
    scene['cameras'].append({'id':'bedroom-door', 'position':[.5, 1.5, 1.5],
                             'look_at':[2.85, 1.45, -1.25], 'fov_deg':65.})
    (SCENE / 'scene-doors.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT / 'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
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
    assemblies = parts({key:bpy.data.materials[name] for key, name in names.items()})
    bpy.context.view_layer.update()
    mechanism = package(assemblies)
    scene = inspection_studio((3., -4., 3.4), (.4, .35, 1.45), 5.2, ((1., -2., 4.), 600, 3.))
    scene.render.filepath = str(output / 'closed.png')
    bpy.ops.render.render(write_still=True)
    scene['scope'] = __doc__
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'bedroom-door.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    (output / 'capture.json').write_text(json.dumps({
        'scope':__doc__, 'source_sha256':before, 'mechanism':mechanism,
        'limits':['Estimated mass and ideal passive hinge; no latch or rotating lever mechanism.',
                  'Closed asset inspection only; no physics or avatar handling acceptance.',
                  'Only the room-side handle is exposed by the current single-region grasp schema.']}, indent=2)+'\n')
    print('BEDROOM_DOOR_EXPORTED', flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
