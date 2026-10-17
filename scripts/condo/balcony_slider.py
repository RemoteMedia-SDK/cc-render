"""Author passive balcony glazing on a horizontal rail; no avatar motion."""
import hashlib
import json
import os
from pathlib import Path
import struct
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from fridge_mechanism import assembly, panel, runtime
from bedside_mechanism import export_parts, inspection_studio, SCENE, SOURCE

OUT = SCENE / 'interaction-authoring/balcony-slider'


def glazing(part, center, palette):
    x, y, z = center
    panel(part, part['id']+'_glass', center, (1.86, .015, 2.64), palette['glass'])
    for side in (-1, 1):
        panel(part, part['id']+f'_stile_{side}', (x+side*.955, y, z), (.05, .055, 2.74), palette['dark'])
        panel(part, part['id']+f'_rail_{side}', (x, y, z+side*1.345), (1.86, .055, .05), palette['dark'])


def parts(palette):
    retained = assembly('balcony_retained_pane', (0, 0, 0), None)
    panel(retained, 'balcony_original_right_glass', (3., 0, 1.45), (1.94, .015, 2.84), palette['glass'])
    fixed = assembly('balcony_fixed_panel', (1., -.02, 1.5), None)
    glazing(fixed, fixed['origin'], palette)
    leaf = assembly('balcony_sliding_door', (-1., .10, 1.5), 45.)
    glazing(leaf, leaf['origin'], palette)
    handle = panel(leaf, 'balcony_interior_pull', (-1.86, .18, 1.08), (.025, .025, .32), palette['dark'])
    for height in (.94, 1.22):
        panel(leaf, f'balcony_pull_mount_{height}', (-1.86, .15, height), (.025, .045, .025), palette['dark'])
    leaf['handle_region'] = {'mesh':handle.name, 'centre':runtime(handle.location),
                             'half_extents':[.0125, .16, .0125]}
    leaf['joint'] = {'kind':'prismatic', 'parent':retained['id'],
                     'parent_anchor':runtime(leaf['origin']), 'child_anchor':[0, 0, 0],
                     'axis':[1, 0, 0], 'limits_m':[0, 2.]}
    return [retained, fixed, leaf]


def package(assemblies):
    scene = json.loads((SCENE / 'scene-doors.json').read_text())
    with (SCENE / scene['geometry']).open('rb') as stream:
        stream.read(12)
        size, kind = struct.unpack('<II', stream.read(8))
        assert kind == 0x4E4F534A
        gltf = json.loads(stream.read(size))
    node = 'South glass pane.002'
    assert sum(n.get('name') == node for n in gltf['nodes']) == 1
    mechanism = {'id':'balcony_slider', 'placement':[0, 0, 5],
                 'replaces_solid':'south_glass_pane_002', 'replaces_nodes':[node],
                 'parts':export_parts(assemblies, OUT)}
    scene['id'] += '-balcony-slider'
    scene['mechanisms'].append(mechanism)
    for cloth in scene['cloths']:
        if 'south_glass_pane_002' in cloth['collision_solids']:
            cloth['collision_solids'].remove('south_glass_pane_002')
        cloth['collision_fixed_parts'].extend(['balcony_retained_pane', 'balcony_fixed_panel'])
    scene['cameras'].append({'id':'balcony-door', 'position':[-3., 1.65, 2.7],
                             'look_at':[.1, 1.3, 5.1], 'fov_deg':72.})
    (SCENE / 'scene-balcony-slider.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT / 'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
    return mechanism


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    names = {'glass':'Glazing placeholder', 'dark':'Dark bronze'}
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        assert all(name in available.materials for name in names.values())
        requested.materials = list(names.values())
    assemblies = parts({key:bpy.data.materials[name] for key, name in names.items()})
    bpy.context.view_layer.update()
    mechanism = package(assemblies)
    scene = inspection_studio((-4., 6., 3.5), (.7, 0, 1.4), 7.2, ((-2., 4., 5.), 850, 4.))
    scene.render.filepath = str(output / 'closed.png')
    bpy.ops.render.render(write_still=True)
    scene['scope'] = __doc__
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'balcony-slider.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    (output / 'capture.json').write_text(json.dumps({
        'scope':__doc__, 'source_sha256':before, 'mechanism':mechanism,
        'limits':['Estimated 45 kg mass and ideal passive rail; no latch or roller model.',
                  'Existing sill remains; no avatar threshold crossing or manipulation proof.',
                  'Interior handle only; dynamic cloth contact remains unsupported.']}, indent=2)+'\n')
    print('BALCONY_SLIDER_EXPORTED', flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
