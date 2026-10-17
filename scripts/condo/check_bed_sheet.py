"""Standalone, unpinned top-sheet gravity diagnostic; no avatar or runtime binding.

Run in Blender with --python-exit-code 1 --python this_file -- NEW_OUTPUT_DIR.
The condo source stays unchanged. Material IDs survive the simulation.
"""
import hashlib
import json
import math
import os
from pathlib import Path
import sys

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'scenes/toronto_condo/toronto-condo.blend'
COLUMNS, ROWS = 32, 36
END_FRAME = 241
TOTAL_MASS_KG = .8
SUPPORT_DISTANCE_M = .008
# Prototype coefficients for this grid/mass, not measured linen properties.
CLOTH_PARAMETERS = {
    'quality': 48,
    'tension_stiffness': 80,
    'compression_stiffness': 80,
    'shear_stiffness': 40,
    'bending_stiffness': .001,
    'tension_damping': .01,
    'compression_damping': .01,
    'shear_damping': .01,
    'bending_damping': .0005,
    'air_damping': .01,
}


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def load_mattress():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    with bpy.data.libraries.load(str(SOURCE), link=False) as (available, requested):
        name = 'Bedroom detail mattress'
        assert name in available.objects
        requested.objects = [name]
    mattress = requested.objects[0]
    bpy.context.collection.objects.link(mattress)
    mattress.modifiers.new('Sheet collision', 'COLLISION')
    mattress.collision.thickness_outer = .003
    mattress.collision.cloth_friction = 40
    return mattress


def sheet_mesh():
    points = [(3.05 + 2.3*c/COLUMNS, 2.0 + 2.3*r/ROWS, .80)
              for r in range(ROWS+1) for c in range(COLUMNS+1)]
    faces = []
    for r in range(ROWS):
        for c in range(COLUMNS):
            a = r*(COLUMNS+1)+c
            faces.append((a, a+1, a+COLUMNS+2, a+COLUMNS+1))
    mesh = bpy.data.meshes.new('Top sheet material grid')
    mesh.from_pydata(points, [], faces)
    mesh.update()
    uv = mesh.uv_layers.new(name='MaterialUV')
    for loop in mesh.loops:
        row, column = divmod(loop.vertex_index, COLUMNS+1)
        uv.data[loop.index].uv = (column/COLUMNS, row/ROWS)
    sheet = bpy.data.objects.new('Separate top sheet prototype', mesh)
    bpy.context.collection.objects.link(sheet)
    material = bpy.data.materials.new('Sheet ivory diagnostic')
    material.diffuse_color = (.72, .66, .53, 1)
    sheet.data.materials.append(material)
    for face in mesh.polygons:
        face.use_smooth = True
    return sheet


def configure_cloth(sheet):
    cloth = sheet.modifiers.new('Unpinned cloth', 'CLOTH')
    settings = cloth.settings
    settings.mass = TOTAL_MASS_KG / len(sheet.data.vertices)
    for name, value in CLOTH_PARAMETERS.items():
        setattr(settings, name, value)
    collision = cloth.collision_settings
    collision.use_collision = True
    collision.distance_min = .003
    collision.collision_quality = 8
    collision.use_self_collision = True
    collision.self_distance_min = .003
    cloth.point_cache.frame_start = 1
    cloth.point_cache.frame_end = END_FRAME
    return cloth


def material_surface(sheet):
    mesh = sheet.data
    mesh.calc_loop_triangles()
    points = [v.co.copy() for v in mesh.vertices]
    triangles = [list(t.vertices) for t in mesh.loop_triangles]
    edges = sorted({tuple(sorted((t[i], t[(i+1)%3]))) for t in triangles for i in range(3)})
    return {'schema_version': 1, 'id': 'bed_top_sheet',
            'coordinates': 'runtime metres: X east, Y up, Z south',
            'positions': [[v.x, v.z, -v.y] for v in points],
            'triangles': triangles, 'edges': edges,
            'rest_edge_lengths_m': [(points[a]-points[b]).length for a,b in edges],
            'material_uv': [[c/COLUMNS,r/ROWS] for r in range(ROWS+1) for c in range(COLUMNS+1)],
            'edge_candidates': {
                'foot': list(range(COLUMNS+1)),
                'head': list(range(ROWS*(COLUMNS+1), len(points))),
                'west': [r*(COLUMNS+1) for r in range(ROWS+1)],
                'east': [r*(COLUMNS+1)+COLUMNS for r in range(ROWS+1)]},
            'pinned_vertices': [], 'total_mass_kg': TOTAL_MASS_KG, 'runtime_enabled': False,
            'limits': ['Flat rest surface; gravity fixture only, no fitted sheet or duvet coupling.',
                       'Boundary candidates do not establish exposed or reachable grasp regions.',
                       'No runtime cloth solver, hand contact, grasp or avatar manipulation.']}


