"""Author hollow bedside drawers and an opt-in package; never avatar motion.

Blender --background --python-exit-code 1 --python this_file -- NEW_PREVIEW_DIR
Open poses below are labelled geometry inspections, not simulated interactions.
"""
import hashlib
import json
import os
from pathlib import Path
import struct
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from fridge_mechanism import assembly, panel, runtime
from scene_export import export_selection

ROOT = Path(__file__).resolve().parents[2]
SCENE = ROOT / 'scenes/toronto_condo'
SOURCE = SCENE / 'toronto-condo.blend'
OUT = SCENE / 'interaction-authoring/bedside'


def shell(palette):
    part = assembly('bedside_shell', (0, 0, 0), None)
    for x in (-.258, .258):
        panel(part, f'bedside_side_{x}', (x, 0, .335), (.024, .60, .53), palette['oak'])
    panel(part, 'bedside_back', (0, .288, .335), (.492, .024, .53), palette['oak'])
    panel(part, 'bedside_bottom', (0, 0, .082), (.492, .552, .024), palette['oak'])
    panel(part, 'bedside_divider', (0, 0, .355), (.492, .552, .016), palette['oak'])
    panel(part, 'bedside_plinth', (0, 0, .045), (.43, .49, .09), palette['dark'])
    panel(part, 'bedside_stone_top', (0, 0, .62), (.57, .63, .04), palette['stone'])
    return part


def drawer(name, height, palette):
    part = assembly(name, (0, 0, height), 3.5)
    panel(part, name+'_front', (0, -.314, height), (.518, .025, .225), palette['oak'])
    panel(part, name+'_floor', (0, -.02, height-.098), (.476, .55, .016), palette['oak'])
    for x in (-.230, .230):
        panel(part, name+f'_side_{x}', (x, -.02, height-.018), (.016, .55, .16), palette['oak'])
    for y in (-.293, .247):
        panel(part, name+f'_end_{y}', (0, y, height-.018), (.444, .016, .16), palette['oak'])
    handle = panel(part, name+'_pull', (0, -.335, height+.075), (.18, .018, .012), palette['dark'])
    part['joint'] = {'kind':'prismatic', 'parent':'bedside_shell',
                     'parent_anchor':runtime(part['origin']), 'child_anchor':[0, 0, 0],
                     'axis':[0, 0, 1], 'limits_m':[0, .4]}
    part['handle_region'] = {'mesh':handle.name, 'centre':runtime(handle.location),
                            'half_extents':[.09, .006, .009]}
    return part


def export_parts(parts, destination=OUT):
    rows = []
    for part in parts:
        origin = part['pivot'].location.copy()
        part['pivot'].location = (0, 0, 0)
        bpy.context.view_layer.update()
        path = destination / (part['id']+'.glb')
        export_selection(path, [part['collection']])
        part['pivot'].location = origin
        bpy.context.view_layer.update()
        row = {'id':part['id'], 'geometry':str(path.relative_to(SCENE)),
               'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
               'position':runtime(part['origin']), 'mass_kg':part['mass_kg'],
               'colliders':part['colliders']}
        for key in ('joint', 'handle_region'):
            if key in part:
                row[key] = part[key]
        rows.append(row)
    return rows


def package(parts):
    scene = json.loads((SCENE / 'scene-cloth.json').read_text())
    with (SCENE / scene['geometry']).open('rb') as stream:
        stream.read(12)
        size, kind = struct.unpack('<II', stream.read(8))
        assert kind == 0x4E4F534A
        gltf = json.loads(stream.read(size))
    nodes = [n['name'] for n in gltf['nodes']
             if n.get('name', '').startswith('Bedroom detail bedside ')]
    assert len(nodes) == 7, 'Review bedside replacement geometry'
    mechanism = {'id':'bedside_drawers', 'placement':[5.52, 0, -4.18],
                 'replaces_solid':'bedside_unit', 'replaces_nodes':nodes, 'parts':export_parts(parts)}
    scene['id'] += '-drawers'
    scene['mechanisms'].append(mechanism)
    for cloth in scene['cloths']:
        cloth['collision_solids'].remove('bedside_unit')
        cloth['collision_fixed_parts'] = ['bedside_shell']
    (SCENE / 'scene-drawers.json').write_text(json.dumps(scene, indent=2)+'\n')
    (OUT / 'mechanism.json').write_text(json.dumps(mechanism, indent=2)+'\n')
    return mechanism


def inspection_studio(camera_position, target, framing, lighting):
    """Shared static asset/recorded-physics inspection setup, never runtime motion."""
    scene = bpy.context.scene
    bpy.ops.object.camera_add(location=camera_position)
    camera = bpy.context.object
    camera.rotation_euler = (Vector(target)-camera.location).to_track_quat('-Z', 'Y').to_euler()
    camera.data.type, camera.data.ortho_scale = 'ORTHO', framing
    scene.camera = camera
    position, energy, size = lighting
    bpy.ops.object.light_add(type='AREA', location=position)
    bpy.context.object.data.energy, bpy.context.object.data.size = energy, size
    scene.world = bpy.data.worlds.new('Furniture inspection studio')
    scene.world.color = (.2, .2, .2)
    scene.render.engine, scene.cycles.device = 'CYCLES', 'CPU'
    scene.cycles.samples = 24
    scene.render.resolution_x, scene.render.resolution_y = 1280, 960
    scene.render.resolution_percentage = 100
    return scene


def preview(parts, output):
    scene = inspection_studio((1.3, -1.6, 1.25), (0, -.12, .30), 1.75, ((.5, -1., 2.), 180, 2.))
    for name, travel in [('closed', 0.), ('open-inspection', .4)]:
        for part in parts[1:]:
            part['pivot'].location.y = -travel
        scene.render.filepath = str(output / (name+'.png'))
        bpy.ops.render.render(write_still=True)
    for part in parts[1:]:
        part['pivot'].location.y = 0.
    bpy.context.view_layer.update()


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    names = {'oak':'Kitchen light oak veneer', 'dark':'Bedroom shadow reveal',
             'stone':'Honed travertine - ambientCG Travertine009'}
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        assert all(name in available.materials for name in names.values())
        requested.materials = list(names.values())
    palette = {key:bpy.data.materials[name] for key, name in names.items()}
    parts = [shell(palette), drawer('bedside_lower_drawer', .23, palette),
             drawer('bedside_upper_drawer', .48, palette)]
    bpy.context.view_layer.update()
    mechanism = package(parts)
    preview(parts, output)
    scene = bpy.context.scene
    scene['scope'] = 'Hollow drawer asset authoring; no body motion or opening animation'
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'bedside-mechanism.blend'))
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == before
    report = {'scope':__doc__, 'source_sha256':before, 'mechanism':mechanism,
              'travel_m':.4, 'inspection_poses_only':True,
              'limits':['Ideal passive rail; drawer masses are initial estimates.',
                        'Open screenshot is a geometry inspection, not physics or avatar evidence.',
                        'Cloth contacts fixed shell only; dynamic drawer/cloth coupling is unsupported.']}
    (output / 'capture.json').write_text(json.dumps(report, indent=2)+'\n')
    print('BEDSIDE_EXPORTED', len(parts), flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
