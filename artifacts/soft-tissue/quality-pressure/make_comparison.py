"""Compare reviewed and candidate renders with identical timing and tracked crops."""
from pathlib import Path
import hashlib
import json
import subprocess

HERE = Path(__file__).resolve().parent
baseline = HERE.parent/'lab-boundary-nodal-front-01'
candidate = HERE.parent/'lab-quality-front-01'
old = [json.loads(line) for line in (baseline/'frames.ndjson').read_text().splitlines()]
new = [json.loads(line) for line in (candidate/'frames.ndjson').read_text().splitlines()]
crops = json.loads((HERE.parent/'volume-audit/boundary-tracked-crop.json').read_text())
assert [row['pts_ms'] for row in old] == [row['pts_ms'] for row in new] == [row['pts_ms'] for row in crops]
assert (baseline/'packets.ndjson').read_bytes() == (candidate/'packets.ndjson').read_bytes()
commands = []
for row in crops:
    time = (row['pts_ms']-old[0]['pts_ms'])/1000.
    x, y = row['x'], row['y']
    commands.append(f'{time:.3f} crop@left x {x}, crop@left y {y}, crop@right x {x}, crop@right y {y};')
with (HERE/'comparison-crops.cmd').open('x') as file:
    file.write('\n'.join(commands)+'\n')
for label, folder, rows in [('baseline', baseline, old), ('candidate', candidate, new)]:
    entries = ['ffconcat version 1.0']
    for i, row in enumerate(rows):
        dt = rows[i+1]['pts_ms']-row['pts_ms'] if i+1 < len(rows) else 17
        entries.extend([f"file '{(folder/row['image']).resolve()}'", 'option framerate 1000', f'duration {dt/1000:.3f}'])
    with (HERE/f'{label}.ffconcat').open('x') as file:
        file.write('\n'.join(entries)+'\n')
filters = [f'[0:v]sendcmd=f={HERE/"comparison-crops.cmd"},crop@left=320:320,scale=640:640:flags=lanczos,pad=640:690:0:50:color=0x111923,drawtext=text=Reviewed baseline:fontcolor=white:fontsize=24:x=20:y=12[left]',
           '[1:v]crop@right=320:320,scale=640:640:flags=lanczos,pad=640:690:0:50:color=0x111923,drawtext=text=Localized volume treatment:fontcolor=white:fontsize=24:x=20:y=12[right]',
           '[left][right]hstack=inputs=2[out]']
command = ['/usr/bin/ffmpeg', '-nostdin', '-v', 'error', '-n']
for label in ['baseline', 'candidate']:
    command += ['-f', 'concat', '-safe', '0', '-i', str(HERE/f'{label}.ffconcat')]
video = HERE/'reviewed-vs-local-volume.mp4'
command += ['-filter_complex', ';'.join(filters), '-map', '[out]', '-c:v', 'libx264', '-crf', '18', '-preset', 'fast', '-pix_fmt', 'yuv420p', '-fps_mode', 'vfr', '-enc_time_base', '1:1000', '-video_track_timescale', '1000', '-bf', '0', '-movflags', '+faststart', str(video)]
subprocess.run(command, check=True)
probe = json.loads(subprocess.check_output(['/usr/bin/ffprobe', '-v', 'error', '-select_streams', 'v:0', '-show_frames', '-show_entries', 'frame=best_effort_timestamp_time', '-of', 'json', str(video)]))
assert [round(float(row['best_effort_timestamp_time'])*1000) for row in probe['frames']] == [row['pts_ms']-old[0]['pts_ms'] for row in old]
(HERE/'video-validation.json').write_text(json.dumps({'frames': len(old), 'every_source_timestamp_matches': True,
    'source_packets_identical': True, 'identical_crop_both_views': True, 'spatial_scale': 2, 'physics_amplitude_multiplier': 1,
    'video_sha256': hashlib.sha256(video.read_bytes()).hexdigest()}, indent=2)+'\n')
print(video)
