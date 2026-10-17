"""Original warm-modern kitchen geometry; no avatar motion or prop actuators."""
import math
import os
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from scene_export import ensure_surface_uv, grouped_solids, update_saved_environments, update_solid_manifests
from tabletop_props import box, finish, material

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
PREFIX = 'Kitchen detail '
STONE = 'Honed travertine - ambientCG Travertine009'


def surfaces():
    oak = bpy.data.materials.get('Kitchen light oak veneer')
    if oak is None:
        oak = bpy.data.materials['Oak floor - Poly Haven wood_floor'].copy()
        oak.name = 'Kitchen light oak veneer'
    oak.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value = .48
    steel = material('Kitchen brushed steel', (.36, .38, .37), .28)
    steel.node_tree.nodes.get('Principled BSDF').inputs['Metallic'].default_value = .85
    return {'oak': oak, 'stone': bpy.data.materials[STONE], 'steel': steel,
            'black': material('Kitchen black ceramic', (.009, .012, .013), .18),
            'dark': material('Kitchen recessed shadow', (.024, .028, .026), .7),
            'ring': material('Kitchen induction etching', (.18, .20, .19), .45)}


def panel(collection, palette, name, centre, size, surface='oak', group='kitchen_cabinetry'):
    obj = box(PREFIX + name, centre, size, palette[surface], collection)
    obj['kitchen_group'] = group
    obj.modifiers['Soft manufactured edge'].width = min(.006, min(size) / 4)
    ensure_surface_uv(obj)
    for face in obj.data.polygons:
        axis = max(range(3), key=lambda n: abs(face.normal[n]))
        axes = ((1, 2), (0, 2), (0, 1))[axis]
        for index in face.loop_indices:
            point = obj.data.vertices[obj.data.loops[index].vertex_index].co
            uv = tuple(point[n] / 1.2 for n in axes)
            if surface == 'oak':
                # A continuous board patch becomes veneer on each cabinet panel.
                uv = (.012 + (point[axes[0]] / size[axes[0]] + .5) * .085,
                      .2 + (point[axes[1]] + size[axes[1]] / 2) / 1.7)
            obj.data.uv_layers[0].data[index].uv = uv
    return obj


def tube(collection, palette, name, points, radius, surface='steel', group='kitchen_cabinetry'):
    curve = bpy.data.curves.new(PREFIX + name, 'CURVE')
    curve.dimensions = '3D'
    curve.bevel_depth, curve.bevel_resolution = radius, 3
    curve.use_fill_caps = True
    spline = curve.splines.new('POLY')
    spline.points.add(len(points) - 1)
    for point, position in zip(spline.points, points):
        point.co = (*position, 1)
    obj = bpy.data.objects.new(PREFIX + name, curve)
    collection.objects.link(obj)
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.convert(target='MESH')
    finish(obj, obj.name, palette[surface], collection)
    obj['kitchen_group'] = group
    ensure_surface_uv(obj)
    return obj


def sink_and_counter(collection, palette):
    counter = panel(collection, palette, 'cutout worktop', (-5.55, -.175, .93), (.85, 2.5, .065), 'stone')
    cutter = box('Temporary sink aperture', (-5.52, .35, .93), (.51, .59, .3), palette['dark'], collection)
    bpy.context.view_layer.objects.active = counter
    cut = counter.modifiers.new('Actual sink opening', 'BOOLEAN')
    cut.operation, cut.solver, cut.object = 'DIFFERENCE', 'EXACT', cutter
    bpy.ops.object.modifier_move_up(modifier=cut.name)
    bpy.ops.object.modifier_move_up(modifier=cut.name)
    bpy.ops.object.modifier_apply(modifier=cut.name)
    bpy.data.objects.remove(cutter, do_unlink=True)
    # The body is a hollow cabinet shell; the basin sits below the cutout.
    panel(collection, palette, 'sink bottom', (-5.52, .35, .755), (.51, .59, .016), 'steel')
    for x in (-5.775, -5.265):
        panel(collection, palette, 'sink side', (x, .35, .847), (.014, .60, .19), 'steel')
    for y in (.055, .645):
        panel(collection, palette, 'sink end', (-5.52, y, .847), (.51, .014, .19), 'steel')
    bpy.ops.mesh.primitive_cylinder_add(vertices=48, radius=.033, depth=.003, location=(-5.52, .35, .765))
    drain = finish(bpy.context.object, PREFIX + 'sink drain', palette['dark'], collection)
    drain['kitchen_group'] = 'kitchen_cabinetry'
    points = [(-5.83, .35, .963), (-5.83, .35, 1.19)]
    points += [(-5.70 + .13 * math.cos(a), .35, 1.19 + .13 * math.sin(a))
               for a in [math.pi - i * math.pi / 32 for i in range(33)]]
    points.append((-5.57, .35, 1.10))
    tube(collection, palette, 'arched faucet', points, .014, group='kitchen_faucet')
    tube(collection, palette, 'faucet mixer connector', [(-5.83, .35, 1.01), (-5.83, .40, 1.01)], .012, group='kitchen_faucet')
    tube(collection, palette, 'faucet lever', [(-5.83, .40, 1.01), (-5.83, .50, 1.08)], .008, group='kitchen_faucet')


