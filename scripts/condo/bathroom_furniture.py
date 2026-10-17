"""Static warm-modern bathroom fixtures, authored in Blender metres."""
import os
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from kitchen_furniture import tube
from scene_export import ensure_surface_uv, veneer_uv, grouped_solids, update_saved_environments, update_manifest_entries
from tabletop_props import box, finish, lathe, material
from upholstery import NAME as FABRIC_NAME, unwrap_cushions

ROOT = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
PREFIX = 'Bathroom detail '
STONE = 'Honed travertine - ambientCG Travertine009'


def collection():
    return next(c for c in bpy.data.collections if c.name.startswith('02 '))


def piece(name, centre, size, surface, group):
    obj = box(PREFIX + name, centre, size, surface, collection())
    obj['bathroom_group'] = group
    ensure_surface_uv(obj)
    if surface.name == 'Kitchen light oak veneer':
        veneer_uv(obj)
    return obj


def pipe(name, points, radius, surface, group):
    obj = tube(collection(), {'steel':surface}, name, points, radius, group=group)
    obj.name = obj.data.name = PREFIX + name
    del obj['kitchen_group']
    obj['bathroom_group'] = group
    return obj


def vessel(name, profile, location, surface, group):
    obj = bpy.data.objects.new(PREFIX + name, lathe(profile))
    collection().objects.link(obj)
    finish(obj, obj.name, surface, collection())
    obj.location = location
    obj['bathroom_group'] = group
    ensure_surface_uv(obj)
    return obj


def palette():
    metal = material('Bathroom brushed nickel', (.48, .50, .49), .25)
    metal.node_tree.nodes['Principled BSDF'].inputs['Metallic'].default_value = .9
    mirror = material('Bathroom environment-reflective mirror', (.72, .74, .73), .06)
    mirror.node_tree.nodes['Principled BSDF'].inputs['Metallic'].default_value = 1
    glass = material('Bathroom clear shower glazing', (.68, .78, .75), .12)
    glass.node_tree.nodes['Principled BSDF'].inputs['Alpha'].default_value = .12
    glass.surface_render_method = 'DITHERED'
    return {'oak':bpy.data.materials['Kitchen light oak veneer'],
            'stone':bpy.data.materials[STONE], 'metal':metal, 'mirror':mirror, 'glass':glass,
            'ceramic':material('Bathroom glazed ivory ceramic', (.78, .76, .70), .20),
            'dark':material('Bathroom shadow reveals', (.025, .028, .026), .6),
            'opal':material('Bathroom opal diffuser', (.80, .76, .65), .6)}


def vanity(p):
    group = 'vanity'
    piece('vanity bottom', (-5.58, 2.83, .30), (.65, 1.10, .035), p['oak'], group)
    piece('vanity back', (-5.89, 2.83, .56), (.025, 1.10, .52), p['oak'], group)
    for y in (2.29, 3.37):
        piece('vanity side', (-5.58, y, .56), (.65, .025, .52), p['oak'], group)
    for z in (.435, .695):
        piece('vanity drawer', (-5.241, 2.83, z), (.026, 1.07, .242), p['oak'], group)
        piece('vanity finger rail', (-5.222, 2.83, z + .096), (.012, .90, .012), p['dark'], group)
    counter = piece('stone basin counter', (-5.56, 2.83, .86), (.72, 1.16, .045), p['stone'], group)
    bpy.ops.mesh.primitive_cylinder_add(vertices=64, radius=.278, depth=.4, location=(-5.51, 2.85, .86))
    cutter = bpy.context.object
    cutter.scale.x = .76
    bpy.context.view_layer.objects.active = counter
    cut = counter.modifiers.new('Recessed basin aperture', 'BOOLEAN')
    cut.operation, cut.solver, cut.object = 'DIFFERENCE', 'EXACT', cutter
    bpy.ops.object.modifier_move_up(modifier=cut.name)
    bpy.ops.object.modifier_move_up(modifier=cut.name)
    bpy.ops.object.modifier_apply(modifier=cut.name)
    bpy.data.objects.remove(cutter, do_unlink=True)
    bowl = vessel('recessed ceramic basin', [(0, .675), (.05, .675), (.19, .70),
        (.276, .855), (.29, .872), (.29, .887), (.266, .887), (.255, .84),
        (.17, .72), (.05, .703), (0, .703)], (-5.51, 2.85, 0), p['ceramic'], group)
    bowl.scale.x = .76
    vessel('basin drain', [(0, 0), (.026, 0), (.026, .003), (0, .003)],
           (-5.51, 2.85, .704), p['metal'], group)
    pipe('basin faucet', [(-5.80, 2.85, .884), (-5.80, 2.85, 1.10),
         (-5.51, 2.85, 1.10), (-5.51, 2.85, 1.05)], .012, p['metal'], 'bathroom_faucet')
    pipe('mixer lever', [(-5.80, 2.85, 1.02), (-5.80, 2.96, 1.04)], .008, p['metal'], 'bathroom_faucet')
    piece('stone backsplash', (-5.912, 2.83, 1.04), (.018, 1.16, .30), p['stone'], 'bathroom_backsplash')


