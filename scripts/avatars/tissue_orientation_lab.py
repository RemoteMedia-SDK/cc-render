"""Gravity/inertia lab: turn one frozen standing pose as a rigid body (face-down,
upside-down, side-lying, turns, shakes) and record the breast tissue response.

Lab only. The packets are scripted rigid-body motion, marked by
synthetic-rig.json beside them; they are a physics test, never ARDY output, and
never feed the office runtime.

    python scripts/avatars/tissue_orientation_lab.py list
    python scripts/avatars/tissue_orientation_lab.py run all --label orientation-01
    python scripts/avatars/tissue_orientation_lab.py run face-down shake-side --label probe-01
"""
from pathlib import Path
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import math
import os
import shutil
import subprocess
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
LAB = ROOT/'artifacts/soft-tissue/orientation-lab'
POSE = LAB/'standing-pose.json'
BINARY = ROOT/'target/debug/examples/tissue_lab'
GLB = ROOT.parent/'remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb'
ARKIT = ROOT.parent/'remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json'
ASSET = ROOT/'assets/tissue/beautiful_assistant.base-calibrated-s40-k32-v4-prototype.json'
FPS = 60
# Packet-space pivot relative to the pelvis: between the breasts, so the chest
# views keep them framed through any rotation.
PIVOT_FROM_ROOT = np.array([0.0, 0.27, 0.08])
AXES = {'x': (1, 0, 0), 'y': (0, 1, 0), 'z': (0, 0, 1)}

SETTLE = ('hold', 1.5)


def lie(axis, degrees, over=1.5):
    return [SETTLE, ('rotate', over, axis, degrees), ('hold', 3.0), ('rotate', over, axis, -degrees), ('hold', 1.5)]


# World axes: +x is the avatar's left, +y up, +z the way she faces.
SCENARIOS = {
    'face-down': lie('x', 90),
    'face-up': lie('x', -90),
    'upside-down': lie('x', 180, over=2.5),
    'side-left': lie('z', -90),
    'side-right': lie('z', 90),
    'turn': [SETTLE, ('rotate', 0.5, 'y', 90), ('hold', 1.0), ('rotate', 1.0, 'y', -180), ('hold', 1.0),
             ('rotate', 0.5, 'y', 90), ('hold', 1.5)],
    'tumble': [SETTLE, ('rotate', 4.0, 'x', 720), ('hold', 2.0)],
    'shake-side': [SETTLE, ('shake', 3.0, 'x', 0.03, 3.0), ('hold', 2.0)],
    'shake-bounce': [SETTLE, ('shake', 3.0, 'y', 0.03, 3.0), ('hold', 2.0)],
    'shake-twist': [SETTLE, ('shake', 3.0, 'yaw', 20.0, 2.0), ('hold', 2.0)],
}


def quat(axis, degrees):
    half = math.radians(degrees) / 2
    return np.array([*(math.sin(half) * np.array(AXES[axis], float)), math.cos(half)])


def multiply(a, b):
    (ax, ay, az, aw), (bx, by, bz, bw) = a, b
    return np.array([aw*bx + ax*bw + ay*bz - az*by, aw*by - ax*bz + ay*bw + az*bx,
                     aw*bz + ax*by - ay*bx + az*bw, aw*bw - ax*bx - ay*by - az*bz])


def rotate(q, v):
    u, w = q[:3], q[3]
    return v + 2 * np.cross(u, np.cross(u, v) + w * v)


def ease(t):
    """Minimum-jerk 0..1: zero velocity and acceleration at both ends."""
    return t**3 * (10 - 15*t + 6*t**2)


def envelope(t, duration, ramp=0.5):
    edge = min(t, duration - t, ramp) / ramp
    return 0.5 - 0.5 * math.cos(math.pi * max(edge, 0.0))


def body_motion(segments):
    """(seconds, rotation xyzw, translation m) at FPS, rotations about the pivot."""
    samples, orientation, now = [], np.array([0., 0., 0., 1.]), 0.0
    for segment in segments:
        kind, duration = segment[0], segment[1]
        steps = round(duration * FPS)
        for i in range(1 if samples else 0, steps + 1):
            t = i / FPS
            rotation, offset = orientation, np.zeros(3)
            if kind == 'rotate':
                rotation = multiply(quat(segment[2], segment[3] * ease(t / duration)), orientation)
            elif kind == 'shake':
                _, _, channel, amplitude, hz = segment
                wave = amplitude * envelope(t, duration) * math.sin(2 * math.pi * hz * t)
                if channel in AXES:
                    offset = wave * np.array(AXES[channel], float)
                else:
                    rotation = multiply(quat({'yaw': 'y', 'pitch': 'x', 'roll': 'z'}[channel], wave), orientation)
            samples.append((now + t, rotation, offset))
        if kind == 'rotate':
            orientation = multiply(quat(segment[2], segment[3]), orientation)
        now += steps / FPS
    return samples