def studio(scene):
    bpy.ops.object.camera_add(location=(7.6, -.8, 3.6))
    camera = bpy.context.object
    camera.rotation_euler = (Vector((4.2,3.25,.40))-camera.location).to_track_quat('-Z','Y').to_euler()
    camera.data.type = 'ORTHO'
    camera.data.ortho_scale = 4.7
    scene.camera = camera
    bpy.ops.object.light_add(type='AREA', location=(3,1,5))
    bpy.context.object.data.energy = 500
    bpy.context.object.data.size = 4
    scene.world = bpy.data.worlds.new('Cloth diagnostic studio')
    scene.world.color = (.18,.18,.18)
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.samples = 16
    scene.render.resolution_x = 1280
    scene.render.resolution_y = 720
    scene.render.resolution_percentage = 100


def observe(sheet, mattress_bvh, surface, previous, cloth_name='Unpinned cloth'):
    deps = bpy.context.evaluated_depsgraph_get()
    evaluated = sheet.evaluated_get(deps)
    points = [v.co.copy() for v in evaluated.data.vertices]
    assert len(points) == len(surface['positions']), 'Material topology changed'
    assert all(math.isfinite(x) for v in points for x in v), 'Nonfinite cloth'
    stretch = max((points[a]-points[b]).length/rest
                  for (a,b),rest in zip(surface['edges'],surface['rest_edge_lengths_m']))
    penetration = 0.0
    supported = 0
    for point in points:
        hit, normal, _, distance = mattress_bvh.find_nearest(point)
        if (point-hit).dot(normal) < 0:
            penetration = max(penetration, distance)
        if normal.z > .8 and distance < SUPPORT_DISTANCE_M:
            supported += 1
    speed = max((a-b).length*24 for a,b in zip(points,previous)) if previous else 0
    metrics = {'max_edge_stretch_ratio': stretch, 'max_vertex_penetration_m': penetration,
               'max_vertex_speed_m_s': speed, 'lowest_vertex_m': min(v.z for v in points),
               'mattress_support_fraction': supported / len(points)}
    result = evaluated.modifiers[cloth_name].solver_result
    if result is not None:
        metrics['solver_status'] = sorted(result.status)
        metrics['solver_max_error'] = result.max_error
        metrics['solver_max_iterations'] = result.max_iterations
    return points, metrics


def check(out):
    out.mkdir(parents=True, exist_ok=False)
    source_hash = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    mattress = load_mattress()
    sheet = sheet_mesh()
    cloth = configure_cloth(sheet)
    write_json(out/'configuration.json', {
        'cloth': {name: getattr(cloth.settings,name) for name in CLOTH_PARAMETERS},
        'mass_per_vertex_kg': cloth.settings.mass,
        'mattress_friction': mattress.collision.cloth_friction,
        'support_distance_m': SUPPORT_DISTANCE_M,
        'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'script_source': Path(__file__).read_text(),
    })
    surface = material_surface(sheet)
    write_json(out/'top-sheet-surface.json', surface)
    scene = bpy.context.scene
    scene.frame_end = END_FRAME
    scene.render.fps = 24
    scene.gravity = (0,0,-9.81)
    studio(scene)
    bpy.context.view_layer.update()
    evaluated = mattress.evaluated_get(bpy.context.evaluated_depsgraph_get())
    bvh = BVHTree.FromPolygons(
        [evaluated.matrix_world @ v.co for v in evaluated.data.vertices],
        [list(f.vertices) for f in evaluated.data.polygons])
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'top-sheet-fixture.blend'))
    samples, previous = [], None
    for frame in range(1,END_FRAME+1):
        scene.frame_set(frame)
        previous, metrics = observe(sheet, bvh, surface, previous)
        samples.append({'frame': frame, **metrics})
        if frame in (1,121,END_FRAME):
            scene.render.filepath = str(out/f'sheet-{frame:04d}.png')
            bpy.ops.render.render(write_still=True)
        if frame % 24 == 0:
            print('CLOTH_FRAME', frame, metrics, flush=True)
    write_json(out/'final-positions.json', [[v.x,v.z,-v.y] for v in previous])
    result = {'scope': 'Blender gravity diagnostic only; no avatar, grips, pins or runtime cloth.',
              'blender_version': bpy.app.version_string, 'source_sha256': source_hash,
              'source_unchanged': hashlib.sha256(SOURCE.read_bytes()).hexdigest() == source_hash,
              'vertices': len(previous), 'triangles': len(surface['triangles']),
              'vertex_mass_kg': cloth.settings.mass, 'total_mass_kg': cloth.settings.mass*len(previous),
              'samples': samples,
              'limits': ['Vertex penetration only; not a continuous triangle collision or self-intersection certificate.',
                         'Support fraction counts vertices near an upward surface; it does not measure contact forces.',
                         'Material coefficients are prototype values, not calibrated fabric properties.']}
    result['pass'] = (result['source_unchanged'] and
                      max(s['max_vertex_penetration_m'] for s in samples) < .003 and
                      max(s['max_edge_stretch_ratio'] for s in samples) < 1.10 and
                      max(s['max_vertex_speed_m_s'] for s in samples[-12:]) < .05 and
                      min(s['mattress_support_fraction'] for s in samples[-12:]) > .4 and
                      all(s.get('solver_status') == ['SUCCESS'] for s in samples[1:]))
    write_json(out/'cloth-validation.json', result)
    print('CLOTH_RESULT', {k:v for k,v in result.items() if k != 'samples'}, flush=True)
    assert result['pass'], 'Cloth diagnostic failed; evidence retained'


if __name__ == '__main__':
    check(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
