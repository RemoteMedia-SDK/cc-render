"""Import selected CC0 furnishings into the authored Blender scene."""
import math
from pathlib import Path
import bpy
from mathutils import Matrix, Vector


def bake_light_oak(source, destination):
    """A lighter, lower-contrast oak finish, retaining the source board grain."""
    import numpy as np
    width, height = source.size
    pixels = np.empty(width*height*4, dtype=np.float32)
    source.pixels.foreach_get(pixels)
    pixels = pixels.reshape((-1,4))
    pixels[:,:3] = np.clip((pixels[:,:3]*.65 + np.array([.8,.75,.63])*.35)*1.15,0,1)
    result = bpy.data.images.new('Light oak finish',width=width,height=height)
    result.colorspace_settings.name = source.colorspace_settings.name
    result.pixels.foreach_set(pixels.ravel())
    result.filepath_raw = str(destination)
    result.file_format = 'PNG'
    result.save()
    return result


def bake_normal_strength(source, strength, destination):
    """Bevy 0.15 ignores glTF normalTexture.scale; bake amplitude into pixels."""
    import numpy as np
    width, height = source.size
    pixels = np.empty(width * height * 4, dtype=np.float32)
    source.pixels.foreach_get(pixels)
    pixels = pixels.reshape((-1, 4))
    normals = pixels[:, :3] * 2 - 1
    normals[:, :2] *= strength
    normals /= np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-8)
    pixels[:, :3] = normals * .5 + .5
    result = bpy.data.images.new('Oak floor gentle normal', width=width, height=height)
    result.colorspace_settings.name = 'Non-Color'
    result.pixels.foreach_set(pixels.ravel())
    result.filepath_raw = str(destination)
    result.file_format = 'PNG'
    result.save()
    return result


class Furnishings:
    def __init__(self, root, collection, solids):
        self.root = Path(root)
        self.collection = collection
        self.solids = solids
        self.templates = {}

    def import_model(self, asset):
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=str(self.root / asset / f"{asset}_2k.gltf"))
        imported = set(bpy.data.objects) - before
        meshes = [o for o in imported if o.type == 'MESH']
        originals = [(o, o.matrix_world.copy()) for o in meshes]
        for obj, world in originals:
            obj.parent = None
            obj.matrix_world = world
            for collection in list(obj.users_collection):
                collection.objects.unlink(obj)
            self.collection.objects.link(obj)
        for obj in imported - set(meshes):
            bpy.data.objects.remove(obj, do_unlink=True)
        corners = [world @ Vector(c) for obj, world in originals for c in obj.bound_box]
        low = Vector(tuple(min(c[i] for c in corners) for i in range(3)))
        high = Vector(tuple(max(c[i] for c in corners) for i in range(3)))
        # Keep immutable mesh data, not placed objects whose transforms/data
        # may later be adapted into a sofa or another instance.
        self.templates[asset] = ([(obj.data, world) for obj, world in originals], low, high)
        return originals, low, high

    def place(self, asset, name, position, extent, axis=0, yaw=0, collider=False, stretch_x=1):
        if asset in self.templates:
            source, low, high = self.templates[asset]
            objects = []
            for mesh, world in source:
                copy = bpy.data.objects.new(name, mesh)
                self.collection.objects.link(copy)
                objects.append((copy, world))
        else:
            objects, low, high = self.import_model(asset)
        center = Vector(((low.x+high.x)/2, (low.y+high.y)/2, low.z))
        scale = extent / (high[axis]-low[axis])
        placement = Matrix.Translation(Vector(position)) @ Matrix.Rotation(math.radians(yaw),4,'Z') @ Matrix.Scale(scale,4) @ Matrix.Translation(-center)
        for i,(obj,world) in enumerate(objects):
            if stretch_x != 1:
                widen_center(obj, world, center.x, (high.x-low.x)/2, stretch_x)
                obj.matrix_world = placement
            else:
                obj.matrix_world = placement @ world
            obj.name = f"{name} {i}"
            obj['source_asset'] = asset
            obj['license'] = 'CC0-1.0'
        bpy.context.view_layer.update()
        if collider:
            corners = [obj.matrix_world @ Vector(c) for obj,_ in objects for c in obj.bound_box]
            low = Vector(tuple(min(c[i] for c in corners) for i in range(3)))
            high = Vector(tuple(max(c[i] for c in corners) for i in range(3)))
            center = (low+high)/2
            half = (high-low)/2
            facing = Vector((math.sin(math.radians(yaw)), math.cos(math.radians(yaw))))
            clearance = abs(facing.x)*half.x + abs(facing.y)*half.y + .65
            self.solids.append({'id': name.lower().replace(' ','_'),
                'position': [center.x,center.z,-center.y], 'half_extents':[half.x,half.z,half.y],
                'navigation_obstacle':True,'approach':[facing.x*clearance,0,facing.y*clearance]})


def widen_center(obj, world, center_x, half_width, factor):
    """Extend the centre span while translating the arms/legs at original thickness."""
    obj.data = obj.data.copy()
    obj.data.transform(world)
    extension = half_width * (factor-1)
    for vertex in obj.data.vertices:
        point = vertex.co.copy()
        blend = max(-1., min(1., (point.x-center_x)/(half_width*.4)))
        point.x += extension*blend
        vertex.co = point
    obj.data.update()


def replace_proxies(root, furniture, plants, solids):
    prefixes = ('Sofa ', 'Loose linen cushion', 'Reading chair ')
    for obj in list(furniture.objects):
        if obj.name.startswith(prefixes):
            bpy.data.objects.remove(obj, do_unlink=True)
    solids[:] = [s for s in solids if s['id'] != 'sofa_base']
    for obj in list(plants.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    models = Furnishings(root, furniture, solids)
    # An oak-framed bench sofa based on the clean-lined lounge-chair geometry.
    models.place('modern_arm_chair_01','Living sofa',(2.8,-2.5,0),.9,yaw=180,stretch_x=3.4,collider=True)
    models.place('modern_arm_chair_01','Reading armchair',(5,-3.7,0),.82, yaw=-120,collider=True)
    for i,(x,y,height) in enumerate([(5.3,-.2,1.5),(5.3,-4.4,1.25),(-5.3,-6.4,1.05),(5.3,-6.4,1.05)]):
        models.place('potted_plant_02',f'Window plant {i}',(x,y,0),height,axis=2,yaw=i*79,collider=True)
    for material in bpy.data.materials:
        if material.name.startswith('modern_arm_chair_01_pillow'):
            shader = material.node_tree.nodes.get('Principled BSDF')
            for socket in ['Base Color','Roughness']:
                for link in list(shader.inputs[socket].links):material.node_tree.links.remove(link)
            shader.inputs['Base Color'].default_value=(.70,.65,.54,1)
            shader.inputs['Roughness'].default_value=.78
