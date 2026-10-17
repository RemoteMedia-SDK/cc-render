"""Hollow fridge and separate door assemblies; asset fixture, not avatar motion."""
import hashlib
import json
import math
import os
from pathlib import Path
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from tabletop_props import box, material
from scene_export import export_selection

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'scenes/toronto_condo/interaction-authoring/fridge'


def runtime(point):
    return [float(point[0]), float(point[2]), float(-point[1])]


def assembly(name, origin, mass):
    collection = bpy.data.collections.new(name)
    bpy.context.scene.collection.children.link(collection)
    pivot = bpy.data.objects.new(name, None)
    collection.objects.link(pivot)
    pivot.location = origin
    return {'id':name, 'origin':origin, 'mass_kg':mass, 'collection':collection,
            'pivot':pivot, 'colliders':[], 'meshes':[]}


def panel(part, name, centre, size, surface):
    obj = box(name, centre, size, surface, part['collection'])
    obj.parent = part['pivot']
    obj.location = Vector(centre) - Vector(part['origin'])
    part['meshes'].append(obj)
    part['colliders'].append({'id':name, 'position':runtime(obj.location),
                             'half_extents':[size[0]/2,size[2]/2,size[1]/2]})
    return obj


def shell(p):
    body = assembly('fridge_body', (0,0,0), None)
    # Original envelope: 0.80 deep x 0.73 wide x 2.36 high. Front is +X.
    panel(body,'fridge_back',(-.375,0,1.18),(.05,.73,2.36),p['case'])
    for y in (-.34,.34):
        panel(body,'fridge_side_'+str(y),(.015,y,1.18),(.73,.05,2.36),p['case'])
    for z in (.035,2.325):
        panel(body,'fridge_cap_'+str(z),(.015,0,z),(.73,.63,.07),p['liner'])
    panel(body,'freezer_divider',(.015,0,.7475),(.73,.63,.025),p['liner'])
    for z in (.34,1.10,1.45,1.80,2.12):
        panel(body,'fridge_shelf_'+str(z),(-.015,0,z),(.65,.62,.014),p['shelf'])
    panel(body,'fridge_liner_back',(-.345,0,1.18),(.01,.63,2.22),p['liner'])
    return body


def door(name, centre_z, height, mass, p):
    # North/front edge. Positive Blender Z maps to runtime +Y.
    part = assembly(name,(.4225,.3575,centre_z),mass)
    panel(part,name+'_outer',(.4225,0,centre_z),(.045,.715,height),p['steel'])
    panel(part,name+'_liner',(.395,0,centre_z),(.010,.61,height-.08),p['liner'])
    panel(part,name+'_handle',(.480,-.25,centre_z+.10),(.026,.022,.40),p['handle'])
    for z in (centre_z-.075,centre_z+.275):
        panel(part,name+'_handle_mount_'+str(z),(.458,-.25,z),(.024,.025,.025),p['handle'])
    part['joint']={'kind':'revolute','parent':'fridge_body',
        'parent_anchor':runtime(part['origin']),'child_anchor':[0,0,0],
        'axis':[0,1,0],'limits_rad':[0,math.radians(90)],'motor':None}
    part['handle_region']={'mesh':name+'_handle',
        'centre':runtime(Vector((.480,-.25,centre_z+.10))-Vector(part['origin'])),
        'half_extents':[.013,.20,.011]}
    return part


