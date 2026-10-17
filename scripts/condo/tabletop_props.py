"""Original decorative mug and cloth-bound book for the warm-modern condo."""
import math
import os
from pathlib import Path
import sys

import bmesh
import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from scene_export import update_saved_environments

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'


def material(name, colour, roughness):
    result = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    result.use_nodes = True
    shader = result.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Base Color'].default_value = (*colour, 1)
    shader.inputs['Roughness'].default_value = roughness
    shader.inputs['Metallic'].default_value = 0
    return result


def finish(obj, name, surface, collection):
    obj.name = name
    obj.data.name = name
    obj.data.materials.clear()
    obj.data.materials.append(surface)
    for old in list(obj.users_collection):
        old.objects.unlink(obj)
    collection.objects.link(obj)
    obj['source'] = 'Original procedural condo tabletop detail'
    return obj


def box(name, centre, size, surface, collection):
    bpy.ops.mesh.primitive_cube_add(size=1, location=centre)
    obj = finish(bpy.context.object, name, surface, collection)
    obj.dimensions = size
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    bevel = obj.modifiers.new('Soft manufactured edge', 'BEVEL')
    bevel.width = min(.0015, min(size) / 3)
    bevel.segments = 3
    obj.modifiers.new('Face normals', 'WEIGHTED_NORMAL')
    return obj


def lathe(profile, segments=64):
    vertices, rings, faces = [], [], []
    for radius, height in profile:
        ring = []
        for j in range(segments if radius else 1):
            angle = j * math.tau / segments
            ring.append(len(vertices))
            vertices.append((radius * math.cos(angle), radius * math.sin(angle), height))
        rings.append(ring)
    for lower, upper in zip(rings, rings[1:]):
        for j in range(segments):
            k = (j + 1) % segments
            if len(lower) == 1:
                faces.append((lower[0], upper[k], upper[j]))
            elif len(upper) == 1:
                faces.append((lower[j], lower[k], upper[0]))
            else:
                faces.append((lower[j], lower[k], upper[k], upper[j]))
    mesh = bpy.data.meshes.new('Lathed ceramic')
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    assert all(edge.is_manifold for edge in bm.edges), 'Lathed vessel must be watertight'
    bm.to_mesh(mesh)
    bm.free()
    for polygon in mesh.polygons:
        polygon.use_smooth = True
    return mesh


def mug(collection, base):
    ceramic = material('Ivory glazed ceramic', (.72, .67, .58), .25)
    profile = [(0, 0), (.037, 0), (.043, .004), (.046, .10), (.046, .115),
        (.045, .120), (.042, .122), (.039, .119), (.039, .020), (.034, .014), (0, .014)]
    body = bpy.data.objects.new('Ceramic cup', lathe(profile))
    collection.objects.link(body)
    finish(body, 'Ceramic cup', ceramic, collection)
    body.location = base
    diagonal = Vector((math.sqrt(.5), math.sqrt(.5), 0))
    bpy.ops.mesh.primitive_torus_add(major_segments=48, minor_segments=12,
        major_radius=.029, minor_radius=.006,
        location=Vector(base) + diagonal * .059 + Vector((0, 0, .066)),
        rotation=(math.pi / 2, 0, math.pi / 4))
    handle = finish(bpy.context.object, 'Condo tabletop mug handle', ceramic, collection)
    for polygon in handle.data.polygons:
        polygon.use_smooth = True
    liquid = bpy.data.objects.new('Condo tabletop coffee', lathe([
        (0, .101), (.0385, .101), (.0385, .104), (.036, .1032), (0, .1032)]))
    collection.objects.link(liquid)
    finish(liquid, 'Condo tabletop coffee', material('Coffee surface', (.035, .012, .004), .20), collection)
    liquid.location = base


def book(collection, base):
    x, y, z = base
    cloth = material('Book sage cloth', (.15, .23, .19), .82)
    paper = material('Book warm paper', (.77, .73, .63), .88)
    ink = material('Book cover warm lettering', (.62, .53, .35), .6)
    for name, height in [('Condo tabletop book back cover', .0015), ('Book on coffee table', .0335)]:
        box(name, (x, y, z + height), (.30, .22, .003), cloth, collection)
    box('Condo tabletop book page block', (x + .002, y, z + .0175), (.287, .208, .026), paper, collection)
    box('Condo tabletop book spine', (x - .147, y, z + .0175), (.006, .22, .032), cloth, collection)
    curve = bpy.data.curves.new('Book cover lettering', 'FONT')
    curve.body = 'FIELD\nNOTES'
    curve.align_x = 'CENTER'
    curve.size = .022
    curve.space_line = 1.25
    text = bpy.data.objects.new('Condo tabletop book lettering', curve)
    collection.objects.link(text)
    text.location = (x, y + .022, z + .03515)
    curve.materials.append(ink)
    bpy.ops.object.select_all(action='DESELECT')
    text.select_set(True)
    bpy.context.view_layer.objects.active = text
    bpy.ops.object.convert(target='MESH')
    text['source'] = 'Original decorative book cover; no real publication represented'


def replace_tabletop_props():
    collection = next(c for c in bpy.data.collections if c.name.startswith('02 '))
    table = bpy.data.objects['Coffee table']
    surface_z = max((table.matrix_world @ Vector(corner)).z for corner in table.bound_box)
    for obj in list(collection.objects):
        if obj.name in ('Ceramic cup', 'Book on coffee table') or obj.name.startswith('Condo tabletop '):
            bpy.data.objects.remove(obj, do_unlink=True)
    mug(collection, (3.25, -.4, surface_z))
    book(collection, (2.8, -.4, surface_z + .0004))
    print('TABLETOP_PROPS_AUTHORED', 'surface_z', surface_z, flush=True)


def update_saved_packages():
    update_saved_environments(ROOT, replace_tabletop_props)


if __name__ == '__main__':
    update_saved_packages()
    os._exit(0)
