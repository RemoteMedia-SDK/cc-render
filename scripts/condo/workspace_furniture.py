"""Original static oak workspace and entry; no avatar poses or interaction scripts."""
import math
import os
from pathlib import Path
import sys

import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).parent))
from dining_furniture import dining_chair
from scene_export import ensure_surface_uv, veneer_uv, grouped_solids, update_saved_environments, update_manifest_entries
from tabletop_props import box, finish, lathe, material
from upholstery import NAME as FABRIC_NAME, unwrap_cushions

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
PREFIX = 'Workspace detail '


def collection():
    return next(c for c in bpy.data.collections if c.name.startswith('02 '))


def piece(name, centre, size, surface, group):
    obj = box(PREFIX + name, centre, size, surface, collection())
    obj['workspace_group'] = group
    ensure_surface_uv(obj)
    if surface.name == 'Kitchen light oak veneer':
        veneer_uv(obj)
    return obj


def rod(name, start, end, radius, surface, group):
    direction = Vector(end) - Vector(start)
    bpy.ops.mesh.primitive_cylinder_add(vertices=32, radius=radius,
        depth=direction.length, location=(Vector(start) + Vector(end)) / 2)
    obj = finish(bpy.context.object, PREFIX + name, surface, collection())
    obj.rotation_euler = direction.to_track_quat('Z', 'Y').to_euler()
    obj['workspace_group'] = group
    ensure_surface_uv(obj)
    for face in obj.data.polygons:
        face.use_smooth = len(face.vertices) == 4
    return obj


def desk():
    oak = bpy.data.materials['Kitchen light oak veneer']
    dark = material('Workspace dark bronze', (.045, .038, .027), .42)
    top = piece('oak desktop', (1.52, 3.2, .76), (.70, 2.05, .055), oak, 'workspace_desk')
    top.modifiers['Soft manufactured edge'].width = .012
    for y in (2.30, 4.10):
        for x in (1.24, 1.80):
            piece('tapered frame leg', (x, y, .36), (.045, .045, .72), dark, 'workspace_desk')
        piece('end frame rail', (1.52, y, .69), (.60, .035, .055), dark, 'workspace_desk')
    piece('drawer case', (1.55, 3.90, .59), (.58, .49, .27), oak, 'workspace_desk')
    piece('drawer front', (1.245, 3.90, .59), (.025, .465, .245), oak, 'workspace_desk')
    piece('drawer finger pull', (1.225, 3.90, .66), (.015, .18, .013), dark, 'workspace_desk')
    piece('cable tray', (1.79, 3.15, .66), (.10, 1.1, .07), dark, 'workspace_desk')


def workstation():
    dark = bpy.data.materials['Workspace dark bronze']
    screen = material('Workspace inactive blue-black screen', (.012, .022, .028), .24)
    piece('monitor foot', (1.65, 3.15, .802), (.20, .31, .028), dark, 'workspace_monitor')
    piece('monitor neck', (1.72, 3.15, .94), (.028, .06, .28), dark, 'workspace_monitor')
    piece('monitor housing', (1.70, 3.15, 1.20), (.045, .73, .43), dark, 'workspace_monitor')
    piece('monitor screen', (1.674, 3.15, 1.20), (.004, .695, .392), screen, 'workspace_monitor')
    felt = material('Workspace charcoal felt', (.075, .08, .075), .95)
    piece('desk mat', (1.36, 3.13, .791), (.32, .82, .004), felt, 'workspace_accessories')
    piece('keyboard base', (1.37, 3.18, .805), (.14, .43, .022), dark, 'workspace_accessories')
    key = material('Workspace warm keycaps', (.35, .34, .30), .7)
    for row in range(4):
        for column in range(13):
            piece('keycap', (1.32 + row * .032, 2.99 + column * .030, .82),
                  (.025, .025, .008), key, 'workspace_accessories')
    mouse = piece('mouse', (1.36, 2.86, .81), (.095, .055, .035), dark, 'workspace_accessories')
    mouse.modifiers['Soft manufactured edge'].width = .015
    paper = material('Workspace ivory paper', (.72, .68, .59), .9)
    piece('notebook cover', (1.43, 2.43, .80), (.22, .28, .018), felt, 'workspace_accessories')
    piece('notebook pages', (1.43, 2.43, .812), (.21, .268, .009), paper, 'workspace_accessories')
    rod('pen', (1.35, 2.40, .826), (1.50, 2.47, .826), .004, dark, 'workspace_accessories')


