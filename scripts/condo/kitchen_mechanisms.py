"""Author fifteen hollow kitchen/island drawers on passive rails; physics assets only."""
import hashlib
import json
import os
from pathlib import Path
import sys

import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).parent))
from fridge_mechanism import assembly, panel, runtime
from bedside_mechanism import export_parts, inspection_studio, SCENE, SOURCE

OUT = SCENE/'interaction-authoring/kitchen-drawers'


def retain_shell(kind, base):
    """Copy retained source geometry and its per-mesh bounds into a fixed part."""
    group = 'kitchen_cabinetry' if kind == 'kitchen' else 'island_cabinet'
    omitted = ('drawer front', 'drawer pull') if kind == 'kitchen' else ('island body', 'island drawer', 'island pull')
    fixed = assembly(kind+'_drawer_shell', (0, 0, 0), None)
    names = []
    for obj in list(bpy.data.objects):
        if obj.get('kitchen_group') != group:
            continue
        names.append(obj.name)
        if any(obj.name.startswith('Kitchen detail '+n) for n in omitted):
            continue
        fixed['collection'].objects.link(obj)
        bpy.context.view_layer.update()
        obj.matrix_world = Matrix.Translation(-Vector(base)) @ obj.matrix_world
        obj.parent = fixed['pivot']
        bpy.context.view_layer.update()
        corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
        low = Vector(tuple(min(c[i] for c in corners) for i in range(3)))
        high = Vector(tuple(max(c[i] for c in corners) for i in range(3)))
        half = (high-low)/2
        fixed['meshes'].append(obj)
        fixed['colliders'].append({'id':obj.name, 'position':runtime((high+low)/2),
            'half_extents':[half.x, half.z, half.y]})
    assert names and fixed['meshes']
    return fixed, names


def tray(part, name, center, size, material):
    """An open rectangular storage bin; center.z is the bottom surface."""
    x, y, bottom = center
    depth, width, height = size
    wall = .016
    panel(part, name+'_floor', (x, y, bottom+wall/2), (depth, width, wall), material)
    for sign in (-1, 1):
        panel(part, name+f'_side_{sign}', (x, y+sign*(width-wall)/2, bottom+wall+height/2),
              (depth, wall, height), material)
        panel(part, name+f'_end_{sign}', (x+sign*(depth-wall)/2, y, bottom+wall+height/2),
              (wall, width-2*wall, height), material)


def drawer(kind, column, row, y, height, face_height, p):
    name = f'{kind}_drawer_{column}_{row}'
    part = assembly(name, (0, y, height), 5. if kind == 'kitchen' else 7.)
    sign, front, width = (1, .417, .609) if kind == 'kitchen' else (-1, .517, .696)
    panel(part, name+'_front', (sign*front, y, height), (.026, width, face_height), p['oak'])
    panel(part, name+'_pull', (sign*(front+.026), y, height+face_height/2-.025),
          (.022, .22 if kind == 'kitchen' else .28, .012), p['steel'])
    bottom, wall_height = height-face_height/2+.035, face_height-.07
    if kind == 'kitchen':
        bins = [((.025, y, bottom), (.754, .58, wall_height))]
        if row == 2 and column in (1, 2):
            # Sink spans local Y .218–.832 and reaches X .292. Side storage
            # plus a shallow front pocket stay outside that retained geometry.
            side_y, side_width = (.12, .16) if column == 1 else (1.035, .35)
            pocket_y, pocket_width = (.40, .39) if column == 1 else (.77, .18)
            bins = [((.025, side_y, bottom), (.754, side_width, wall_height)),
                    ((.356, pocket_y, bottom), (.092, pocket_width, wall_height))]
    else:
        bins = [((-.05, y, bottom), (.90, .666, wall_height))]
    for index, (center, size) in enumerate(bins):
        tray(part, name+f'_bin_{index}', center, size, p['oak'])
    part['joint'] = {'kind':'prismatic', 'parent':kind+'_drawer_shell',
        'parent_anchor':runtime(part['origin']), 'child_anchor':[0, 0, 0],
        'axis':[sign, 0, 0], 'limits_m':[0, .4]}
    return part


