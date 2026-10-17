"""Passive inward-opening condo entry door with retained frame and transom."""
import hashlib
import json
import math
import os
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from fridge_mechanism import assembly, panel, runtime
from bedside_mechanism import export_parts, inspection_studio, SCENE, SOURCE
from kitchen_mechanisms import retain_objects

OUT = SCENE/'interaction-authoring/entry-door'
BASE = (.145, 4.94, 0)


def door_parts(palette):
    objects = [o for o in bpy.data.objects if o.get('workspace_group') == 'entry_door']
    fixed_objects = [o for o in objects if o.name.startswith(('Workspace detail entry door jamb',
        'Workspace detail entry door lintel', 'Workspace detail entry transom'))]
    assert len(objects) == 6 and len(fixed_objects) == 4
    fixed = retain_objects('entry_door_frame', BASE, fixed_objects)
    leaf = assembly('entry_door_leaf', (0, 0, 1.14), 25.)
    panel(leaf, 'entry_door_panel', (-.5025, 0, 1.14), (1.005, .045, 2.22), palette['oak'])
    for side, label in ((-1, 'room'), (1, 'outside')):
        panel(leaf, 'entry_door_mount_'+label, (-.85, side*.0325, 1.02), (.045, .02, .09), palette['dark'])
        panel(leaf, 'entry_door_pull_'+label, (-.80, side*.054, 1.02), (.15, .018, .018), palette['dark'])
    leaf['joint'] = {'kind':'revolute', 'parent':'entry_door_frame',
        'parent_anchor':runtime(leaf['origin']), 'child_anchor':[0, 0, 0],
        'axis':[0, 1, 0], 'limits_rad':[0, math.pi/2]}
    return [fixed, leaf], [o.name for o in objects]


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        requested.objects = [n for n in available.objects if n.startswith('Workspace detail entry ')]
    palette = {'oak':bpy.data.materials['Kitchen light oak veneer'],
               'dark':bpy.data.materials['Workspace dark bronze']}
    parts, names = door_parts(palette)
    bpy.context.view_layer.update()
    mechanism = {'id':'condo_entry_door', 'placement':runtime(BASE),
        'replaces_solid':'entry_door', 'replaces_nodes':names, 'parts':export_parts(parts, OUT)}
    scene = json.loads((SCENE/'scene-upper-cabinets.json').read_text())
    scene['id'] += '-entry-door'
    scene['mechanisms'].append(mechanism)
    for cloth in scene['cloths']:
        cloth['collision_solids'].remove('entry_door')
        cloth['collision_fixed_parts'].append('entry_door_frame')
    scene['cameras'].append({'id':'entry-door', 'position':[-1.35, 1.5, -2.6],
        'look_at':[-.35, 1.4, -4.94], 'fov_deg':62.})
    (SCENE/'scene-entry-door.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT/'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
    studio = inspection_studio((-3., -4., 3.2), (-.5, -.25, 1.45), 3.8,
                               ((-1., -2., 4.), 500, 3.))
    studio.render.filepath = str(output/'closed.png')
    bpy.ops.render.render(write_still=True)
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'entry-door.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    (output/'capture.json').write_text(json.dumps({'scope':__doc__, 'source_sha256':before,
        'mechanism':mechanism, 'limits':['Closed authoring preview only.',
        'Estimated 25 kg mass, ideal hinge; no latch or closer. Both handles are rigid with the leaf.',
        'No exterior hallway or avatar passage acceptance.']}, indent=2)+'\n')
    print('ENTRY_DOOR_EXPORTED', flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
