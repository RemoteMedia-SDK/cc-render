"""Author a wall caddy and three loose toiletry bottles; no avatar motion."""
import hashlib
import json
import math
import os
from pathlib import Path
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from fridge_mechanism import assembly, panel, runtime
from bedside_mechanism import export_parts, inspection_studio, SCENE
from tabletop_props import material, lathe, finish
from caddy_interactions import add_interactions

OUT = SCENE / 'interaction-authoring/shower-caddy'
# Blender Z up, metres. Shelf front leaves the normal shower arrival region clear.
BASE = (-5.82, 4.30, 1.30)


def caddy(palette):
    part = assembly('shower_caddy', (0, 0, 0), None)
    panel(part, 'caddy_wall_spine', (-.073, 0, .12), (.014, .52, .30), palette['metal'])
    panel(part, 'caddy_shelf', (0, 0, -.009), (.16, .56, .018), palette['metal'])
    panel(part, 'caddy_front_rail', (.076, 0, .025), (.008, .56, .025), palette['metal'])
    for y in (-.276, .276):
        panel(part, 'caddy_end_rail_'+str(y), (0, y, .025), (.16, .008, .025), palette['metal'])
    for y in (-.22, .22):
        panel(part, 'caddy_mount_'+str(y), (-.078, y, .225), (.012, .035, .045), palette['metal'])
    return part


def bottle(name, caption, y, colour, palette):
    part = assembly(name, (0, y, .112), .30)
    body = material(name+' satin', colour, .32)
    profile = [(0, -.11), (.027, -.11), (.033, -.104), (.033, .065),
               (.030, .080), (.013, .092), (.013, .106), (0, .106)]
    obj = bpy.data.objects.new(name+'_body', lathe(profile, 48))
    part['collection'].objects.link(obj)
    finish(obj, name+'_body', body, part['collection'])
    obj.parent = part['pivot']
    # Physical barrel and neck share the authored bottle origin.
    part['colliders'] = [
        {'id':name+'_barrel', 'position':[0, -.014, 0], 'half_extents':[.033, .096, .033]},
        {'id':name+'_neck', 'position':[0, .098, 0], 'half_extents':[.013, .010, .013]}]
    # Open dispensing mouth, visible dark recess; no animated cap shortcut.
    mouth = bpy.data.objects.new(name+'_mouth', lathe([(0,.1061),(.009,.1061),(.009,.107),(0,.107)],32))
    part['collection'].objects.link(mouth)
    finish(mouth, name+'_mouth', palette['dark'], part['collection'])
    mouth.parent = part['pivot']
    curve = bpy.data.curves.new(name+'_label', 'FONT')
    curve.body, curve.align_x, curve.size = caption, 'CENTER', .009
    text = bpy.data.objects.new(name+'_label', curve)
    part['collection'].objects.link(text)
    text.parent = part['pivot']
    text.location = (.0335, 0, .018)
    # Text plane faces into the shower (+X), its baseline across the bottle.
    text.rotation_euler = (math.pi/2, 0, math.pi/2)
    curve.materials.append(palette['ink'])
    bpy.ops.object.select_all(action='DESELECT')
    text.select_set(True)
    bpy.context.view_layer.objects.active = text
    bpy.ops.object.convert(target='MESH')
    return part


def prop(row, label):
    boxes = row['colliders']
    lo = [min(s['position'][k]-s['half_extents'][k] for s in boxes) for k in range(3)]
    hi = [max(s['position'][k]+s['half_extents'][k] for s in boxes) for k in range(3)]
    # Keep the original asset origin; a symmetric conservative metadata envelope.
    half = [max(abs(a), abs(b)) for a,b in zip(lo,hi)]
    position = [a+b for a,b in zip(runtime(BASE), row['position'])]
    dynamic = row['mass_kg'] is not None
    parts = [{'position':s['position'], 'half_extents':s['half_extents'],
              'cylinder':dynamic} for s in boxes]
    obj = {'id':row['id'], 'label':label, 'position':position, 'half_extents':half,
           'color':[.4,.4,.35], 'dynamic':dynamic, 'parts':parts,
           'grasp':[0,0,0] if dynamic else None, 'seat':None, 'surface':None}
    return {'object':obj, 'geometry':row['geometry'], 'sha256':row['sha256'], 'mass_kg':row['mass_kg']}


def build(output):
    output.mkdir(parents=True, exist_ok=False)
    OUT.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    palette = {k:material('Caddy '+k,c,r) for k,c,r in [
        ('metal',(.10,.085,.06),.30), ('dark',(.015,.012,.009),.4), ('ink',(.82,.78,.65),.6)]}
    palette['metal'].node_tree.nodes['Principled BSDF'].inputs['Metallic'].default_value = .8
    parts = [caddy(palette),
        bottle('shampoo','SHAMPOO',0,(.12,.21,.18),palette),
        bottle('conditioner','CONDITIONER',-.18,(.44,.36,.23),palette),
        bottle('body_wash','BODY WASH',.18,(.23,.14,.08),palette)]
    bpy.context.view_layer.update()
    rows = export_parts(parts, OUT)
    scene = json.loads((SCENE/'scene-lightmap.json').read_text())
    scene['id'] += '-shower-caddy'
    scene['props'] = [prop(r,label) for r,label in zip(rows,
        ['Bronze shower caddy','Shampoo bottle','Conditioner bottle','Body wash bottle'])]
    # The shelf protrudes into the shower. Keep the shared navigation destination
    # 30cm clear of its front, still under the shower spray; this is scene metadata,
    # not an avatar pose or a prescribed approach trajectory.
    shelf = scene['props'][0]['object']
    approach = [shelf['position'][0]+shelf['half_extents'][0]+.30, 0., shelf['position'][2]]
    destinations = [p['object'] for p in scene['props']]
    destinations.append(next(s for s in scene['solids'] if s['id'] == 'shower_screen'))
    for obj in destinations:
        obj['approach'] = [a-p for a,p in zip(approach,obj['position'])]
    add_interactions(scene)
    for row in scene['props'][1:]:
        obj = row['object']
        scene['water_fixtures'].append({'id':obj['id']+'_liquid','kind':'pour',
            'outlet':[obj['position'][0],obj['position'][1]+.107,obj['position'][2]],
            'surface_y':.025,'spray_radius_m':.002,'max_liters_per_minute':.3,
            'container':{'prop':obj['id'],'mouth':[0,.107,0],'capacity_liters':.25}})
    scene['cameras'].append({'id':'shower-caddy','position':[-4.35,1.65,-3.55],
        'look_at':[-5.76,1.36,-4.30], 'fov_deg':58})
    (SCENE/'scene-shower-caddy.json').write_text(json.dumps(scene,indent=2)+'\n')
    preview = inspection_studio((1,-1,.55), (0,0,.10), .85, ((.6,-.5,1.4),65,1.2))
    preview.render.filepath = str(output/'caddy.png')
    bpy.ops.render.render(write_still=True)
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'shower-caddy.blend'))
    (output/'capture.json').write_text(json.dumps({'scope':__doc__,'props':scene['props'],
        'limits':['Open dispensing mouths; no cap manipulation.',
                  'Bottle colliders approximate barrel and neck; mass estimated at 300g.',
                  'Authoring preview is not avatar interaction proof.']},indent=2))
    print('SHOWER_CADDY_EXPORTED',flush=True)


if __name__ == '__main__':
    build(Path(sys.argv[sys.argv.index('--')+1]).resolve())
    os._exit(0)
