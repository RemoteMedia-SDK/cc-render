"""Original static bedroom furnishings, authored in Blender metres."""
import math
import os
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from scene_export import ensure_surface_uv, grouped_solids, update_saved_environments, update_solid_manifests
from tabletop_props import box, finish, lathe, material
from upholstery import NAME as FABRIC_NAME, unwrap_cushions

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
PREFIX = 'Bedroom detail '
DUVET_GRID = (56, 64)


def collection():
    return next(c for c in bpy.data.collections if c.name.startswith('02 '))


def cloth_material():
    name = 'Bedroom sage linen'
    result = bpy.data.materials.get(name)
    if result is None:
        result = bpy.data.materials[FABRIC_NAME].copy()
        result.name = name
    shader = result.node_tree.nodes.get('Principled BSDF')
    for link in list(shader.inputs['Base Color'].links):
        result.node_tree.links.remove(link)
    shader.inputs['Base Color'].default_value = (.20, .27, .22, 1)
    return result


def piece(name, centre, size, surface, group='bed_base'):
    obj = box(PREFIX + name, centre, size, surface, collection())
    obj['bedroom_group'] = group
    ensure_surface_uv(obj)
    return obj


def wood_piece(name, centre, size, group='bed_base'):
    obj = piece(name, centre, size, bpy.data.materials['Kitchen light oak veneer'], group)
    mesh = obj.data
    along = max(range(3), key=lambda i: size[i])
    for face in mesh.polygons:
        axis = max(range(3), key=lambda i: abs(face.normal[i]))
        transverse = next(i for i in range(3) if i != axis and i != along) if axis != along else (along + 1) % 3
        for index in face.loop_indices:
            point = mesh.vertices[mesh.loops[index].vertex_index].co
            mesh.uv_layers[0].data[index].uv = (.015 + .08 * (point[transverse] / size[transverse] + .5), .1 + (point[along] + size[along] / 2) / max(1.7, size[along] * 1.2))
    obj.modifiers['Soft manufactured edge'].width = .008
    return obj


def soft_piece(name, centre, size, group='bed_base'):
    obj = piece(name, centre, size, bpy.data.materials[FABRIC_NAME], group)
    bevel = obj.modifiers['Soft manufactured edge']
    bevel.width, bevel.segments = min(size) * .38, 8
    bpy.context.view_layer.objects.active = obj
    for modifier in list(obj.modifiers):
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    unwrap_cushions(obj, 0)
    return obj


def pillow(centre, angle):
    columns, rows = 32, 20
    vertices, faces = [], []
    for side in (1, -1):
        for row in range(rows + 1):
            v = 2 * row / rows - 1
            for column in range(columns + 1):
                u = 2 * column / columns - 1
                loft = .015 + .085 * max((1 - u * u) * (1 - v * v), 0) ** .45
                vertices.append((.385 * u * (1 - .06 * v ** 8), .23 * v * (1 - .06 * u ** 8), side * loft))
    offset = (columns + 1) * (rows + 1)
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
    mesh = bpy.data.meshes.new('Puffed linen pillow')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(PREFIX + 'linen pillow', mesh)
    collection().objects.link(obj)
    finish(obj, obj.name, bpy.data.materials[FABRIC_NAME], collection())
    obj.location = centre
    obj.rotation_euler = (.10, 0, angle)
    obj['bedroom_group'] = 'bed_base'
    for face in mesh.polygons:
        face.use_smooth = True
    ensure_surface_uv(obj)
    unwrap_cushions(obj, 0)