def base_cabinets(collection, palette):
    panel(collection, palette, 'base plinth', (-5.59, -.175, .065), (.66, 2.5, .13), 'dark')
    panel(collection, palette, 'base bottom', (-5.55, -.175, .15), (.8, 2.5, .035))
    panel(collection, palette, 'base back', (-5.935, -.175, .52), (.025, 2.5, .74))
    for y in (-1.425, -.8, -.175, 1.075):
        panel(collection, palette, 'base divider', (-5.55, y, .52), (.8, .018, .74))
    for y in (-.4875, .1375, .7625):
        for z, height in ((.29, .285), (.59, .285), (.82, .145)):
            panel(collection, palette, 'drawer front', (-5.137, y, z), (.026, .609, height))
            panel(collection, palette, 'drawer pull', (-5.114, y, z + height / 2 - .025), (.018, .22, .012), 'steel')
    panel(collection, palette, 'backsplash', (-5.915, -.175, 1.29), (.02, 2.5, .66), 'stone', 'kitchen_backsplash')
    for y in (-.35, .20, .75):
        panel(collection, palette, 'upper cabinet', (-5.72, y, 2.12), (.40, .54, .72), group='kitchen_upper')
        panel(collection, palette, 'upper door', (-5.505, y, 2.12), (.024, .524, .702), group='kitchen_upper')
        panel(collection, palette, 'upper finger rail', (-5.487, y, 1.79), (.018, .44, .014), 'dark', 'kitchen_upper')
    sink_and_counter(collection, palette)


def appliances(collection, palette):
    panel(collection, palette, 'oven casing', (-5.38, -1.11, .49), (.46, .60, .67), 'dark')
    panel(collection, palette, 'oven glass', (-5.135, -1.11, .46), (.027, .55, .46), 'black')
    panel(collection, palette, 'oven controls', (-5.12, -1.11, .79), (.03, .57, .10), 'steel')
    panel(collection, palette, 'oven handle', (-5.08, -1.11, .69), (.028, .43, .025), 'steel')
    panel(collection, palette, 'induction hob', (-5.53, -1.06, .968), (.61, .62, .012), 'black')
    for x in (-5.68, -5.38):
        for y in (-1.21, -.91):
            points = [(x + .10 * math.cos(a), y + .10 * math.sin(a), .975)
                      for a in [i * math.tau / 64 for i in range(65)]]
            tube(collection, palette, 'induction ring', points, .0011, 'ring')
    panel(collection, palette, 'hood canopy', (-5.64, -1.10, 1.88), (.59, .64, .10), 'steel', 'kitchen_hood')
    panel(collection, palette, 'hood chimney', (-5.80, -1.10, 2.23), (.25, .32, .60), 'steel', 'kitchen_hood')
    panel(collection, palette, 'fridge case', (-5.55, 1.45, 1.18), (.8, .73, 2.36), 'dark', 'kitchen_fridge')
    for z, height in ((.40, .68), (1.55, 1.59)):
        panel(collection, palette, 'fridge door', (-5.13, 1.45, z), (.045, .715, height), 'steel', 'kitchen_fridge')
        panel(collection, palette, 'fridge handle', (-5.086, 1.20, z + .10), (.035, .022, .40), 'steel', 'kitchen_fridge')


def island(collection, palette):
    group = 'island_cabinet'
    panel(collection, palette, 'island plinth', (-3.55, 0, .065), (.84, 1.98, .13), 'dark', group)
    panel(collection, palette, 'island body', (-3.55, 0, .505), (1, 2.15, .75), 'oak', group)
    for y in (-.71, 0, .71):
        panel(collection, palette, 'island front', (-3.037, y, .52), (.025, .696, .73), 'oak', group)
        for z, h in ((.30, .31), (.68, .42)):
            panel(collection, palette, 'island drawer', (-4.063, y, z), (.026, .696, h), 'oak', group)
            panel(collection, palette, 'island pull', (-4.089, y, z + h / 2 - .025), (.022, .28, .012), 'steel', group)
    panel(collection, palette, 'island stone top', (-3.55, 0, .94), (1.15, 2.30, .07), 'stone', group)
    for y in (-1.125, 1.125):
        panel(collection, palette, 'island waterfall end', (-3.55, y, .4725), (1.15, .05, .945), 'stone', group)


def kitchen_solids():
    return grouped_solids('kitchen_group', {'kitchen_cabinetry', 'kitchen_fridge', 'island_cabinet'})


def replace_kitchen():
    collection = next(c for c in bpy.data.collections if c.name.startswith('02 '))
    for obj in list(collection.objects):
        if obj.name.startswith(PREFIX) or obj.name in ('Kitchen cabinetry', 'Kitchen stone worktop', 'Sink', 'Island cabinet', 'Island stone worktop'):
            bpy.data.objects.remove(obj, do_unlink=True)
    palette = surfaces()
    base_cabinets(collection, palette)
    appliances(collection, palette)
    island(collection, palette)
    for obj in collection.objects:
        if obj.name.startswith(PREFIX):
            obj['source'] = 'Original procedural kitchen; existing CC0 oak and travertine maps'
    print('KITCHEN_AUTHORED', sum(o.name.startswith(PREFIX) for o in collection.objects), flush=True)


def update_collision_manifests():
    solids = kitchen_solids()
    update_solid_manifests(ROOT, solids)
    print('KITCHEN_COLLIDERS_EXPORTED', len(solids), flush=True)


if __name__ == '__main__':
    update_saved_environments(ROOT, replace_kitchen)
    update_collision_manifests()
    os._exit(0)