def build_mechanism(kind, p):
    base = (-5.55, -.175, 0) if kind == 'kitchen' else (-3.55, 0, 0)
    fixed, names = retain_shell(kind, base)
    if kind == 'kitchen':
        panel(fixed, 'kitchen_missing_bay_divider', (0, .625, .44), (.8, .018, .58), p['oak'])
        columns, rows = (-.3125, .3125, .9375), ((.29, .285), (.59, .285), (.82, .145))
    else:
        for y in (-1.065, -.355, .355, 1.065):
            panel(fixed, f'island_internal_divider_{y}', (0, y, .505), (1., .018, .75), p['oak'])
        panel(fixed, 'island_internal_back', (.48, 0, .505), (.024, 2.112, .75), p['oak'])
        panel(fixed, 'island_internal_bottom', (0, 0, .15), (.95, 2.112, .024), p['oak'])
        columns, rows = (-.71, 0., .71), ((.30, .31), (.68, .42))
    parts = [fixed]
    for col, y in enumerate(columns):
        for row, (height, face_height) in enumerate(rows):
            parts.append(drawer(kind, col, row, y, height, face_height, p))
    bpy.context.view_layer.update()
    mechanism = {'id':kind+'_drawers', 'placement':runtime(base),
        'replaces_solid':'kitchen_cabinetry' if kind == 'kitchen' else 'island_cabinet',
        'replaces_nodes':names, 'parts':export_parts(parts, OUT)}
    return mechanism, parts


def package(mechanisms):
    scene = json.loads((SCENE/'scene-vanity-drawers.json').read_text())
    scene['id'] += '-kitchen-drawers'
    scene['mechanisms'].extend(mechanisms)
    for cloth in scene['cloths']:
        for m in mechanisms:
            cloth['collision_solids'].remove(m['replaces_solid'])
            cloth['collision_fixed_parts'].append(m['parts'][0]['id'])
    scene['cameras'].append({'id':'kitchen-drawers', 'position':[-3.95, 1.75, 2.4],
        'look_at':[-4.55, .55, .0], 'fov_deg':76.})
    (SCENE/'scene-kitchen-drawers.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT/'mechanisms.json').write_text(json.dumps(mechanisms, indent=2)+'\n')


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        requested.objects = [n for n in available.objects if n.startswith('Kitchen detail ')]
    p = {key:bpy.data.materials[name] for key, name in
         [('oak', 'Kitchen light oak veneer'), ('steel', 'Kitchen brushed steel')]}
    mechanisms = []
    for kind in ('kitchen', 'island'):
        mechanism, parts = build_mechanism(kind, p)
        mechanisms.append(mechanism)
        # Keep this mechanism alone in its studio for trace replay.
        for obj in bpy.context.scene.objects:
            obj.hide_render = all(obj not in part['meshes'] for part in parts)
        sign = 1 if kind == 'kitchen' else -1
        scene = inspection_studio((sign*3., -3.2, 2.8), (0, .05, .50), 3.4,
                                  ((sign*2., -2., 4.), 450, 3.))
        scene.render.filepath = str(output/(kind+'-closed.png'))
        bpy.ops.render.render(write_still=True)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(OUT/(kind+'-drawers.blend')))
    package(mechanisms)
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    (output/'capture.json').write_text(json.dumps({'scope':__doc__, 'source_sha256':before,
        'mechanisms':mechanisms, 'limits':['Closed authoring inspection only.',
        'Estimated masses and ideal passive rails. Grip zones belong to the functionality workstream.']}, indent=2)+'\n')
    print('KITCHEN_DRAWERS_EXPORTED', flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
