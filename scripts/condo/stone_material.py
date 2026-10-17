"""Author a honed travertine coffee-table finish in Blender and export both packages."""
import os
from pathlib import Path
import sys

import bpy
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from furnishing import bake_normal_strength
from scene_export import update_saved_environments

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
SOURCE = ROOT / 'models/Travertine009'


def load_map(channel):
    image = bpy.data.images.load(str(SOURCE / f'Travertine009_2K-PNG_{channel}.png'), check_existing=True)
    image.colorspace_settings.name = 'sRGB' if channel == 'Color' else 'Non-Color'
    return image


def honed_roughness():
    source = load_map('Roughness')
    width, height = source.size
    pixels = np.empty(width * height * 4, dtype=np.float32)
    source.pixels.foreach_get(pixels)
    pixels = pixels.reshape((-1, 4))
    pixels[:, :3] = .38 + .65 * pixels[:, :3]
    image = bpy.data.images.new('Travertine honed roughness', width=width, height=height)
    image.colorspace_settings.name = 'Non-Color'
    image.pixels.foreach_set(pixels.ravel())
    image.filepath_raw = str(ROOT / 'travertine-honed-roughness.png')
    image.file_format = 'PNG'
    image.save()
    return image


def assign_stone():
    obj = bpy.data.objects['Coffee table']
    name = 'Honed travertine - ambientCG Travertine009'
    material = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    material.use_nodes = True
    material.node_tree.nodes.clear()
    for image in list(bpy.data.images):
        if image.users == 0 and Path(image.filepath_raw).name in ('travertine-normal-soft.png', 'travertine-honed-roughness.png'):
            bpy.data.images.remove(image)
    shader = material.node_tree.nodes.new('ShaderNodeBsdfPrincipled')
    output = material.node_tree.nodes.new('ShaderNodeOutputMaterial')
    material.node_tree.links.new(shader.outputs['BSDF'], output.inputs['Surface'])
    shader.inputs['Metallic'].default_value = 0
    shader.inputs['Base Color'].default_value = (1, 1, 1, 1)
    normal = bake_normal_strength(load_map('NormalGL'), .10, ROOT / 'travertine-normal-soft.png')
    normal.name = 'Travertine gentle normal'
    for socket, image in [('Base Color', load_map('Color')), ('Roughness', honed_roughness()), ('Normal', normal)]:
        texture = material.node_tree.nodes.new('ShaderNodeTexImage')
        texture.image = image
        output = texture.outputs['Color']
        if socket == 'Normal':
            normal_map = material.node_tree.nodes.new('ShaderNodeNormalMap')
            material.node_tree.links.new(output, normal_map.inputs['Color'])
            output = normal_map.outputs['Normal']
        material.node_tree.links.new(output, shader.inputs[socket])
    obj.data.materials.clear()
    obj.data.materials.append(material)
    uv = obj.data.uv_layers[0]
    # The material's authored repeat is 1.2 m. Only UV0 changes: UV1 is the
    # existing sky-irradiance atlas and must remain byte-for-byte unchanged.
    for polygon in obj.data.polygons:
        axis = max(range(3), key=lambda i: abs(polygon.normal[i]))
        axes = [(1, 2), (0, 2), (0, 1)][axis]
        for index in polygon.loop_indices:
            point = obj.data.vertices[obj.data.loops[index].vertex_index].co
            uv.data[index].uv = (point[axes[0]] / 1.2, point[axes[1]] / 1.2)
    uv.active_render = True
    obj['surface_source'] = 'https://ambientcg.com/a/Travertine009'
    obj['surface_license'] = 'CC0-1.0'


def update_saved_packages():
    update_saved_environments(ROOT, assign_stone)


if __name__ == '__main__':
    update_saved_packages()
    os._exit(0)
