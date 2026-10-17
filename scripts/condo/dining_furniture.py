"""Original warm-modern dining chairs and table, authored in Blender metres."""
import math
import os
from pathlib import Path
import sys

import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).parent))
from scene_export import ensure_surface_uv, update_saved_environments
from tabletop_props import box, finish, lathe
from upholstery import NAME as FABRIC_NAME, unwrap_cushions

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
WOOD = 'Outdoor warm timber'
STONE = 'Honed travertine - ambientCG Travertine009'


def wood_uv(obj, staves=1):
    """Grain follows local Z, sampling continuous boards without floor joints."""
    ensure_surface_uv(obj)
    mesh = obj.data
    radius = max(math.hypot(v.co.x, v.co.y) for v in mesh.vertices)
    bottom = min(v.co.z for v in mesh.vertices)
    period = math.tau / staves
    for face in mesh.polygons:
        points = [mesh.vertices[mesh.loops[i].vertex_index].co for i in face.loop_indices]
        angles = [math.atan2(p.y, p.x) % period for p in points]
        if max(angles) - min(angles) > period / 2:
            angles = [a + period if a < period / 2 else a for a in angles]
        for index, point, angle in zip(face.loop_indices, points, angles):
            uv = (.01 + angle * radius / 1.7, .2 + (point.z - bottom) / 1.7)
            if abs(face.normal.z) > .5:
                uv = (.055 + point.x / 1.7, .5 + point.y / 1.7)
            mesh.uv_layers[0].data[index].uv = uv


def wooden_member(collection, start, end, radii):
    direction = Vector(end) - Vector(start)
    bpy.ops.mesh.primitive_cone_add(vertices=24, radius1=radii[0], radius2=radii[1],
        depth=direction.length, location=(Vector(start) + Vector(end)) / 2)
    obj = finish(bpy.context.object, 'Dining detail wood member', bpy.data.materials[WOOD], collection)
    wood_uv(obj)
    obj.rotation_euler = direction.to_track_quat('Z', 'Y').to_euler()
    for face in obj.data.polygons:
        face.use_smooth = len(face.vertices) == 4
    bevel = obj.modifiers.new('Small timber edge', 'BEVEL')
    bevel.width, bevel.segments = .0015, 2
    return obj


def curved_back(collection, fabric):
    columns, rows = 20, 12
    vertices, faces = [], []
    for side in (-1, 1):
        for row in range(rows + 1):
            z = .56 + .36 * row / rows
            corner = max(abs(z - .74) - (.18 - .055), 0)
            width = .24 - .055 + math.sqrt(max(.055 ** 2 - corner ** 2, 0))
            for column in range(columns + 1):
                x = width * (2 * column / columns - 1)
                y = .22 - .055 * (x / .24) ** 2 + (z - .65) * .15 + side * .03
                vertices.append((x, y, z))
    offset = (rows + 1) * (columns + 1)
    for row in range(rows):
        for column in range(columns):
            a = row * (columns + 1) + column
            face = (a, a + 1, a + columns + 2, a + columns + 1)
            faces.extend((face, tuple(i + offset for i in reversed(face))))
    rim = list(range(columns + 1))
    rim += [r * (columns + 1) + columns for r in range(1, rows + 1)]
    rim += [rows * (columns + 1) + c for c in reversed(range(columns))]
    rim += [r * (columns + 1) for r in reversed(range(1, rows))]
    for a, b in zip(rim, rim[1:] + rim[:1]):
        faces.append((b, a, a + offset, b + offset))
    mesh = bpy.data.meshes.new('Curved dining upholstery')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new('Dining detail curved back', mesh)
    collection.objects.link(obj)
    finish(obj, obj.name, fabric, collection)
    ensure_surface_uv(obj)
    for face in mesh.polygons:
        face.use_smooth = True
    bevel = obj.modifiers.new('Rounded upholstery perimeter', 'BEVEL')
    bevel.width, bevel.segments = .012, 3


