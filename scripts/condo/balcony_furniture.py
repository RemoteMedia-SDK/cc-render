"""Original slatted outdoor lounge furniture, authored in metres in Blender."""
import math
import json
import os
from pathlib import Path
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from scene_export import update_saved_environments
from tabletop_props import box, finish, lathe, material
from upholstery import NAME as FABRIC_NAME, unwrap_cushions

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
PREFIX = 'Outdoor lounge '


def timber_material():
    surface = material('Outdoor warm timber', (1, 1, 1), .60)
    nodes, links = surface.node_tree.nodes, surface.node_tree.links
    nodes.clear()
    shader = nodes.new('ShaderNodeBsdfPrincipled')
    shader.inputs['Roughness'].default_value = .60
    output = nodes.new('ShaderNodeOutputMaterial')
    links.new(shader.outputs['BSDF'], output.inputs['Surface'])
    colour = nodes.new('ShaderNodeTexImage')
    colour.image = bpy.data.images.load(str(ROOT / 'models/wood_floor/wood_floor_diff_2k.jpg'), check_existing=True)
    links.new(colour.outputs['Color'], shader.inputs['Base Color'])
    texture = nodes.new('ShaderNodeTexImage')
    texture.image = bpy.data.images.load(str(ROOT / 'wood_floor_normal_soft.png'), check_existing=True)
    texture.image.colorspace_settings.name = 'Non-Color'
    normal = nodes.new('ShaderNodeNormalMap')
    links.new(texture.outputs['Color'], normal.inputs['Color'])
    links.new(normal.outputs['Normal'], shader.inputs['Normal'])
    surface['source'] = 'https://polyhaven.com/a/wood_floor'
    surface['license'] = 'CC0-1.0'
    return surface


def timber_uv(obj):
    # Sample inside unbroken boards; grain runs along each beam's local Z.
    # One source tile represents 1.7 metres. Avoid seams in the floor texture.
    centre_u = (.055, .388)[sum(obj.name.encode()) % 2]
    length = max(v.co.z for v in obj.data.vertices) - min(v.co.z for v in obj.data.vertices)
    uv = obj.data.uv_layers[0].data
    for face in obj.data.polygons:
        for index in face.loop_indices:
            point = obj.data.vertices[obj.data.loops[index].vertex_index].co
            transverse = point.y if abs(face.normal.x) > .5 else point.x
            along = point.y if abs(face.normal.z) > .5 else point.z
            uv[index].uv = (centre_u + transverse / 1.7, .2 + (along + length / 2) / 1.7)
    obj['timber_source'] = 'Poly Haven wood_floor, CC0; interior board patches at 1.7 m scale'


def timber(name, start, end, width, collection):
    delta = Vector(end) - Vector(start)
    obj = box(PREFIX + name, (Vector(start) + Vector(end)) / 2,
              (width, width, delta.length), bpy.data.materials['Outdoor warm timber'], collection)
    obj.rotation_euler = delta.to_track_quat('Z', 'Y').to_euler()
    obj['source'] = 'Original procedural balcony furniture'
    timber_uv(obj)
    return obj


def lounge_chair(collection, x):
    y = -6.1
    for dx in (-.35, .35):
        for dy in (-.31, .29):
            timber('leg', (x + dx, y + dy, -.02), (x + dx, y + dy, .36), .055, collection)
        timber('seat rail', (x + dx, y - .35, .33), (x + dx, y + .32, .33), .055, collection)
        timber('back post', (x + dx, y + .29, .33), (x + dx, y + .405, .88), .05, collection)
        timber('arm support', (x + dx, y - .26, .33), (x + dx, y - .26, .62), .04, collection)
        timber('armrest', (x + dx, y - .37, .62), (x + dx, y + .36, .62), .065, collection)
    for dy in (-.30, -.18, -.06, .06, .18, .30):
        timber('seat slat', (x - .35, y + dy, .35), (x + .35, y + dy, .35), .075, collection)
    for z in (.43, .53, .63, .73, .83):
        dy = .29 + (z - .33) * .115 / .55
        timber('back slat', (x - .35, y + dy, z), (x + .35, y + dy, z), .065, collection)
    fabric = bpy.data.materials[FABRIC_NAME]
    seat = box(PREFIX + 'seat cushion', (x, y - .015, .4375), (.64, .65, .10), fabric, collection)
    back = box(PREFIX + 'back cushion', (x, y + .235, .65), (.64, .11, .39), fabric, collection)
    back.rotation_euler.x = math.radians(-12)
    for obj in (seat, back):
        obj.modifiers['Soft manufactured edge'].width = .035
        obj.modifiers['Soft manufactured edge'].segments = 6
        obj['source'] = 'Original procedural balcony furniture; CC0 Terlenka fabric'
        unwrap_cushions(obj, 0)