def mirror_and_lights(p):
    group = 'bathroom_mirror'
    piece('mirror frame', (-5.906, 2.83, 1.65), (.034, .88, .94), p['metal'], group)
    piece('mirror surface', (-5.885, 2.83, 1.65), (.004, .846, .906), p['mirror'], group)
    for name, y in [('south', 2.30), ('north', 3.36)]:
        group = 'bathroom_sconce_' + name
        piece('sconce mount ' + name, (-5.90, y, 1.65), (.04, .055, .12), p['metal'], group)
        pipe('sconce diffuser ' + name, [(-5.855, y, 1.34), (-5.855, y, 1.96)],
             .022, p['opal'], group)
    vessel('ceiling trim', [(0, 0), (.145, 0), (.145, .02), (0, .02)],
           (-4.30, 3.50, 2.85), p['metal'], 'bathroom_ceiling_light')
    vessel('ceiling diffuser', [(0, 0), (.128, 0), (.128, .004), (0, .004)],
           (-4.30, 3.50, 2.844), p['opal'], 'bathroom_ceiling_light')


def shower(p):
    piece('shower floor', (-5.23, 4.30, .021), (1.35, 1.20, .004), p['stone'], 'shower_floor')
    piece('linear shower drain', (-5.80, 4.30, .024), (.025, .65, .003), p['dark'], 'shower_floor')
    piece('shower west tile', (-5.913, 4.30, 1.23), (.018, 1.21, 2.42), p['stone'], 'shower_west_cladding')
    piece('shower north tile', (-5.23, 4.912, 1.23), (1.37, .018, 2.42), p['stone'], 'shower_north_cladding')
    piece('fixed shower screen', (-5.23, 3.695, 1.15), (1.35, .012, 2.26), p['glass'], 'shower_screen')
    for x in (-5.90, -4.56):
        piece('shower screen edge', (x, 3.695, 1.15), (.012, .018, 2.26), p['metal'], 'shower_screen')
    piece('shower screen top', (-5.23, 3.695, 2.28), (1.35, .018, .014), p['metal'], 'shower_screen')
    pipe('rain shower arm', [(-5.90, 4.30, 2.23), (-5.55, 4.30, 2.23),
         (-5.55, 4.30, 2.18)], .012, p['metal'], 'shower_head')
    vessel('rain shower head', [(0, 0), (.12, 0), (.12, .018), (0, .018)],
           (-5.55, 4.30, 2.16), p['metal'], 'shower_head')
    piece('shower mixer plate', (-5.895, 4.30, 1.10), (.025, .16, .12), p['metal'], 'shower_mixer')
    pipe('shower mixer handle', [(-5.87, 4.30, 1.10), (-5.82, 4.30, 1.10),
         (-5.82, 4.40, 1.10)], .009, p['metal'], 'shower_mixer')


def toilet(p):
    group = 'bathroom_toilet'
    bowl = vessel('toilet ceramic body', [(0, 0), (.13, 0), (.145, .04),
        (.13, .15), (.16, .28), (.225, .36), (.245, .39), (.245, .405),
        (.21, .405), (.20, .38), (.16, .30), (.08, .26), (0, .26)],
        (-3.35, 4.30, .02), p['ceramic'], group)
    bowl.scale = (.82, 1.25, 1)
    lid = vessel('closed toilet seat lid', [(0, 0), (.23, 0), (.248, .006),
        (.25, .015), (.243, .026), (.22, .03), (0, .03)],
        (-3.35, 4.30, .434), p['ceramic'], group)
    lid.scale = (.82, 1.25, 1)
    tank = piece('toilet cistern', (-3.35, 4.62, .53), (.40, .20, .62), p['ceramic'], group)
    tank.modifiers['Soft manufactured edge'].width = .035
    tank.modifiers['Soft manufactured edge'].segments = 6
    piece('cistern lid', (-3.35, 4.62, .852), (.415, .215, .025), p['ceramic'], group)
    piece('toilet rear support', (-3.35, 4.49, .20), (.25, .20, .36), p['ceramic'], group)
    vessel('flush button', [(0, 0), (.022, 0), (.022, .005), (0, .005)],
           (-3.35, 4.62, .866), p['metal'], group)
    pipe('paper holder', [(-2.89, 4.02, .73), (-3.02, 4.02, .73), (-3.02, 4.16, .73)],
         .008, p['metal'], 'bathroom_paper_holder')
    pipe('paper roll', [(-3.02, 4.04, .73), (-3.02, 4.14, .73)], .052,
         material('Bathroom soft paper', (.74, .72, .66), .95), 'bathroom_paper_holder')