def bed_frame():
    for x in (3.28, 5.12):
        wood_piece('side rail', (x, 3.4, .255), (.10, 2.18, .25))
    for y in (2.36, 4.44):
        wood_piece('end rail', (4.2, y, .255), (1.84, .10, .25))
    for x in (3.37, 5.03):
        for y in (2.43, 4.37):
            wood_piece('recessed foot', (x, y, .095), (.10, .10, .19))
    soft_piece('mattress', (4.2, 3.4, .48), (1.80, 2.12, .28))
    wood_piece('headboard frame', (4.2, 4.55, .80), (2.00, .10, 1.35), 'bed_headboard')
    for x in (3.73, 4.67):
        soft_piece('headboard upholstered panel', (x, 4.47, 1.0), (.90, .10, .82), 'bed_headboard')
    for x, angle in ((3.75, -.07), (4.65, .06)):
        pillow((x, 4.08, .73), angle)


def duvet():
    columns, rows = DUVET_GRID
    vertices, faces = [], []
    for row in range(rows + 1):
        y = 2.18 + 1.78 * row / rows
        foot_drop = .16 * (max(2.37 - y, 0) / .19) ** 1.45
        for column in range(columns + 1):
            x = -.97 + 1.94 * column / columns
            side_drop = .17 * (max(abs(x) - .80, 0) / .17) ** 1.55
            # Broad, low folds; this is a static textile mesh, not avatar cloth dynamics.
            folds = .009 * math.sin(15 * x + 7 * y) + .005 * math.sin(30 * x - 5 * y)
            folds *= .3 + .7 * (abs(x) / .97) ** 2
            loft = .023 * math.cos(x / .97 * math.pi / 2)
            turnback = .045 * math.exp(-((y - 3.77) / .11) ** 2)
            z = .659 + loft + folds + turnback - side_drop - foot_drop
            vertices.append((4.2 + x, y, z))
    for row in range(rows):
        for column in range(columns):
            a = row * (columns + 1) + column
            faces.append((a, a + 1, a + columns + 2, a + columns + 1))
    mesh = bpy.data.meshes.new('Softly folded sage duvet')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(PREFIX + 'folded duvet', mesh)
    collection().objects.link(obj)
    finish(obj, obj.name, cloth_material(), collection())
    obj['bedroom_group'] = 'bed_base'
    for face in mesh.polygons:
        face.use_smooth = True
    thickness = obj.modifiers.new('Soft duvet thickness', 'SOLIDIFY')
    thickness.thickness = .028
    thickness.offset = 0
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.modifier_apply(modifier=thickness.name)
    ensure_surface_uv(obj)
    unwrap_cushions(obj, 0)


def duvet_rim_arc(vertex):
    columns, rows = DUVET_GRID
    row, column = divmod(vertex, columns + 1)
    if row == 0:
        return column
    if column == columns:
        return columns + row
    if row == rows:
        return 2 * columns + rows - column
    assert column == 0, 'Rim vertex must lie on the grid boundary'
    return 2 * (columns + rows) - row


