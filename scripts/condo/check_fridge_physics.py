"""Blender-only passive-hinge fixture under labelled lateral gravity loads.

No avatar, motors or runtime actuators. This checks asset mechanics, not Bevy
integration or successful manipulation. The saved source scene is not changed.
"""
import json
import math
import os
from pathlib import Path
import sys

import bpy
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[2]
ASSETS=ROOT/'scenes/toronto_condo/interaction-authoring/fridge'


def studio(scene):
    for obj in bpy.data.objects:
        if obj.type=='MESH':obj.hide_render=not obj.name.endswith('_physics_fixture')
    bpy.ops.object.camera_add(location=(4,-4,3.2))
    camera=bpy.context.object
    camera.rotation_euler=(Vector((.20,0,1.15))-camera.location).to_track_quat('-Z','Y').to_euler()
    camera.data.type='ORTHO';camera.data.ortho_scale=5.2;scene.camera=camera
    bpy.ops.object.light_add(type='AREA',location=(3,-2,4))
    bpy.context.object.data.energy=500;bpy.context.object.data.size=4
    scene.world=bpy.data.worlds.new('Mechanics studio');scene.world.color=(.20,.20,.20)
    scene.render.engine='CYCLES';scene.cycles.device='CPU';scene.cycles.samples=16
    scene.render.resolution_x=1280;scene.render.resolution_y=720;scene.render.resolution_percentage=100


def check(out):
    out.mkdir(parents=True,exist_ok=True)
    bpy.ops.wm.open_mainfile(filepath=str(ASSETS/'fridge-mechanism.blend'))
    manifest=json.loads((ASSETS/'mechanism.json').read_text())
    scene=bpy.context.scene;scene.frame_end=481;scene.render.fps=24
    scene.gravity=(3,0,0)
    scene.rigidbody_world.substeps_per_frame=8;scene.rigidbody_world.solver_iterations=30
    scene.rigidbody_world.point_cache.frame_end=481
    studio(scene)
    bpy.context.view_layer.update()
    fixtures={}
    for part in manifest['parts'][1:]:
        obj=bpy.data.objects[part['id']+'_physics_fixture']
        anchor=Vector((part['position'][0],-part['position'][2],part['position'][1]))
        fixtures[part['id']]=(obj,anchor,obj.matrix_world.inverted()@anchor)
    samples=[]
    for frame in range(1,482):
        if frame==242:scene.gravity=(-3,-3,0)
        scene.frame_set(frame);deps=bpy.context.evaluated_depsgraph_get();deps.update()
        row={'frame':frame,'doors':{}}
        for name,(obj,anchor,local_anchor) in fixtures.items():
            transform=obj.evaluated_get(deps).matrix_world
            row['doors'][name]={'angle_deg':math.degrees(transform.to_quaternion().to_euler('XYZ').z),
                'anchor_error_m':((transform@local_anchor)-anchor).length}
        samples.append(row)
        if frame in (1,241,481):
            scene.render.filepath=str(out/f'fixture-{frame:04d}.png')
            bpy.ops.render.render(write_still=True)
    report={'load_schedule':[{'frames':[1,241],'gravity_blender':[3,0,0]},
                             {'frames':[242,481],'gravity_blender':[-3,-3,0]}],
        'scope':'Diagnostic lateral gravity, no avatar or motors. Blender Bullet fixture only; not normal runtime gravity or Bevy/Rapier acceptance.',
        'samples':samples,'results':{}}
    for name in fixtures:
        angles=[r['doors'][name]['angle_deg'] for r in samples]
        anchor=max(r['doors'][name]['anchor_error_m'] for r in samples)
        report['results'][name]={'minimum_angle_deg':min(angles),'maximum_angle_deg':max(angles),
            'open_angle_deg':angles[240],'final_angle_deg':angles[-1],'maximum_anchor_error_m':anchor,
            'pass':abs(angles[240]-90)<1 and abs(angles[-1])<1 and min(angles)>-1.5 and max(angles)<91.5 and anchor<.005}
    (out/'physics-cycle.json').write_text(json.dumps(report,indent=2)+'\n')
    print('HINGE_FIXTURE',report['results'],flush=True)
    assert all(r['pass'] for r in report['results'].values()),'Hinge mechanics fixture failed; evidence retained'


if __name__=='__main__':
    check(Path(sys.argv[sys.argv.index('--')+1]))
    os._exit(0)