def task_lamp():
    bronze = bpy.data.materials['Workspace dark bronze']
    group = 'workspace_lamp'
    rod('lamp foot', (1.58, 4.00, .788), (1.58, 4.00, .811), .09, bronze, group)
    rod('lamp upright', (1.58, 4.00, .81), (1.58, 4.00, 1.33), .011, bronze, group)
    rod('lamp arm', (1.58, 4.00, 1.33), (1.43, 3.89, 1.38), .01, bronze, group)
    bpy.ops.mesh.primitive_cone_add(vertices=48, radius1=.105, radius2=.052,
        depth=.11, end_fill_type='NOTHING', location=(1.43, 3.89, 1.35))
    obj = finish(bpy.context.object, PREFIX + 'lamp shade', bronze, collection())
    obj['workspace_group'] = group
    ensure_surface_uv(obj)
    shade = obj.modifiers.new('Shade thickness', 'SOLIDIFY')
    shade.thickness = .002
    diffuser = material('Workspace warm lamp diffuser', (.75, .66, .48), .7)
    rod('lamp diffuser', (1.43, 3.89, 1.302), (1.43, 3.89, 1.305), .098, diffuser, group)


def chair():
    dining_chair(collection(), 99, (0, 0))
    obj = bpy.data.objects['Dining chair 99']
    obj.matrix_world = (Matrix.Translation((.58, 3.20, 0)) @ Matrix.Rotation(math.pi / 2, 4, 'Z')
                        @ Matrix.Translation((3.7, 3.2, 0)) @ obj.matrix_world)
    obj.name = obj.data.name = PREFIX + 'curved upholstered desk chair'
    obj['workspace_group'] = 'workspace_chair'


def shelves():
    oak = bpy.data.materials['Kitchen light oak veneer']
    sage = material('Workspace sage book cloth', (.17, .25, .20), .9)
    paper = bpy.data.materials['Workspace ivory paper']
    for z in (1.91, 2.30):
        piece('floating oak shelf', (1.79, 3.2, z), (.26, 1.35, .035), oak, 'workspace_shelves')
        for index in range(5):
            y = 2.68 + index * .045
            height = .19 + .018 * (index % 3)
            for side in (-1, 1):
                piece('cloth book cover', (1.78, y + side * .018, z + .018 + height / 2),
                      (.16, .004, height), sage, 'workspace_shelves')
            piece('cloth book spine', (1.701, y, z + .018 + height / 2),
                  (.006, .038, height), sage, 'workspace_shelves')
            piece('book paper', (1.783, y, z + .018 + height / 2),
                  (.148, .032, height - .014), paper, 'workspace_shelves')
    clay = material('Workspace ceramic vase', (.48, .32, .20), .48)
    obj = bpy.data.objects.new(PREFIX + 'shelf ceramic vase', lathe([
        (0, 0), (.045, 0), (.055, .025), (.042, .12), (.021, .15),
        (.021, .17), (.016, .17), (.016, .15), (.025, .035), (0, .035)]))
    collection().objects.link(obj)
    finish(obj, obj.name, clay, collection())
    obj.location = (1.78, 3.62, 1.929)
    obj['workspace_group'] = 'workspace_shelves'
    ensure_surface_uv(obj)


