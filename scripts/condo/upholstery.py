"""Apply a fine beige woven finish to the existing sofa and reading chair."""
import json
import math
import os
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from furnishing import bake_normal_strength
from scene_export import update_saved_environments

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
SOURCE = ROOT / 'models/terlenka'
NAME = 'Beige woven upholstery - Poly Haven Terlenka'


def fabric_material():
    material = bpy.data.materials.get(NAME) or bpy.data.materials.new(NAME)
    material.use_nodes = True
    nodes, links = material.node_tree.nodes, material.node_tree.links
    nodes.clear()
    shader = nodes.new('ShaderNodeBsdfPrincipled')
    output = nodes.new('ShaderNodeOutputMaterial')
    links.new(shader.outputs['BSDF'], output.inputs['Surface'])
    shader.inputs['Metallic'].default_value = 0
    for socket, channel in [('Base Color', 'diff'), ('Roughness', 'rough'), ('Normal', 'nor_gl')]:
        image = bpy.data.images.load(str(SOURCE / f'terlenka_{channel}_2k.jpg'), check_existing=True)
        image.colorspace_settings.name = 'sRGB' if socket == 'Base Color' else 'Non-Color'
        texture = nodes.new('ShaderNodeTexImage')
        if socket == 'Normal':
            image = bake_normal_strength(image, .2, ROOT / 'upholstery-normal-soft.png')
            image.name = 'Woven upholstery gentle normal'
        texture.image = image
        colour = texture.outputs['Color']
        if socket == 'Normal':
            normal = nodes.new('ShaderNodeNormalMap')
            links.new(colour, normal.inputs['Color'])
            colour = normal.outputs['Normal']
        links.new(colour, shader.inputs[socket])
    return material


def unwrap_cushions(obj, slot):
    """Unwrap only cushion UV0 at source scale; preserve wood and baked UV1."""
    mesh = obj.data
    preserved = [[tuple(item.uv) for item in layer.data] for layer in mesh.uv_layers][1:]
    wood_uv = {i: tuple(mesh.uv_layers[0].data[i].uv)
               for face in mesh.polygons if face.material_index != slot for i in face.loop_indices}
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    mesh.uv_layers.active_index = 0
    mesh.uv_layers[0].active_render = True
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='DESELECT')
    bpy.ops.object.mode_set(mode='OBJECT')
    for face in mesh.polygons:
        face.select = face.material_index == slot
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.uv.smart_project(angle_limit=math.radians(66), island_margin=.02)
    bpy.ops.object.mode_set(mode='OBJECT')
    mesh.calc_loop_triangles()
    uv = mesh.uv_layers[0].data
    surface_area, uv_area = 0., 0.
    for triangle in mesh.loop_triangles:
        if mesh.polygons[triangle.polygon_index].material_index != slot:
            continue
        a, b, c = [obj.matrix_world @ mesh.vertices[i].co for i in triangle.vertices]
        surface_area += (b - a).cross(c - a).length * .5
        a, b, c = [uv[i].uv.copy() for i in triangle.loops]
        u, v = b - a, c - a
        uv_area += abs(u.x * v.y - u.y * v.x) * .5
    dimensions = json.loads((SOURCE / 'provenance.json').read_text())['dimensions_mm']
    repeat_metres = math.sqrt(dimensions[0] * dimensions[1]) / 1000
    scale = math.sqrt(surface_area / uv_area) / repeat_metres
    for face in mesh.polygons:
        if face.material_index == slot:
            for i in face.loop_indices:
                uv[i].uv *= scale
    assert all(tuple(uv[i].uv) == value for i, value in wood_uv.items()), 'Wood UV changed'
    assert preserved == [[tuple(item.uv) for item in layer.data] for layer in mesh.uv_layers][1:], 'Baked UV changed'
    print('UPHOLSTERY_UV', obj.name, 'repeat_m', repeat_metres, 'surface_m2', surface_area, flush=True)


def assign_upholstery():
    material = fabric_material()
    for name in ('Living sofa 0', 'Reading armchair 0'):
        obj = bpy.data.objects[name]
        slot = next(i for i, item in enumerate(obj.data.materials)
                    if item and (item.name.startswith('modern_arm_chair_01_pillow') or item.name == NAME))
        unwrap_cushions(obj, slot)
        obj.data.materials[slot] = material
        obj['upholstery_source'] = 'https://polyhaven.com/a/terlenka'
        obj['upholstery_license'] = 'CC0-1.0'


def update_saved_packages():
    update_saved_environments(ROOT, assign_upholstery)


if __name__ == '__main__':
    update_saved_packages()
    os._exit(0)
