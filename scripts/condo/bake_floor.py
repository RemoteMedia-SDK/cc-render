"""Bake sky irradiance; --floor / --coffee / --balcony update one surface.

Run with Blender after build.py. No avatar is loaded or modified.
The white world is the only light source: diffuse direct + bounced sky,
without the receiving floor's albedo. Runtime sun/practicals remain dynamic.
"""
from pathlib import Path
import json
import os
import sys
import bpy
import numpy as np
sys.path.insert(0, str(Path(__file__).parent))
from scene_export import export_environment

root = Path(__file__).resolve().parents[2]/'scenes/toronto_condo'
refresh_coffee = '--coffee' in sys.argv
refresh_balcony = '--balcony' in sys.argv
refresh_floor = '--floor' in sys.argv
refresh_room = '--refresh-room' in sys.argv
refresh_bedroom = '--bedroom' in sys.argv
refresh_duvet = '--duvet' in sys.argv
assert sum(flag in sys.argv for flag in ('--coffee', '--balcony', '--floor', '--room', '--refresh-room', '--bedroom', '--duvet')) <= 1, 'Choose one bake scope'
partial_slug = 'coffee' if refresh_coffee else 'balcony' if refresh_balcony else 'floor' if refresh_floor else 'room' if refresh_room else 'bedroom' if refresh_bedroom else 'duvet' if refresh_duvet else None
bpy.ops.wm.open_mainfile(filepath=str(root/('toronto-condo-lightmap.blend' if partial_slug else 'toronto-condo.blend')))
scene = bpy.context.scene
scene.render.engine = 'CYCLES'
scene.cycles.device = 'CPU'
scene.cycles.samples = 128 if "--room" in sys.argv or partial_slug else 64
scene.cycles.seed = 731
scene.cycles.max_bounces = 8
scene.cycles.diffuse_bounces = 6
scene.render.bake.use_pass_direct = True
scene.render.bake.use_pass_indirect = True
scene.render.bake.use_pass_color = False
scene.render.bake.margin = 8
scene.render.bake.use_selected_to_active = False
world = bpy.data.worlds.new('Normalized white sky for irradiance bake')
world.use_nodes = True
world.node_tree.nodes['Background'].inputs['Color'].default_value = (1,1,1,1)
world.node_tree.nodes['Background'].inputs['Strength'].default_value = 1
scene.world = world
for obj in scene.objects:
    if obj.type == 'LIGHT': obj.hide_render = True

def bake_surface(object_name, slug):
    obj = bpy.data.objects[object_name]
    obj.data.name = 'Condo baked '+slug
    bpy.ops.object.select_all(action='DESELECT')
    obj.hide_set(False)
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    for modifier in list(obj.modifiers):
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    if refresh_coffee or len(obj.data.uv_layers) == 2:
        assert len(obj.data.uv_layers) == 2 and obj.data.uv_layers[1].name == 'BakedSky'
    else:
        assert len(obj.data.uv_layers) == 1, (object_name, 'expected one source UV layer')
        obj.data.uv_layers.new(name='BakedSky')
        obj.data.uv_layers.active_index = 1
        if slug == 'bedroom-duvet':
            from bedroom_furniture import duvet_bake_uv
            duvet_bake_uv(obj)
        else:
            bpy.ops.object.mode_set(mode='EDIT')
            bpy.ops.mesh.select_all(action='SELECT')
            bpy.ops.uv.smart_project(island_margin=.025)
            bpy.ops.object.mode_set(mode='OBJECT')
    obj.data.uv_layers.active_index = 1
    obj.data.uv_layers[0].active_render = True
    image = bpy.data.images.new(slug+' sky irradiance',width=512,height=512,float_buffer=True)
    image.colorspace_settings.name = 'Non-Color'
    for mat in obj.data.materials:
        node = mat.node_tree.nodes.new('ShaderNodeTexImage')
        node.image = image
        mat.node_tree.nodes.active = node
    print('SKY_BAKE_START',object_name,flush=True)
    bpy.ops.object.bake(type='DIFFUSE',uv_layer='BakedSky')
    pixels = np.empty(512*512*4,dtype=np.float32)
    image.pixels.foreach_get(pixels)
    np.save(root/(slug+'-sky-linear.npy'),pixels.reshape(512,512,4))
    print('SKY_BAKE_RANGE',slug,float(pixels.min()),float(pixels.max()),flush=True)
    obj.data.uv_layers.active_index = 0
    for mat in obj.data.materials:
        for node in list(mat.node_tree.nodes):
            if node.type == 'TEX_IMAGE' and node.image == image:
                mat.node_tree.nodes.remove(node)


targets = [('Interior floor 12 x 10 m','floor')]
bedroom_targets = [('Bedroom rug', 'bedroom-rug'),
                   ('Bedroom detail folded duvet', 'bedroom-duvet'),
                   ('Bedroom detail mattress', 'bedroom-mattress')]
if refresh_coffee:
    targets = [('Coffee table', 'coffee')]
elif refresh_balcony:
    targets = [('Balcony slab 12 x 2.4 m', 'balcony')]
elif refresh_bedroom:
    targets = bedroom_targets
elif refresh_duvet:
    targets = [bedroom_targets[1]]
elif '--room' in sys.argv or refresh_room:
    targets += [('Ceiling','ceiling'),('Living rug','rug'),
                ('Living sofa 0','sofa'),('Coffee table','coffee'),
                ('Balcony slab 12 x 2.4 m','balcony')]
    if 'Bedroom detail folded duvet' in bpy.data.objects:
        targets += bedroom_targets
    if 'Bathroom detail recessed ceramic basin' in bpy.data.objects:
        targets += [('Bathroom floor', 'bathroom-floor')]
for object_name,slug in targets:
    bake_surface(object_name,slug)
gltf = export_environment(root/'interior-city-lightmap.glb')
package = json.loads((root/('scene-lightmap.json' if partial_slug else 'scene.json')).read_text())
package['geometry'] = 'interior-city-lightmap.glb'
updated_images = {slug + '-sky-linear.png' for _, slug in targets}
package['lightmaps'] = [m for m in package['lightmaps'] if m['image'] not in updated_images] if partial_slug else []
for _,slug in targets:
    name = 'Condo baked '+slug
    mesh = next(m for m in gltf['meshes'] if m.get('name') == name)
    for index,primitive in enumerate(mesh['primitives']):
        assert 'TEXCOORD_1' in primitive['attributes'], name
        primitive_name = name if len(mesh['primitives']) == 1 else name+'.'+str(index)
        package['lightmaps'].append({'mesh':primitive_name,'image':slug+'-sky-linear.png','sky_strength':2.0})
(root/('sky-bake-'+partial_slug+'-settings.json' if partial_slug else 'sky-bake-settings.json')).write_text(json.dumps({'samples':scene.cycles.samples,'seed':731,
    'resolution':[512,512],'targets':targets},indent=2)+'\n')
(root/'scene-lightmap.json').write_text(json.dumps(package,indent=2)+'\n')
bpy.context.preferences.filepaths.save_version = 0
bpy.ops.wm.save_as_mainfile(filepath=str(root/'toronto-condo-lightmap.blend'))
print('FLOOR_BAKE_EXPORT_COMPLETE',flush=True)
os._exit(0)