def dining_chair(collection, index, outward):
    before = set(collection.objects)
    fabric = bpy.data.materials[FABRIC_NAME]
    seat = box('Dining detail seat', (0, -.015, .445), (.48, .48, .085), fabric, collection)
    seat.modifiers['Soft manufactured edge'].width = .035
    seat.modifiers['Soft manufactured edge'].segments = 6
    for x in (-1, 1):
        for y in (-1, 1):
            wooden_member(collection, (x * .235, y * .235, 0), (x * .19, y * .19, .415), (.016, .024))
        wooden_member(collection, (x * .19, -.19, .39), (x * .19, .19, .39), (.018, .018))
        wooden_member(collection, (-.19, x * .19, .39), (.19, x * .19, .39), (.018, .018))
        wooden_member(collection, (x * .18, .18, .4), (x * .18, .24, .78), (.018, .018))
    curved_back(collection, fabric)
    objects = list(set(collection.objects) - before)
    bpy.ops.object.select_all(action='DESELECT')
    for obj in objects:
        bpy.context.view_layer.objects.active = obj
        for modifier in list(obj.modifiers):
            bpy.ops.object.modifier_apply(modifier=modifier.name)
        obj.select_set(True)
    bpy.context.view_layer.objects.active = seat
    bpy.ops.object.join()
    seat.name = seat.data.name = f'Dining chair {index}'
    slot = next(i for i, m in enumerate(seat.data.materials) if m == fabric)
    unwrap_cushions(seat, slot)
    dx, dy = outward
    placement = Matrix.Translation((-3.7 + dx, -3.2 + dy, 0)) @ Matrix.Rotation(math.atan2(-dx, dy), 4, 'Z')
    seat.matrix_world = placement @ seat.matrix_world
    seat['source'] = 'Original curved-back dining chair; existing CC0 wood and upholstery maps'


def dining_table(collection):
    stone = bpy.data.materials[STONE]
    top = bpy.data.objects.new('Dining top', lathe([
        (0, .72), (.784, .72), (.797, .724), (.8, .735),
        (.8, .785), (.794, .796), (.784, .8), (0, .8)], segments=96))
    collection.objects.link(top)
    finish(top, top.name, stone, collection)
    top.location = (-3.7, -3.2, 0)
    ensure_surface_uv(top)
    for face in top.data.polygons:
        face.use_smooth = len(face.vertices) == 4
        for i in face.loop_indices:
            point = top.data.vertices[top.data.loops[i].vertex_index].co
            # Project the rim onto a vertical plane so its normal map has area.
            axis = max(range(3), key=lambda n: abs(face.normal[n]))
            axes = ((1, 2), (0, 2), (0, 1))[axis]
            top.data.uv_layers[0].data[i].uv = tuple(point[n] / 1.2 for n in axes)
    vertices, faces = [], []
    segments = 192
    for z in (0, .72):
        for i in range(segments):
            angle = math.tau * i / segments
            radius = .245 + .0045 * math.cos(48 * angle)
            vertices.append((radius * math.cos(angle), radius * math.sin(angle), z))
    for i in range(segments):
        j = (i + 1) % segments
        faces.append((i, j, j + segments, i + segments))
    faces += [tuple(reversed(range(segments))), tuple(range(segments, 2 * segments))]
    mesh = bpy.data.meshes.new('Fluted dining pedestal')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    base = bpy.data.objects.new('Dining pedestal', mesh)
    collection.objects.link(base)
    finish(base, base.name, bpy.data.materials[WOOD], collection)
    wood_uv(base, staves=12)
    base.location = (-3.7, -3.2, 0)
    for face in mesh.polygons:
        face.use_smooth = len(face.vertices) == 4
    for obj in (base, top):
        obj['source'] = 'Original dining table; existing CC0 wood and honed travertine maps'


def replace_dining_furniture():
    collection = next(c for c in bpy.data.collections if c.name.startswith('02 '))
    for obj in list(collection.objects):
        if obj.name.startswith(('Dining chair', 'Dining detail ')) or obj.name in ('Dining top', 'Dining pedestal'):
            bpy.data.objects.remove(obj, do_unlink=True)
    for index, outward in enumerate(((-1, 0), (1, 0), (0, -1), (0, 1))):
        dining_chair(collection, index, outward)
    dining_table(collection)
    print('DINING_FURNITURE_AUTHORED', flush=True)


if __name__ == '__main__':
    update_saved_environments(ROOT, replace_dining_furniture)
    os._exit(0)
