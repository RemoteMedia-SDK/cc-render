"""Use the same torso-tracked crop on unchanged physics-off/on frame sequences."""
import hashlib
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
from scipy.spatial.transform import Rotation

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / 'scripts/avatars'))
from build_tissue_asset import anchor_space, surfaces

source = ROOT / '../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb'
body = next(s for s in surfaces(source) if s['name'] == 'CC_Base_Body')
patch = np.logical_or.reduce([w > .001 for w in body['weights']])
center = anchor_space(body['positions'][patch].mean(axis=0)[None], body['anchor_inverse'])[0]
folder = HERE.parent / 'lab-soft-nodal-isochoric-front-01'
frames = [json.loads(l) for l in (folder / 'frames.ndjson').read_text().splitlines()]
trace = [json.loads(l) for l in (folder / 'tissue.ndjson').read_text().splitlines()]
off = HERE.parent / 'lab-front-off-01'
off_frames = [json.loads(l) for l in (off / 'frames.ndjson').read_text().splitlines()]
assert [r['pts_ms'] for r in frames] == [r['pts_ms'] for r in off_frames]
assert (folder / 'packets.ndjson').read_bytes() == (off / 'packets.ndjson').read_bytes()
camera = np.array([0., 1.55, 2.6]); target = np.array([0., 1.5, 0.])
forward = target - camera; forward /= np.linalg.norm(forward)
right = np.cross(forward, [0., 1., 0.]); right /= np.linalg.norm(right)
up = np.cross(right, forward)
focal = 450 / np.tan(np.deg2rad(38 / 2))
commands, crops = [], []
for frame, row in zip(frames, trace, strict=True):
    assert frame['pts_ms'] == row['pts_ms']
    region = row['regions'][0]
    point = Rotation.from_quat(region['anchor_rotation_xyzw']).apply(center) + region['anchor_world_m'] - camera
    pixel = [360 + focal * np.dot(point, right) / np.dot(point, forward),
             450 - focal * np.dot(point, up) / np.dot(point, forward)]
    x, y = int(np.clip(round(pixel[0]) - 160, 0, 400)), int(np.clip(round(pixel[1]) - 160, 0, 580))
    time = (frame['pts_ms'] - frames[0]['pts_ms']) / 1000
    commands.append(f'{time:.3f} crop@left x {x}, crop@left y {y}, crop@right x {x}, crop@right y {y};')
    crops.append({'pts_ms': frame['pts_ms'], 'x': x, 'y': y, 'width': 320, 'height': 320})
(HERE / 'soft-isochoric-tracked-crop.cmd').write_text('\n'.join(commands) + '\n')
(HERE / 'soft-isochoric-tracked-crop.json').write_text(json.dumps(crops) + '\n')
for label, capture, rows in [('off', off, off_frames), ('on', folder, frames)]:
    entries = ['ffconcat version 1.0']
    for i, row in enumerate(rows):
        dt = rows[i+1]['pts_ms'] - row['pts_ms'] if i+1 < len(rows) else 17
        entries.extend([f"file '{(capture / row['image']).resolve()}'", 'option framerate 1000', f'duration {dt / 1000:.3f}'])
    (HERE / f'soft-isochoric-comparison-{label}.ffconcat').write_text('\n'.join(entries) + '\n')
filters = [f'[0:v]sendcmd=f={HERE / "soft-isochoric-tracked-crop.cmd"},crop@left=320:320,scale=640:640:flags=lanczos,pad=640:690:0:50:color=0x111923,drawtext=text=Physics off:fontcolor=white:fontsize=24:x=20:y=12[off]',
           '[1:v]crop@right=320:320,scale=640:640:flags=lanczos,pad=640:690:0:50:color=0x111923,drawtext=text=Material sensitivity - not accepted:fontcolor=white:fontsize=24:x=20:y=12[on]',
           '[off][on]hstack=inputs=2[out]']
output = HERE / 'tracked-off-on-soft-isochoric.mp4'
command = ['/usr/bin/ffmpeg', '-nostdin', '-v', 'error', '-n']
for label in ['off', 'on']:
    command += ['-f', 'concat', '-safe', '0', '-i', str(HERE / f'soft-isochoric-comparison-{label}.ffconcat')]
command += ['-filter_complex', ';'.join(filters), '-map', '[out]', '-c:v', 'libx264', '-crf', '18', '-preset', 'fast', '-pix_fmt', 'yuv420p', '-fps_mode', 'vfr', '-enc_time_base', '1:1000', '-video_track_timescale', '1000', '-bf', '0', '-movflags', '+faststart', str(output)]
subprocess.run(command, check=True)
probe = json.loads(subprocess.check_output(['/usr/bin/ffprobe','-v','error','-select_streams','v:0','-show_frames','-show_entries','frame=best_effort_timestamp_time','-of','json',str(output)]))
assert [round(float(f['best_effort_timestamp_time']) * 1000) for f in probe['frames']] == [f['pts_ms'] - frames[0]['pts_ms'] for f in frames]
(HERE / 'soft-isochoric-video-validation.json').write_text(json.dumps({'frames':len(frames),'every_source_timestamp_matches':True,'source_packets_identical':True,'identical_crop_both_views':True,'spatial_scale':2,'physics_amplitude_multiplier':1,'video_sha256':hashlib.sha256(output.read_bytes()).hexdigest()},indent=2)+'\n')
print(output)
