"""Hollow desk drawer on a passive rail; preserves the source desktop and frame."""
import hashlib
import json
import os
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from fridge_mechanism import assembly, panel, runtime
from bedside_mechanism import export_parts, inspection_studio, SCENE, SOURCE
from kitchen_mechanisms import retain_objects, tray

OUT = SCENE/'interaction-authoring/desk'
BASE = (1.55, 3.90, 0)


def desk_parts(p):
    objects = [o for o in bpy.data.objects if o.get('workspace_group') == 'workspace_desk']
    retained = [o for o in objects if not o.name.startswith('Workspace detail drawer ')]
    fixed = retain_objects('desk_shell', BASE, retained)
    # Keep the outer case envelope; the original front overlapped the north leg.
    # A narrow fixed filler and a narrower drawer leave that leg unobstructed.
    panel(fixed, 'desk_case_back', (.282, 0, .59), (.016, .49, .27), p['oak'])
    panel(fixed, 'desk_case_bottom', (0, 0, .463), (.548, .49, .016), p['oak'])
    for y in (-.237, .237):
        panel(fixed, f'desk_case_side_{y}', (0, y, .598), (.548, .016, .254), p['oak'])
    panel(fixed, 'desk_leg_clearance_filler', (-.305, .2075, .59), (.025, .055, .245), p['oak'])
    moving = assembly('desk_drawer', (0, -.04, .59), 2.5)
    panel(moving, 'desk_drawer_front', (-.305, -.04, .59), (.025, .395, .245), p['oak'])
    panel(moving, 'desk_drawer_pull', (-.325, -.04, .66), (.015, .18, .013), p['dark'])
    # Shallow tray also clears the retained cable organizer behind the desktop.
    tray(moving, 'desk_drawer_bin_0', (-.005, -.04, .482), (.53, .36, .11), p['oak'])
    moving['joint'] = {'kind':'prismatic', 'parent':'desk_shell',
        'parent_anchor':runtime(moving['origin']), 'child_anchor':[0, 0, 0],
        'axis':[-1, 0, 0], 'limits_m':[0, .35]}
    return [fixed, moving], [o.name for o in objects]


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        requested.objects = [n for n in available.objects if n.startswith('Workspace detail ')]
    palette = {'oak':bpy.data.materials['Kitchen light oak veneer'],
               'dark':bpy.data.materials['Workspace dark bronze']}
    parts, names = desk_parts(palette)
    bpy.context.view_layer.update()
    mechanism = {'id':'desk_drawer', 'placement':runtime(BASE),
        'replaces_solid':'workspace_desk', 'replaces_nodes':names, 'parts':export_parts(parts, OUT)}
    scene = json.loads((SCENE/'scene-kitchen-drawers.json').read_text())
    scene['id'] += '-desk'
    scene['mechanisms'].append(mechanism)
    for cloth in scene['cloths']:
        cloth['collision_solids'].remove('workspace_desk')
        cloth['collision_fixed_parts'].append('desk_shell')
    scene['cameras'].append({'id':'desk-drawer', 'position':[-.45, 1.15, -4.5],
        'look_at':[1.4, .6, -3.85], 'fov_deg':55.})
    (SCENE/'scene-desk-drawer.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT/'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
    studio = inspection_studio((-1.9, -2.5, 1.6), (-.05, -.60, .40), 2.6,
                               ((-1.5, -2., 3.), 350, 3.))
    studio.render.filepath = str(output/'desk-closed.png')
    bpy.ops.render.render(write_still=True)
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'desk-drawer.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    (output/'capture.json').write_text(json.dumps({'scope':__doc__, 'mechanism':mechanism,
        'source_sha256':before, 'limits':['Closed authoring preview only.',
        'Estimated 2.5 kg empty mass; ideal passive rail. Grip zones owned by functionality agent.']}, indent=2)+'\n')
    print('DESK_DRAWER_EXPORTED', flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
