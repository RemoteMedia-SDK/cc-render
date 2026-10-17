"""Hollow vanity drawers on passive rails, including basin clearance in the upper drawer."""
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import sys

import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).parent))
from fridge_mechanism import assembly, panel, runtime
from bedside_mechanism import export_parts, inspection_studio, SCENE, SOURCE

OUT = SCENE / 'interaction-authoring/vanity'


def shell(p):
    fixed = assembly('vanity_shell', (0, 0, 0), None)
    panel(fixed, 'vanity_bottom', (0, 0, .30), (.65, 1.10, .035), p['oak'])
    panel(fixed, 'vanity_back', (-.31, 0, .56), (.025, 1.10, .52), p['oak'])
    for y in (-.54, .54):
        panel(fixed, f'vanity_side_{y}', (0, y, .56), (.65, .025, .52), p['oak'])
    # Retained source meshes render the counter and basin. Conservative fixed
    # collision envelopes reserve their volume; the basin is not a usable cavity.
    for name, center, size in [
        ('vanity_retained_counter', (.02, 0, .86), (.72, 1.16, .045)),
        ('vanity_retained_basin', (.07, .02, .781), (.441, .58, .212)),
    ]:
        fixed['colliders'].append({'id':name, 'position':runtime(center),
            'half_extents':[size[0]/2, size[2]/2, size[1]/2]})
    return fixed


def drawer(name, height, upper, p):
    part = assembly(name, (0, 0, height), 5.)
    panel(part, name+'_front', (.342, 0, height), (.026, 1.07, .242), p['oak'])
    handle = panel(part, name+'_pull', (.358, 0, height+.096), (.012, .90, .012), p['dark'])
    # Two side bins leave a through-notch around the retained basin. The lower
    # drawer has a full-width tray; both keep real open storage cavities.
    bins = [(-.43, .18), (.43, .18)] if upper else [(0., 1.04)]
    for index, (y, width) in enumerate(bins):
        prefix = f'{name}_bin_{index}'
        panel(part, prefix+'_floor', (.01, y, height-.098), (.56, width, .016), p['oak'])
        for side in (-1, 1):
            panel(part, prefix+f'_side_{side}', (.01, y+side*(width/2-.008), height-.018),
                  (.56, .016, .16), p['oak'])
        for x in (-.262, .282):
            panel(part, prefix+f'_end_{x}', (x, y, height-.018), (.016, width-.032, .16), p['oak'])
    part['joint'] = {'kind':'prismatic', 'parent':'vanity_shell',
        'parent_anchor':runtime(part['origin']), 'child_anchor':[0, 0, 0],
        'axis':[1, 0, 0], 'limits_m':[0, .40]}
    part['handle_region'] = {'mesh':handle.name, 'centre':runtime(handle.location),
        'half_extents':[.006, .006, .45]}
    return part


def accessories():
    """Move the blocking towel to the east wall; retain exact soap mesh bounds."""
    fixed = assembly('vanity_wall_towel', (0, 0, 0), None)
    names = ['Bathroom detail '+n for n in ('folded hand towel', 'towel rail', 'soap dispenser', 'soap pump')]
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        assert all(n in available.objects for n in names)
        requested.objects = list(names)
    solids = []
    shift = Matrix.Translation(Vector((-8.095, 5.33, .60))) @ Matrix.Rotation(math.pi, 4, 'Z')
    local = Matrix.Translation(Vector((5.58, -2.83, 0)))
    for index, name in enumerate(names):
        obj = bpy.data.objects[name]
        if index < 2:
            fixed['collection'].objects.link(obj)
            obj.matrix_world = local @ shift @ obj.matrix_world
            obj.parent = fixed['pivot']
            fixed['meshes'].append(obj)
        corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
        low = Vector(tuple(min(c[i] for c in corners) for i in range(3)))
        high = Vector(tuple(max(c[i] for c in corners) for i in range(3)))
        center, half = (low+high)/2, (high-low)/2
        row = {'id':'vanity_accessory_'+str(index), 'position':runtime(center),
               'half_extents':[half.x, half.z, half.y]}
        if index < 2:
            fixed['colliders'].append(row)
        else:
            solids.append({**row, 'navigation_obstacle':False})
    return fixed, names[:2], solids


def package(parts, moved_nodes, accessory_solids):
    scene = json.loads((SCENE/'scene-bathroom-door.json').read_text())
    with (SCENE/scene['geometry']).open('rb') as stream:
        stream.read(12)
        size, kind = struct.unpack('<II', stream.read(8))
        assert kind == 0x4E4F534A
        gltf = json.loads(stream.read(size))
    nodes = [n['name'] for n in gltf['nodes'] if n.get('name', '').startswith('Bathroom detail vanity ')]
    assert len(nodes) == 8
    mechanism = {'id':'vanity_drawers', 'placement':[-5.58, 0, -2.83],
        'replaces_solid':'vanity', 'replaces_nodes':nodes+moved_nodes, 'parts':export_parts(parts, OUT)}
    scene['solids'] = [s for s in scene['solids'] if s['id'] != 'bathroom_accessories'] + accessory_solids
    scene['mechanisms'].append(mechanism)
    scene['id'] += '-vanity-drawers'
    for cloth in scene['cloths']:
        cloth['collision_solids'].remove('vanity')
        cloth['collision_solids'].remove('bathroom_accessories')
        cloth['collision_solids'].extend(s['id'] for s in accessory_solids)
        cloth['collision_fixed_parts'].append('vanity_shell')
        cloth['collision_fixed_parts'].append('vanity_wall_towel')
    scene['cameras'].append({'id':'vanity-drawers', 'position':[-3.2, 1.35, -3.5],
        'look_at':[-5.5, .65, -2.83], 'fov_deg':58.})
    (SCENE/'scene-vanity-drawers.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT/'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
    return mechanism


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    names = {'oak':'Kitchen light oak veneer', 'dark':'Bathroom shadow reveals'}
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        assert all(n in available.materials for n in names.values())
        requested.materials = list(names.values())
    p = {key:bpy.data.materials[value] for key, value in names.items()}
    parts = [shell(p), drawer('vanity_lower_drawer', .435, False, p),
             drawer('vanity_upper_drawer', .695, True, p)]
    towel, moved_nodes, accessory_solids = accessories()
    parts.append(towel)
    bpy.context.view_layer.update()
    mechanism = package(parts, moved_nodes, accessory_solids)
    scene = inspection_studio((2.2, -2., 1.8), (.15, 0, .55), 2.0, ((2., -1., 3.), 240, 2.))
    for name, travel in [('closed', 0.), ('open-inspection', .4)]:
        for part in parts:
            if 'joint' not in part:
                continue
            part['pivot'].location.x = travel
        scene.render.filepath = str(output/(name+'.png'))
        bpy.ops.render.render(write_still=True)
    for part in parts:
        if 'joint' in part:
            part['pivot'].location.x = 0.
    bpy.context.view_layer.update()
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'vanity-drawers.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    report = {'scope':__doc__, 'source_sha256':before, 'mechanism':mechanism,
        'limits':['Static inspection poses only; physics verified separately.',
                  'Estimated 5 kg drawers, ideal passive rails; no soft-close or latch.',
                  'Retained counter/basin are absent from this isolated studio render.',
                  'Conservative basin collider reserves solid volume, not a usable sink cavity.']}
    (output/'capture.json').write_text(json.dumps(report, indent=2)+'\n')
    print('VANITY_EXPORTED', flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