def entry():
    oak = bpy.data.materials['Kitchen light oak veneer']
    dark = bpy.data.materials['Workspace dark bronze']
    piece('entry cabinet', (-1.8, 4.65, 1.13), (1.38, .48, 2.10), oak, 'entry_storage')
    piece('entry cabinet plinth', (-1.8, 4.65, .04), (1.24, .36, .08), dark, 'entry_storage')
    for x in (-2.145, -1.455):
        piece('entry cabinet door', (x, 4.395, 1.13), (.676, .026, 2.075), oak, 'entry_storage')
        rod('cabinet handle', (x + .22, 4.367, .92), (x + .22, 4.367, 1.16), .007, dark, 'entry_storage')
    piece('entry bench oak seat', (-2.36, 3.05, .39), (.58, 1.14, .045), oak, 'entry_bench')
    for y in (2.57, 3.53):
        piece('entry bench end', (-2.36, y, .195), (.50, .045, .39), oak, 'entry_bench')
    piece('entry shoe shelf', (-2.36, 3.05, .12), (.47, .94, .028), oak, 'entry_bench')
    seat = piece('entry bench cushion', (-2.36, 3.05, .44), (.54, 1.10, .075),
                 bpy.data.materials[FABRIC_NAME], 'entry_bench')
    seat.modifiers['Soft manufactured edge'].width = .032
    unwrap_cushions(seat, 0)
    piece('entry art frame', (-2.70, 3.05, 1.53), (.035, .96, .83), oak, 'entry_art')
    canvas = material('Entry warm canvas', (.67, .62, .52), .95)
    sage = material('Entry sage artwork', (.18, .28, .23), .95)
    clay = material('Entry clay artwork', (.42, .23, .13), .95)
    piece('entry canvas', (-2.677, 3.05, 1.53), (.008, .90, .77), canvas, 'entry_art')
    piece('sage abstract panel', (-2.67, 2.89, 1.60), (.005, .29, .48), sage, 'entry_art')
    piece('clay abstract panel', (-2.664, 3.21, 1.40), (.005, .25, .31), clay, 'entry_art')
    piece('entry door', (-.35, 4.97, 1.14), (1.06, .045, 2.28), oak, 'entry_door')
    for x in (-.895, .195):
        piece('entry door jamb', (x, 4.94, 1.17), (.035, .09, 2.34), dark, 'entry_door')
    piece('entry door lintel', (-.35, 4.94, 2.31), (1.125, .09, .065), dark, 'entry_door')
    piece('entry transom infill', (-.35, 4.97, 2.62), (1.10, .15, .56),
          bpy.data.materials['Warm plaster'], 'entry_door')
    rod('entry door handle', (-.77, 4.922, 1.02), (-.62, 4.922, 1.02), .009, dark, 'entry_door')


def workspace_solids():
    return grouped_solids('workspace_group',
        {'workspace_desk', 'workspace_chair', 'entry_storage', 'entry_bench', 'entry_door'})


def workspace_lights():
    return [{'id':'workspace_lamp', 'position':[1.43, 1.29, -3.89],
             'color_srgb':[1.0, .83, .64], 'lumens':240.0, 'range_m':2.0,
             'radius_m':.04, 'shadows':True,
             'diffuser':{'mesh':bpy.data.objects[PREFIX + 'lamp diffuser'].data.name, 'emissive_strength':35.0}}]


def workspace_camera():
    return {'id':'workspace', 'position':[-1.4, 1.65, .15],
            'look_at':[-.15, 1.15, -3.5], 'fov_deg':72}


def replace_workspace():
    for obj in list(collection().objects):
        if obj.name.startswith((PREFIX, 'Desk ', 'Monitor', 'Entry storage')):
            bpy.data.objects.remove(obj, do_unlink=True)
    desk()
    workstation()
    task_lamp()
    chair()
    shelves()
    entry()
    for obj in collection().objects:
        if obj.name.startswith(PREFIX):
            obj['source'] = 'Original static workspace and entry; existing CC0 oak and upholstery maps'
    print('WORKSPACE_AUTHORED', sum(o.name.startswith(PREFIX) for o in collection().objects), flush=True)


if __name__ == '__main__':
    update_saved_environments(ROOT, replace_workspace)
    update_manifest_entries(ROOT, 'solids', workspace_solids())
    update_manifest_entries(ROOT, 'cameras', [workspace_camera()])
    from export_practical_lights import export
    export()
    os._exit(0)