def add_blender_mechanics(parts):
    """Separate Bullet diagnostic proxies; runtime compound shapes are in JSON."""
    proxies = {}
    for part in parts:
        copies=[]
        for obj in part['meshes']:
            clone=obj.copy();clone.data=obj.data.copy();bpy.context.scene.collection.objects.link(clone)
            world=obj.matrix_world.copy();clone.parent=None;clone.matrix_world=world;copies.append(clone)
        bpy.ops.object.select_all(action='DESELECT')
        for obj in copies:obj.select_set(True)
        bpy.context.view_layer.objects.active=copies[0]
        bpy.ops.object.join();proxy=bpy.context.object;proxy.name=part['id']+'_physics_fixture'
        proxy.hide_render=True;proxy.display_type='WIRE'
        bpy.ops.rigidbody.object_add()
        proxy.rigid_body.type='PASSIVE' if part['mass_kg'] is None else 'ACTIVE'
        proxy.rigid_body.collision_shape='MESH' if part['mass_kg'] is None else 'CONVEX_HULL'
        if part['mass_kg'] is not None:proxy.rigid_body.mass=part['mass_kg']
        proxy.rigid_body.use_margin=True;proxy.rigid_body.collision_margin=.001
        proxies[part['id']]=proxy
    for part in parts[1:]:
        bpy.ops.object.empty_add(type='PLAIN_AXES',location=part['origin'])
        hinge=bpy.context.object;hinge.name=part['id']+'_passive_hinge'
        bpy.ops.rigidbody.constraint_add()
        constraint=hinge.rigid_body_constraint;constraint.type='HINGE'
        constraint.object1=proxies['fridge_body'];constraint.object2=proxies[part['id']]
        # Bullet reports object1 relative to object2; the exported contract
        # reports the door relative to its fixed parent, hence the sign flip.
        constraint.use_limit_ang_z=True;constraint.limit_ang_z_lower=-math.radians(90)
        constraint.limit_ang_z_upper=0
        constraint.disable_collisions=False


def export(parts):
    payload={'schema_version':1,'id':'kitchen_fridge','coordinates':'runtime X east Y up Z south; metres',
             'placement':[-5.55,0,-1.45],'parts':[],
             'replaces_solid':'kitchen_fridge','replaces_object_prefix':'Kitchen detail fridge ',
             'runtime_enabled':False,
             'limits':['Masses are initial engineering estimates, not measured product data.',
                       'No avatar grasp or scene collision replacement is enabled.',
                       'Diagnostic open poses are mechanical inspection only; no animated opening is exported.']}
    for part in parts:
        origin=part['pivot'].location.copy();part['pivot'].location=(0,0,0)
        bpy.context.view_layer.update()
        name=part['id']+'.glb';path=OUT/name
        export_selection(path,[part['collection']])
        part['pivot'].location=origin;bpy.context.view_layer.update()
        row={'id':part['id'],'geometry':name,'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
             'position':runtime(part['origin']),'mass_kg':part['mass_kg'],'colliders':part['colliders']}
        for key in ('joint','handle_region'):
            if key in part:row[key]=part[key]
        payload['parts'].append(row)
    (OUT/'mechanism.json').write_text(json.dumps(payload,indent=2)+'\n')
    return payload


def build():
    OUT.mkdir(parents=True,exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    palette={'case':material('Fridge charcoal shell',(.08,.09,.10),.42),
             'liner':material('Fridge ivory liner',(.73,.76,.73),.36),
             'shelf':material('Fridge shelf',(.58,.64,.62),.27),
             'steel':material('Fridge brushed steel',(.36,.38,.37),.28),
             'handle':material('Fridge handle',(.25,.27,.26),.24)}
    for name in ('steel','handle'):
        palette[name].node_tree.nodes['Principled BSDF'].inputs['Metallic'].default_value=.85
    parts=[shell(palette),door('fridge_freezer_door',.40,.68,6,palette),
           door('fridge_main_door',1.55,1.59,12,palette)]
    bpy.context.view_layer.update()
    payload=export(parts)
    add_blender_mechanics(parts)
    bpy.context.scene['scope']='Asset mechanics fixture only; not connected to the avatar or active condo'
    bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'fridge-mechanism.blend'))
    print('FRIDGE_ASSEMBLIES_EXPORTED',len(payload['parts']),sum(len(p['colliders']) for p in payload['parts']),flush=True)


if __name__ == '__main__':
    build()
    os._exit(0)
