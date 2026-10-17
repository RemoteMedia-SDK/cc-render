"""Render observed Rapier drawer poses; no avatar or runtime furniture actuator.

Blender --background --python-exit-code 1 --python this_file -- TRACE NEW_OUTPUT
"""
import hashlib
import json
import os
from pathlib import Path
import sys

import bpy
from mathutils import Quaternion, Vector

sys.path.insert(0, str(Path(__file__).parent))
from tabletop_props import box, material

ROOT = Path(__file__).resolve().parents[2]
ASSET = ROOT / 'scenes/toronto_condo/interaction-authoring/bedside/bedside-mechanism.blend'


def blender_point(point):
    return Vector((point[0], -point[2], point[1]))


def preview(trace_path, output):
    output.mkdir(parents=True, exist_ok=False)
    trace = json.loads(trace_path.read_text())
    bpy.ops.wm.open_mainfile(filepath=str(ASSET))
    scene = bpy.context.scene
    block = box('Observed diagnostic obstruction', (0, 0, 0), (.7, .05, .23),
                material('Diagnostic obstruction', (.4, .12, .025), .7), scene.collection)
    placement = Vector(trace['placement'])
    shots = []
    for index in (119, 359, 479, 839):
        sample = trace['frames'][index]
        for state in sample['parts']:
            part = bpy.data.objects[state['id']]
            part.location = blender_point(Vector(state['position'])-placement)
            x, y, z, w = state['rotation_xyzw']
            part.rotation_mode = 'QUATERNION'
            part.rotation_quaternion = Quaternion((w, x, -z, y))
        obstruction = sample.get('obstruction')
        block.hide_render = obstruction is None
        if obstruction:
            block.location = blender_point(Vector(obstruction['position'])-placement)
            x, y, z = obstruction['half_extents']
            block.dimensions = (2*x, 2*z, 2*y)
        bpy.context.view_layer.update()
        name = f'frame-{index:04d}.png'
        scene.render.filepath = str(output / name)
        bpy.ops.render.render(write_still=True)
        shots.append({'file':name, 'frame':index, 'phase':sample['phase'], 'observed':sample})
    report = {'scope':__doc__, 'screenshots':shots,
              'inputs':[{'path':str(p), 'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
                        for p in (trace_path, ASSET, Path(__file__))],
              'limits':['Diagnostic test forces only; no generated avatar manipulation.',
                        'Rendering recorded rigid poses does not certify hand contact.']}
    (output / 'capture.json').write_text(json.dumps(report, indent=2)+'\n')
    print('DRAWER_TRACE_RENDERED', len(shots), flush=True)


if __name__ == '__main__':
    args = sys.argv[sys.argv.index('--')+1:]
    preview(Path(args[0]).resolve(), Path(args[1]).resolve())
    os._exit(0)