def floor_and_accessories(p):
    floor = bpy.data.objects['Bathroom floor']
    floor.data.materials.clear()
    floor.data.materials.append(p['stone'])
    floor['bathroom_group'] = 'bathroom_floor'
    grout = material('Bathroom stone grout', (.29, .29, .26), .9)
    for x in (-5.315, -4.705, -4.095, -3.485):
        piece('floor grout', (x, 3.5, .0201), (.003, 2.85, .0002), grout, 'bathroom_floor')
    for y in (2.645, 3.215, 3.785, 4.355):
        piece('floor grout', (-4.4, y, .0201), (3.05, .003, .0002), grout, 'bathroom_floor')
    cloth = bpy.data.materials[FABRIC_NAME]
    towel = piece('folded hand towel', (-5.204, 2.53, .57), (.022, .32, .30), cloth, 'bathroom_accessories')
    unwrap_cushions(towel, 0)
    pipe('towel rail', [(-5.22, 2.34, .70), (-5.18, 2.34, .70),
         (-5.18, 2.72, .70), (-5.22, 2.72, .70)], .007, p['metal'], 'bathroom_accessories')
    vessel('soap dispenser', [(0, 0), (.035, 0), (.04, .015), (.032, .13), (0, .13)],
           (-5.54, 2.42, .884), p['ceramic'], 'bathroom_accessories')
    pipe('soap pump', [(-5.54, 2.42, 1.014), (-5.54, 2.42, 1.05), (-5.49, 2.42, 1.05)],
         .008, p['metal'], 'bathroom_accessories')


def bathroom_solids():
    return grouped_solids('bathroom_group', {'vanity', 'shower_screen', 'bathroom_toilet'})


def bathroom_lights():
    # Approximate opal surface luminance: lumens / (emitting area * pi).
    # Keep the room's point-light power independent from visible diffuser emission.
    lights = [{'id':'bathroom_sconce_' + name, 'position':[-5.81, 1.65, -y],
        'color_srgb':[1, .88, .72], 'lumens':500, 'range_m':2.4, 'radius_m':.06,
        'shadows':True, 'diffuser':{'mesh':bpy.data.objects[PREFIX + 'sconce diffuser ' + name].data.name, 'emissive_strength':1800}}
        for name, y in [('south', 2.30), ('north', 3.36)]]
    lights.append({'id':'bathroom_ceiling', 'position':[-4.30, 2.77, -3.50],
        'color_srgb':[1, .90, .78], 'lumens':2200, 'range_m':4, 'radius_m':.12,
        'shadows':True, 'diffuser':{'mesh':bpy.data.objects[PREFIX + 'ceiling diffuser'].data.name, 'emissive_strength':12000}})
    return lights


def bathroom_camera():
    return {'id':'bathroom', 'position':[-3.65, 1.65, -2.25],
            'look_at':[-4.90, 1.05, -3.8], 'fov_deg':83}


def replace_bathroom():
    for obj in list(collection().objects):
        if obj.name.startswith(PREFIX) or obj.name in ('Vanity', 'Vanity top', 'Shower tray', 'Toilet proxy'):
            bpy.data.objects.remove(obj, do_unlink=True)
    p = palette()
    vanity(p)
    mirror_and_lights(p)
    shower(p)
    toilet(p)
    floor_and_accessories(p)
    for obj in collection().objects:
        if obj.name.startswith(PREFIX):
            obj['source'] = 'Original static bathroom fixtures; existing CC0 oak, travertine and fabric maps'
    print('BATHROOM_AUTHORED', sum(o.name.startswith(PREFIX) for o in collection().objects), flush=True)


if __name__ == '__main__':
    update_saved_environments(ROOT, replace_bathroom)
    update_manifest_entries(ROOT, 'solids', bathroom_solids())
    update_manifest_entries(ROOT, 'cameras', [bathroom_camera()])
    from export_practical_lights import export
    export()
    os._exit(0)
