"""Author front/rear lens geometry in Blender; runtime controls night emission."""
from pathlib import Path
import sys
import bpy


def lens_material(name, colour):
    material = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    material.use_nodes = True
    shader = material.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Base Color'].default_value = (*colour, 1)
    shader.inputs['Roughness'].default_value = .18
    shader.inputs['Metallic'].default_value = 0
    shader.inputs['Emission Strength'].default_value = 0
    return material


def prepare_lamps(vehicle):
    front = sorted((o for o in vehicle.objects if o.name.startswith('Headlamp')),
                   key=lambda o: o.location.x)
    assert len(front) == 2, 'Expected two authored front lenses'
    for old in list(vehicle.objects):
        if old.name.startswith('Taillamp'):
            mesh = old.data
            bpy.data.objects.remove(old, do_unlink=True)
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)
    white = lens_material('Vehicle headlamp lens', (.72, .78, .86))
    red = lens_material('Vehicle taillamp lens', (.42, .004, .008))
    for side, head in zip(('left', 'right'), front):
        old_mesh = head.data
        head.data = old_mesh.copy()
        if old_mesh.users == 0:
            bpy.data.meshes.remove(old_mesh)
        head.data.name = 'Vehicle headlamp lens ' + side
        head.data.materials.clear()
        head.data.materials.append(white)
        tail = head.copy()
        tail.data = head.data.copy()
        tail.name = 'Taillamp ' + side
        tail['scene_id'] = 'taillamp_' + side
        tail.data.name = 'Vehicle taillamp lens ' + side
        tail.location.y = -head.location.y
        tail.data.materials.clear()
        tail.data.materials.append(red)
        vehicle.objects.link(tail)


def update_saved_vehicle():
    output = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
    bpy.ops.wm.open_mainfile(filepath=str(output / 'toronto-condo.blend'))
    vehicle = bpy.data.collections['07 Traffic vehicle prefab']
    prepare_lamps(vehicle)
    bpy.ops.object.select_all(action='DESELECT')
    for obj in vehicle.objects:
        obj.hide_set(False)
        obj.select_set(True)
    bpy.ops.export_scene.gltf(filepath=str(output / 'vehicle-base.glb'),
        export_format='GLB', use_selection=True, export_apply=True,
        export_extras=True, export_animations=False, export_cameras=False, export_lights=False)
    sys.path.insert(0, str(Path(__file__).parent))
    from vehicle_variants import add_variants
    assert add_variants(output / 'vehicle-base.glb', output / 'vehicle.glb') == 6
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(output / 'toronto-condo.blend'))
    print('VEHICLE_LAMPS_EXPORTED', len(vehicle.objects), flush=True)


if __name__ == '__main__':
    update_saved_vehicle()
    import os
    os._exit(0)