def side_table(collection):
    x, y = 2.6, -6.3
    for angle in (0, math.tau / 3, 2 * math.tau / 3):
        direction = Vector((math.cos(angle), math.sin(angle), 0))
        timber('table leg', Vector((x, y, -.02)) + direction * .26,
               Vector((x, y, .50)) + direction * .19, .045, collection)
    top = bpy.data.objects.new(PREFIX + 'table top', lathe([
        (0, .495), (.305, .495), (.32, .505), (.32, .535), (.305, .545), (0, .545)]))
    collection.objects.link(top)
    finish(top, top.name, bpy.data.materials['Pale limestone'], collection)
    top.location = (x, y, 0)
    top['source'] = 'Original procedural balcony furniture'


def replace_balcony_furniture():
    collection = next(c for c in bpy.data.collections if c.name.startswith('02 '))
    for obj in list(collection.objects):
        if obj.name.startswith(('Balcony chair', 'Balcony table', PREFIX)):
            bpy.data.objects.remove(obj, do_unlink=True)
    timber_material()
    for x in (-3.8, 3.8):
        previous = set(collection.objects)
        lounge_chair(collection, x)
        group = 'balcony_lounge_west' if x < 0 else 'balcony_lounge_east'
        for obj in set(collection.objects) - previous:
            obj['furniture_id'] = group
    previous = set(collection.objects)
    side_table(collection)
    for obj in set(collection.objects) - previous:
        obj['furniture_id'] = 'balcony_side_table'
    bpy.context.view_layer.update()
    objects = [o for o in collection.objects if o.name.startswith(PREFIX)]
    assert len(objects) == 54, len(objects)
    # Furniture stays outside the central two-metre doorway and walking route.
    for obj in objects:
        assert min(abs((obj.matrix_world @ Vector(c)).x) for c in obj.bound_box) > 2
    print('BALCONY_FURNITURE_AUTHORED', len(objects), flush=True)


def balcony_solids():
    """Conservative navigation boxes, not seating/contact support declarations."""
    result = []
    for group in ('balcony_lounge_west', 'balcony_lounge_east', 'balcony_side_table'):
        objects = [o for o in bpy.data.objects if o.get('furniture_id') == group]
        assert objects, group
        corners = [o.matrix_world @ Vector(c) for o in objects for c in o.bound_box]
        low = Vector(tuple(min(c[i] for c in corners) for i in range(3)))
        high = Vector(tuple(max(c[i] for c in corners) for i in range(3)))
        center, half = (low + high) / 2, (high - low) / 2
        result.append({'id': group, 'position': [center.x, center.z, -center.y],
                       'half_extents': [half.x, half.z, half.y],
                       'navigation_obstacle': True, 'approach': [0, 0, half.y + .4]})
    return result


def update_collision_manifests():
    solids = balcony_solids()
    for filename in ('scene.json', 'scene-lightmap.json'):
        path = ROOT / filename
        package = json.loads(path.read_text())
        ids = {s['id'] for s in solids}
        package['solids'] = [s for s in package['solids'] if s['id'] not in ids] + solids
        # Keep the physical ceiling box, but not as a full-balcony XZ obstacle.
        overhang = next(s for s in package['solids'] if s['id'] == 'balcony_overhang')
        assert overhang['position'][1] - overhang['half_extents'][1] >= 2.8
        overhang['navigation_obstacle'] = False
        path.write_text(json.dumps(package, indent=2) + '\n')
    print('BALCONY_COLLIDERS_EXPORTED', len(solids), flush=True)


if __name__ == '__main__':
    update_saved_environments(ROOT, replace_balcony_furniture)
    update_collision_manifests()
    os._exit(0)