def packets(name, pose):
    root = np.array(pose['root_pos'])
    pivot = root + PIVOT_FROM_ROOT
    for i, (_, rotation, offset) in enumerate(body_motion(SCENARIOS[name])):
        joints = [list(q) for q in pose['joint_quats_xyzw']]
        joints[0] = list(multiply(rotation, np.array(joints[0])))
        yield dict(kind='skeletal_pose', session_id=f'synthetic-orientation-{name}', generation_revision=1,
                   skeleton_id='smpl22', root_mode='world', fps=FPS, pts_ms=50 + round(i * 1000 / FPS),
                   root_pos=list(pivot + rotate(rotation, root - pivot) + offset), joint_quats_xyzw=joints)


def write_source(name, folder):
    folder.mkdir(parents=True)
    pose = json.loads(POSE.read_text())
    with (folder/'packets.ndjson').open('x') as out:
        for packet in packets(name, pose):
            out.write(json.dumps(packet, allow_nan=False) + '\n')
    (folder/'synthetic-rig.json').write_text(json.dumps(dict(
        policy='Synthetic rigid-body test rig for the tissue lab; not ARDY motion, never used at runtime.',
        scenario=name, segments=SCENARIOS[name], fps=FPS, pivot_from_root_m=PIVOT_FROM_ROOT.tolist(),
        pose=str(POSE.relative_to(ROOT)), pose_sha256=hashlib.sha256(POSE.read_bytes()).hexdigest(),
        script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()), indent=2) + '\n')
    return folder/'packets.ndjson'


def capture(source, view, out, asset):
    environment = dict(CC_RENDER_FAST='0', CC_AVATAR_TISSUE_HZ='240', CC_AVATAR_TISSUE_ITERATIONS='32',
                       CC_TISSUE_LAB_FPS='60', CC_TISSUE_LAB_CLOCK='fixed', CC_TISSUE_LAB_TRACE_ONLY='0',
                       CC_AVATAR_TISSUE_TRACE_NODES='0', RUST_LOG='info')
    for key in ['CC_AVATAR_TISSUE_SURFACE_SMOOTHING', 'CC_AVATAR_TISSUE_SURFACE_EMBEDDING']:
        if key in os.environ:
            environment[key] = os.environ[key]
    command = [str(BINARY), str(GLB), str(ARKIT), str(source), str(asset), view, str(out)]
    with out.with_suffix('.log').open('x') as log:
        result = subprocess.run(command, cwd=ROOT, env={**os.environ, **environment},
                                stdout=log, stderr=subprocess.STDOUT)
    return dict(argv=command, environment=environment, exit_code=result.returncode)


def response(trace, segments):
    rows = [json.loads(line) for line in trace.read_text().splitlines()]
    ends, now = [], 0.0
    for segment in segments:
        now += round(segment[1] * FPS) / FPS
        ends.append((segment[0], round(50 + now * 1000)))
    regions = {}
    for k, region in enumerate(rows[0]['regions'] if rows else []):
        offsets = [np.array(r['regions'][k]['free_node_centroid_offset_m']) * 1e3 for r in rows]
        at = lambda pts: offsets[min(range(len(rows)), key=lambda i: abs(rows[i]['pts_ms'] - pts))]
        regions[region['name']] = dict(
            peak_centroid_offset_mm=round(max(np.linalg.norm(o) for o in offsets), 2),
            peak_node_offset_mm=round(max(r['regions'][k]['maximum_node_offset_m'] for r in rows) * 1e3, 2),
            minimum_volume_ratio=round(min(r['regions'][k]['metrics']['minimum_volume_ratio'] for r in rows), 3),
            maximum_volume_ratio=round(max(r['regions'][k]['metrics']['maximum_volume_ratio'] for r in rows), 3),
            maximum_speed_m_s=round(max(r['regions'][k]['metrics']['maximum_speed_m_s'] for r in rows), 3),
            # Body frame: +x her left, +y up, +z forward; zero is the standing rest.
            segment_end_offsets_mm=[dict(segment=kind, pts_ms=pts, offset=[round(v, 1) for v in at(pts)])
                                    for kind, pts in ends])
    creases = [(r.get('maximum_crease_deg', 0.), r['pts_ms']) for r in rows]
    return rows, dict(frames=len(rows), last_pts_ms=rows[-1]['pts_ms'] if rows else None, regions=regions,
                      peak_crease_deg=round(max(creases)[0], 1) if creases else None,
                      peak_crease_pts_ms=max(creases)[1] if creases else None)


