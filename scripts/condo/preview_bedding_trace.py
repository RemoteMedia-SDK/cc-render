"""Replay the live ECS bedding trace on the authored bed in Blender.

No Blender cloth solve, smoothing modifier or geometry repair. Visible cloth is
the solver midsurface; collision thickness is reported but not extruded here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

import bpy

sys.path.insert(0,str(Path(__file__).resolve().parent))
from check_bed_sheet import load_mattress, studio, SOURCE
from prepare_cloth_package import SCENE
from preview_cloth_trace import apply_sample, mattress_clearance, animation_rate
from preview_drawer_trace import configure_animation, encode_animation
from tabletop_props import material


def load_bed():
    mattress = load_mattress()
    prefixes = ('Bedroom detail side rail','Bedroom detail end rail',
                'Bedroom detail recessed foot','Bedroom detail headboard')
    with bpy.data.libraries.load(str(SOURCE),link=False) as (available,requested):
        requested.objects = [n for n in available.objects if n.startswith(prefixes)]
    for obj in requested.objects:
        bpy.context.collection.objects.link(obj)
        if obj.name.startswith('Bedroom detail headboard'):
            obj.location.y += .055
    return mattress


def cloth_mesh(definition):
    asset = json.loads((SCENE/definition['surface']).read_text())
    mesh = bpy.data.meshes.new(definition['id'])
    mesh.from_pydata([(x,-z,y) for x,y,z in asset['initial_positions']],[],asset['triangles'])
    mesh.update()
    uv = mesh.uv_layers.new(name='MaterialUV')
    for loop in mesh.loops:
        uv.data[loop.index].uv = asset['material_uv'][loop.vertex_index]
    for face in mesh.polygons:
        face.use_smooth = True
    obj = bpy.data.objects.new(definition['id'],mesh)
    bpy.context.collection.objects.link(obj)
    linear = [c/12.92 if c<=.04045 else ((c+.055)/1.055)**2.4 for c in definition['color_srgb']]
    mesh.materials.append(material(definition['id']+' material',linear,.92))
    return obj


def preview(trace_path, output, diagnostic_prefix=False):
    package_path = SCENE/'scene-bedding.json'
    package = json.loads(package_path.read_text())
    frames = [json.loads(line) for line in trace_path.read_text().splitlines()]
    assert [f['frame'] for f in frames] == list(range(len(frames)))
    if not diagnostic_prefix:
        assert frames[-1]['frame'] == 600, 'Complete ten-second trace required; use --diagnostic-prefix for incomplete progress.'
    expected = {c['id']:c['sha256'] for c in package['cloths']}
    assert {c['id']:c['sha256'] for c in frames[0]['assets']} == expected
    for frame in frames:
        for state in frame['observations']:
            assert state['error'] is None and state['simulated_ticks']==frame['frame'], state
            assert state['mesh_position_sha256']==state['surface_position_sha256'], state
    selected = frames[::6]
    fps = animation_rate(selected)
    output.mkdir(parents=True,exist_ok=False)
    mattress = load_bed()
    cloths = {c['id']:cloth_mesh(c) for c in package['cloths']}
    scene = bpy.context.scene
    studio(scene)
    configure_animation(scene,output)
    shots = []
    for ordinal,frame in enumerate(selected):
        audits = {}
        for layer in frame['layers']:
            cloth = cloths[layer['id']]
            apply_sample(cloth,layer)
            expected_points = [(x,-z,y) for x,y,z in layer['positions']]
            error = max(abs(a-b) for v,p in zip(cloth.data.vertices,expected_points) for a,b in zip(v.co,p))
            assert error==0, error
            audits[layer['id']] = {'mesh_error_m':error,**mattress_clearance(mattress,cloth)}
        filename = f'frames/{ordinal:04d}.png'
        scene.render.filepath = str(output/filename)
        bpy.ops.render.render(write_still=True)
        shots.append({'file':filename,'simulation_seconds':frame['frame']/60,'layers':audits})
    encode_animation(output,shots,fps)
    sources = [trace_path,package_path,SOURCE,Path(__file__),SCENE/'interior-bedding-clearance.glb',
               *[SCENE/c['surface'] for c in package['cloths']],
               *[Path(__file__).with_name(n) for n in ('check_bed_sheet.py','preview_cloth_trace.py',
                  'preview_drawer_trace.py','tabletop_props.py','prepare_cloth_package.py')]]
    report = {'scope':__doc__,'screenshots':shots,'fps':fps,
              'diagnostic_prefix':diagnostic_prefix,
              'last_recorded_tick':frames[-1]['frame'],
              'acceptance_claim':'Incomplete progress only' if diagnostic_prefix else 'Replay only; consult the associated test result',
              'inputs':[{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in sources],
              'final_observations':frames[-1]['observations'],
              'limits':['Fitted shell is a sewn-shape approximation.',
                        'Existing headboard lightmaps are not rebaked after its 55 mm clearance shift.',
                        'Authored-mattress audit uses finite triangle samples, not a continuous guarantee.']}
    (output/'capture.json').write_text(json.dumps(report,indent=2)+'\n')
    print('BEDDING_PREVIEW_COMPLETE',output,flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace',type=Path)
    parser.add_argument('output',type=Path)
    parser.add_argument('--diagnostic-prefix',action='store_true',help='Render incomplete progress without claiming bedding acceptance.')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    preview(args.trace.resolve(),args.output.resolve(),args.diagnostic_prefix)
    os._exit(0)
