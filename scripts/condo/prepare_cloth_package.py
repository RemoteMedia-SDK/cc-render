"""Build opt-in live cloth assets, measuring bed solids from the Blender source.

Run in Blender. Original scene/GLB and the articulated-only package stay intact.
"""
import hashlib
import json
from pathlib import Path
import sys

import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
SCENE = ROOT/'scenes/toronto_condo'


def build():
    source=SCENE/'toronto-condo.blend'
    before=hashlib.sha256(source.read_bytes()).hexdigest()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    prefixes=('Bedroom detail side rail','Bedroom detail end rail','Bedroom detail recessed foot')
    with bpy.data.libraries.load(str(source),link=False) as (available,requested):
        requested.objects=[n for n in available.objects if n.startswith(prefixes) or n=='Bedroom detail mattress']
    objects=requested.objects
    assert len(objects)==9, [o.name for o in objects]
    for obj in objects:bpy.context.collection.objects.link(obj)
    bpy.context.view_layer.update()
    scene=json.loads((SCENE/'scene-articulated.json').read_text())
    scene['solids']=[s for s in scene['solids'] if s['id']!='bed_base']
    measured=[]
    for index,obj in enumerate(sorted(objects,key=lambda o:o.name)):
        points=[obj.matrix_world@Vector(c) for c in obj.bound_box]
        low=Vector([min(p[i] for p in points) for i in range(3)])
        high=Vector([max(p[i] for p in points) for i in range(3)])
        centre,half=(low+high)/2,(high-low)/2
        mattress=obj.name.endswith('mattress')
        solid={'id':'bed_mattress' if mattress else f'bed_frame_{index:02}',
            'position':[centre.x,centre.z,-centre.y], 'half_extents':[half.x,half.z,half.y],
            'border_radius_m':min(high-low)*.38 if mattress else 0.0,'navigation_obstacle':True}
        scene['solids'].append(solid)
        measured.append({'source_object':obj.name,'solid':solid})
    bedding=SCENE/'interaction-authoring/bedding'
    original=json.loads((bedding/'top-sheet-surface.json').read_text())
    asset={'schema_version':1,'rest_positions':original['positions'],
        # Authored placement: clear the neighbouring lamp's conservative collider.
        'initial_positions':[[x-.03,y,z] for x,y,z in original['positions']],
        'triangles':original['triangles'],'material_uv':original['material_uv']}
    target=bedding/'runtime-top-sheet.json'
    target.write_text(json.dumps(asset,separators=(',',':'))+'\n')
    replaced={m['replaces_solid'] for m in scene['mechanisms']}
    scene['cloths']=[{'id':'bed_top_sheet','surface':str(target.relative_to(SCENE)),
        'sha256':hashlib.sha256(target.read_bytes()).hexdigest(),
        'parameters':{'total_mass_kg':.8,'thickness_m':.001,'stretch_compliance':1e-7,
            'bend_compliance':.05,'air_drag_per_second':.2,'spring_damping_ratio':1.,'substeps':24},
        'collision_solids':[s['id'] for s in scene['solids'] if s['id'] not in replaced],
        'replaces_nodes':['Bedroom detail folded duvet'],'color_srgb':[.83,.80,.72]}]
    scene['id']+='-cloth-prototype'
    (SCENE/'scene-cloth.json').write_text(json.dumps(scene,indent=2)+'\n')
    evidence={'source_sha256':before,'source_unchanged':before==hashlib.sha256(source.read_bytes()).hexdigest(),
        'measured_bed_solids':measured,'initial_translation_m':[-.03,0,0],'mattress_radius_basis':'soft_piece uses min(size)*0.38; rounded box approximates authored bevel',
        'limits':['Static duvet hidden in this package only. Pillows remain static; no cloth layer coupling.',
                  'No avatar/body or dynamic-furniture collision, self-collision or hand attachment.']}
    (bedding/'runtime-package-authoring.json').write_text(json.dumps(evidence,indent=2)+'\n')
    assert evidence['source_unchanged']
    print('CLOTH_PACKAGE_READY',SCENE/'scene-cloth.json',flush=True)


if __name__=='__main__':build()