def plot(rows, segments, title, output):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    seconds = [(r['pts_ms'] - 50) / 1000 for r in rows]
    figure, axes = plt.subplots(len(rows[0]['regions']), 1, figsize=(10, 6), sharex=True, squeeze=False)
    for k, axis in enumerate(axes[:, 0]):
        offsets = np.array([r['regions'][k]['free_node_centroid_offset_m'] for r in rows]) * 1e3
        for c, label in enumerate(['x (her left)', 'y (up)', 'z (forward)']):
            axis.plot(seconds, offsets[:, c], label=label)
        now = 0.0
        for segment in segments:
            duration = round(segment[1] * FPS) / FPS
            if segment[0] != 'hold':
                axis.axvspan(now, now + duration, color='0.9')
            now += duration
        axis.set_ylabel(f"{rows[0]['regions'][k]['name']}\ncentroid mm")
        axis.grid(alpha=0.3)
    axes[0, 0].legend(loc='upper right', fontsize=8)
    axes[0, 0].set_title(f'{title}: free-node centroid in the body frame (shaded = moving)')
    axes[-1, 0].set_xlabel('seconds')
    figure.tight_layout()
    figure.savefig(output, dpi=110)
    plt.close(figure)


def side_by_side(name, views, folder):
    from review_tissue_lab import FFMPEG
    command = [FFMPEG, '-v', 'error', '-n']
    for view in views:
        command += ['-i', str(folder/f'{view}.mp4')]
    filters = [f"[{i}:v]drawtext=fontcolor=white:fontsize=22:x=14:y=12:text='{name} | {view}'[v{i}]"
               for i, view in enumerate(views)]
    filters.append(''.join(f'[v{i}]' for i in range(len(views))) + f'hstack=inputs={len(views)}[v]'
                   if len(views) > 1 else '[v0]null[v]')
    command += ['-filter_complex', ';'.join(filters), '-map', '[v]', '-c:v', 'libx264', '-crf', '18',
                '-pix_fmt', 'yuv420p', '-movflags', '+faststart', str(folder/f'{name}.mp4')]
    subprocess.run(command, check=True)


def scenario(name, folder, views, keep_frames, asset):
    from review_tissue_lab import encode
    source = write_source(name, folder/'source')
    result = dict(captures={})
    for view in views:
        capture_dir = folder/view
        launch = capture(source, view, capture_dir, asset)
        result['captures'][view] = launch
        if launch['exit_code'] != 0:
            continue
        encode(capture_dir, folder/f'{view}.mp4')
        if not keep_frames:
            for png in capture_dir.glob('*.png'):
                png.unlink()
    completed = [v for v in views if result['captures'][v]['exit_code'] == 0]
    if completed:
        rows, result['response'] = response(folder/completed[0]/'tissue.ndjson', SCENARIOS[name])
        plot(rows, SCENARIOS[name], name, folder/'response.png')
        side_by_side(name, completed, folder)
    (folder/'result.json').write_text(json.dumps(result, indent=2) + '\n')
    return dict(exit_codes={v: result['captures'][v]['exit_code'] for v in views},
                **{k: result.get('response', {}).get(k) for k in ('frames', 'last_pts_ms', 'peak_crease_deg')},
                regions={r: {k: v for k, v in d.items() if k != 'segment_end_offsets_mm'}
                         for r, d in result.get('response', {}).get('regions', {}).items()})


def run(names, label, views, keep_frames, jobs, asset):
    """Scenarios run `jobs` at a time. An existing label gains only new scenarios."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    out = LAB/label
    out.mkdir(parents=True, exist_ok=True)
    existing = sorted(n for n in names if (out/n).exists())
    if existing:
        raise SystemExit(f'{label} already has {existing}; choose other scenarios or a new label')
    summary_path = out/'summary.json'
    summary = json.loads(summary_path.read_text()) if summary_path.exists() else {}
    with ThreadPoolExecutor(jobs) as pool:
        futures = {name: pool.submit(scenario, name, out/name, views, keep_frames, asset) for name in names}
        for name, future in futures.items():
            summary[name] = future.result()
            summary_path.write_text(json.dumps(summary, indent=2) + '\n')
            print(name, json.dumps(summary[name]['exit_codes']), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('list')
    runner = commands.add_parser('run')
    runner.add_argument('scenarios', nargs='+', help="scenario names or 'all'")
    runner.add_argument('--label', required=True)
    runner.add_argument('--views', nargs='+', default=['chest-front', 'chest-side'],
                        choices=['front', 'side', 'chest-front', 'chest-side'])
    runner.add_argument('--keep-frames', action='store_true', help='Keep the PNG frames after encoding.')
    runner.add_argument('--jobs', type=int, default=4, help='Scenarios rendered at once.')
    runner.add_argument('--asset', type=Path, default=ASSET, help='Tissue asset (default: the selected one).')
    args = parser.parse_args()
    if args.command == 'list':
        for name, segments in SCENARIOS.items():
            print(f"{name:13} {sum(round(s[1] * FPS) / FPS for s in segments):5.1f} s  {segments[1:]}")
        return
    names = list(SCENARIOS) if args.scenarios == ['all'] else args.scenarios
    unknown = sorted(set(names) - set(SCENARIOS))
    if unknown or not args.label.replace('-', '').isalnum():
        parser.error(f'unknown scenarios {unknown} or invalid label')
    run(names, args.label, args.views, args.keep_frames, max(1, args.jobs), args.asset.resolve())


if __name__ == '__main__':
    main()
