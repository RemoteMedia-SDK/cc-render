"""Render observed Rapier mechanism poses; no avatar or runtime furniture actuator.

Blender --background --python-exit-code 1 --python this_file -- TRACE NEW_OUTPUT
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import bpy
from mathutils import Quaternion, Vector

sys.path.insert(0, str(Path(__file__).parent))
from tabletop_props import box, material

ROOT = Path(__file__).resolve().parents[2]
ASSET = ROOT / 'scenes/toronto_condo/interaction-authoring/bedside/bedside-mechanism.blend'


def blender_point(point):
    return Vector((point[0], -point[2], point[1]))


def configure_animation(scene, output):
    (output / 'frames').mkdir()
    scene.render.resolution_percentage = 75
    scene.cycles.samples = 16
    preferences = bpy.context.preferences.addons['cycles'].preferences
    preferences.compute_device_type = 'CUDA'
    preferences.get_devices()
    for device in preferences.devices:
        device.use = device.type == 'CUDA' and '03:00' in device.id
    if not any(device.use for device in preferences.devices):
        raise RuntimeError('Animation render requires the second CUDA GPU')
    scene.cycles.device = 'GPU'
    scene.render.use_persistent_data = True


def encode_animation(output, shots, fps=30):
    for ordinal in (0, len(shots)//2, len(shots)-1):
        shutil.copy2(output / shots[ordinal]['file'], output / f'preview-{ordinal:04d}.png')
    subprocess.run([
        '/usr/bin/ffmpeg', '-v', 'error', '-n', '-framerate', str(fps),
        '-i', str(output / 'frames/%04d.png'), '-vf',
        "drawtext=text='Recorded physics demonstration - no avatar manipulation':"
        'fontcolor=white:fontsize=20:box=1:boxcolor=black@0.7:x=16:y=h-40',
        '-c:v', 'libx264', '-crf', '20', '-pix_fmt', 'yuv420p',
        '-movflags', '+faststart', str(output / 'view.mp4')], check=True)


def preview(trace_path, output, asset=ASSET, frames=(119, 359, 479, 839), animation=False, fps=30):
    assert fps in (10, 15, 30), 'Frame rate must divide the 60 Hz physics trace'
    output.mkdir(parents=True, exist_ok=False)
    trace = json.loads(trace_path.read_text())
    inputs = [{'path':str(p), 'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
              for p in (trace_path, asset, Path(__file__))]
    bpy.ops.wm.open_mainfile(filepath=str(asset))
    scene = bpy.context.scene
    if animation:
        # Diagnostic traces use 60 Hz. Preserve timing without interpolation.
        frames = range(0, len(trace['frames']), 60//fps)
        configure_animation(scene, output)
    block = box('Observed diagnostic obstruction', (0, 0, 0), (.7, .05, .23),
                material('Diagnostic obstruction', (.4, .12, .025), .7), scene.collection)
    placement = Vector(trace['placement'])
    shots = []
    for ordinal, index in enumerate(frames):
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
        name = f'frames/{ordinal:04d}.png' if animation else f'frame-{index:04d}.png'
        scene.render.filepath = str(output / name)
        bpy.ops.render.render(write_still=True)
        shots.append({'file':name, 'frame':index, 'phase':sample['phase'], 'observed':sample})
    if animation:
        encode_animation(output, shots, fps)
    report = {'scope':__doc__, 'screenshots':shots,
              'inputs':inputs, 'fps':fps if animation else None,
              'limits':['Diagnostic test forces only; no generated avatar manipulation.',
                        'Rendering recorded rigid poses does not certify hand contact.']}
    (output / 'capture.json').write_text(json.dumps(report, indent=2)+'\n')
    print('MECHANISM_TRACE_RENDERED', len(shots), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--asset', type=Path, default=ASSET)
    parser.add_argument('--frames', type=int, nargs='+', default=(119, 359, 479, 839))
    parser.add_argument('--animation', action='store_true', help='Render the full 60 Hz trace as a video')
    parser.add_argument('--fps', type=int, choices=(10, 15, 30), default=30,
                        help='Animation frame rate; preserves physical time without interpolation')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    preview(args.trace.resolve(), args.output.resolve(), args.asset.resolve(), args.frames, args.animation, args.fps)
    os._exit(0)