def duvet_face_uv(vertices, count):
    columns, rows = DUVET_GRID
    top = all(v < count for v in vertices)
    bottom = all(v >= count for v in vertices)
    if top or bottom:
        left = .04 if top else .52
        return [(left + .44 * (v % count % (columns + 1)) / columns,
                 .10 + .86 * (v % count // (columns + 1)) / rows) for v in vertices]
    perimeter = 2 * (columns + rows)
    arcs = [duvet_rim_arc(v % count) for v in vertices]
    if max(arcs) - min(arcs) > perimeter / 2:
        arcs = [a + perimeter if a < perimeter / 2 else a for a in arcs]
    return [(.04 + .92 * arc / perimeter, .025 if v < count else .045)
            for v, arc in zip(vertices, arcs)]


def duvet_bake_uv(obj):
    """Continuous top/bottom atlases and a separate padded perimeter strip."""
    columns, rows = DUVET_GRID
    count = (columns + 1) * (rows + 1)
    mesh = obj.data
    assert len(mesh.vertices) == count * 2, 'Expected the authored solidified duvet grid'
    uv = mesh.uv_layers.get('BakedSky') or mesh.uv_layers.new(name='BakedSky')
    for face in mesh.polygons:
        for index, value in zip(face.loop_indices, duvet_face_uv(face.vertices, count)):
            uv.data[index].uv = value
    print('DUVET_CONTINUOUS_BAKE_UV', len(mesh.polygons), flush=True)


def bedside_unit():
    wood_piece('bedside case', (5.52, 4.18, .335), (.54, .60, .53), 'bedside_unit')
    shadow = material('Bedroom shadow reveal', (.03, .035, .03), .7)
    piece('bedside plinth', (5.52, 4.18, .045), (.43, .49, .09), shadow, 'bedside_unit')
    for z in (.23, .48):
        wood_piece('bedside drawer', (5.52, 3.868, z), (.518, .025, .225), 'bedside_unit')
        piece('bedside finger pull', (5.52, 3.847, z + .075), (.18, .018, .012), shadow, 'bedside_unit')
    top = piece('bedside stone top', (5.52, 4.18, .62), (.57, .63, .04), bpy.data.materials['Honed travertine - ambientCG Travertine009'], 'bedside_unit')
    for item in top.data.uv_layers[0].data:
        item.uv *= .5
    ceramic = material('Bedroom lamp ceramic', (.58, .49, .36), .35)
    body = bpy.data.objects.new(PREFIX + 'ceramic lamp base', lathe([(0, 0), (.09, 0), (.10, .035), (.075, .14), (.035, .18), (0, .18)]))
    collection().objects.link(body)
    finish(body, body.name, ceramic, collection())
    body.location = (5.52, 4.18, .64)
    body['bedroom_group'] = 'bedside_lamp'
    bpy.ops.mesh.primitive_cone_add(vertices=64, radius1=.18, radius2=.145,
        depth=.28, end_fill_type='NOTHING', location=(5.52, 4.18, .95))
    shade = finish(bpy.context.object, PREFIX + 'linen lamp shade', bpy.data.materials[FABRIC_NAME], collection())
    thickness = shade.modifiers.new('Linen shade thickness', 'SOLIDIFY')
    thickness.thickness = .014
    bpy.ops.object.modifier_apply(modifier=thickness.name)
    shade['bedroom_group'] = 'bedside_lamp'
    ensure_surface_uv(shade)
    for face in shade.data.polygons:
        face.use_smooth = True
    unwrap_cushions(shade, 0)


def bedroom_solids():
    return grouped_solids('bedroom_group', {'bed_base', 'bed_headboard', 'bedside_unit'})


def bedroom_lights():
    from mathutils import Vector
    shade = bpy.data.objects[PREFIX + 'linen lamp shade']
    centre = sum((shade.matrix_world @ Vector(c) for c in shade.bound_box), Vector()) / 8
    return [{'id':'bedside_lamp', 'position':[centre.x, centre.z, -centre.y],
             'color_srgb':[1.0, .78, .52], 'lumens':600.0, 'range_m':3.5,
             'radius_m':.08, 'shadows':True,
             'diffuser':{'mesh':shade.data.name, 'emissive_strength':50.0}}]


def replace_bedroom():
    names = ('Bed base', 'Mattress', 'Duvet', 'Pillow', 'Pillow.001', 'Bedside unit')
    for obj in list(collection().objects):
        if obj.name.startswith(PREFIX) or obj.name in names:
            bpy.data.objects.remove(obj, do_unlink=True)
    bed_frame()
    duvet()
    bedside_unit()
    for obj in collection().objects:
        if obj.name.startswith(PREFIX):
            obj['source'] = 'Original static bedroom furniture; existing CC0 oak, fabric and travertine maps'
    print('BEDROOM_AUTHORED', sum(o.name.startswith(PREFIX) for o in collection().objects), flush=True)


def update_collision_manifests():
    solids = bedroom_solids()
    update_solid_manifests(ROOT, solids)
    print('BEDROOM_COLLIDERS_EXPORTED', len(solids), flush=True)


if __name__ == '__main__':
    update_saved_environments(ROOT, replace_bedroom)
    update_collision_manifests()
    from export_practical_lights import export
    export()
    os._exit(0)
