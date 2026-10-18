"""Reproduce a paired sensitivity report; optional argument: finer rate 240 or 480."""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts/avatars'))
from review_tissue_lab import encode, lines, np, plt
from retime_tissue_capture import probe_timestamps

ART = ROOT / 'artifacts/soft-tissue'
FAMILY = sys.argv[2] if len(sys.argv) > 2 else 'morph'
assert FAMILY in ('morph', 'bdf2', 'viscous')
OUT = Path(__file__).resolve().parent if FAMILY == 'morph' else ART / 'temporal-integration'
FINE = int(sys.argv[1]) if len(sys.argv) > 1 else 240
assert FINE in (240, 480)
RATES = [FINE // 2, FINE]
PREFIX = 'timestep' if FINE == 240 else 'timestep-240-480'
if FAMILY != 'morph': PREFIX = f'{FAMILY}-{RATES[0]}-{RATES[1]}'
CAPTURES = [ART / ('lab-front-hardening-8' if rate == 120 and FAMILY == 'morph' else f'lab-front-{FAMILY}-{rate}hz-01') for rate in RATES]
traces = [lines(folder / 'tissue.ndjson') for folder in CAPTURES]
frames = [lines(folder / 'frames.ndjson') for folder in CAPTURES]
configs = [json.loads((folder / 'config.json').read_text()) for folder in CAPTURES]
for folder in CAPTURES:
    completion = json.loads((folder / 'capture.json').read_text())
    assert completion['sent_packets'] == 160 and completion['last_pts_ms'] == 8000
assert CAPTURES[0].joinpath('packets.ndjson').read_bytes() == CAPTURES[1].joinpath('packets.ndjson').read_bytes()
for key in ['inputs', 'camera', 'resolution', 'fps', 'tissue_iterations']:
    assert configs[0][key] == configs[1][key], key
assert [int(config['tissue_hz']) for config in configs] == RATES
assets = [json.loads((folder / 'tissue-asset.json').read_text()) for folder in CAPTURES]
assert all(assets[0][key] == assets[1][key] for key in assets[0] if key != 'contact_inactive_morph_targets')
stamps = [[row['pts_ms'] for row in trace] for trace in traces]
assert stamps[0] == stamps[1] == [row['pts_ms'] for row in frames[0]] == [row['pts_ms'] for row in frames[1]]
times = np.asarray(stamps[0]) / 1000
envelopes = [np.asarray([row['maximum_surface_displacement_m'] for row in trace]) * 1000 for trace in traces]
centroids = [np.asarray([[region['free_node_centroid_offset_m'] for region in row['regions']] for row in trace]) * 1000 for trace in traces]
report = {
    'rates_hz': RATES, 'source_frames': len(times), 'integration': 'implicit_euler' if FAMILY == 'morph' else 'bdf2', 'damping': 'continuum' if FAMILY == 'viscous' else 'edge',
    'maximum_surface_envelope_difference_mm': float(np.max(np.abs(envelopes[0] - envelopes[1]))),
    'maximum_centroid_vector_difference_mm': float(np.max(np.linalg.norm(centroids[0] - centroids[1], axis=2))),
    'first_landing_peak_surface_displacement_mm': [float(e[(times >= 1.3) & (times <= 1.9)].max()) for e in envelopes],
    'whole_sequence_peak_surface_displacement_mm': [float(e.max()) for e in envelopes],
    'minimum_element_volume_ratio': [min(r['metrics']['minimum_volume_ratio'] for row in trace for r in row['regions']) for trace in traces],
    'maximum_active_contacts': [max(row['integration']['active_contact_samples'] for row in trace) for trace in traces],
    'limitations': 'Two-rate sensitivity only, not a converged reference or anatomical calibration. Surface envelopes and centroids do not bound per-vertex error. Offline capture is not a real-time benchmark.'
}
(OUT / f'{PREFIX}-comparison.json').write_text(json.dumps(report, indent=2) + '\n')
fig, axes = plt.subplots(2, 1, figsize=(11, 6), sharex=True, layout='constrained')
for rate, envelope, centroid in zip(RATES, envelopes, centroids):
    axes[0].plot(times, envelope, label=f'{rate} Hz')
    axes[1].plot(times, centroid[:, 0, 1], label=f'{rate} Hz')
axes[0].set_ylabel('Maximum skin displacement (mm)')
axes[1].set_ylabel('Left tissue centroid Y offset (mm)')
axes[1].set_xlabel('Unchanged body source time (s)')
for axis in axes:
    axis.legend()
    axis.grid(alpha=.25)
fig.savefig(OUT / f'{PREFIX}-response.png', dpi=160)
video = OUT / f'{FAMILY + "-" if FAMILY == "viscous" else ""}{FINE}hz-jump.mp4'
if not video.exists():
    encode(CAPTURES[1], video)
assert probe_timestamps(video) == [stamp - stamps[0][0] for stamp in stamps[0]]
old = ART / 'lab-review-hardening-8/front-on.mp4' if FINE == 240 and FAMILY == 'morph' else OUT / f'{FAMILY + "-" if FAMILY == "viscous" else ""}{RATES[0]}hz-jump.mp4'
if not old.exists(): encode(CAPTURES[0], old)
assert probe_timestamps(old) == [stamp - stamps[0][0] for stamp in stamps[0]]
filters = [f'[{i}:v]crop=600:900:60:0,pad=600:940:0:40:color=0x18202c,drawtext=fontcolor=white:fontsize=20:x=14:y=10:text={rate} Hz[v{i}]' for i, rate in enumerate(RATES)]
filters += ['[v0][v1]hstack=inputs=2[v]']
result = OUT / f'{PREFIX}-comparison.mp4'
subprocess.run(['/usr/bin/ffmpeg', '-v', 'error', '-n', '-i', str(old), '-i', str(video), '-filter_complex', ';'.join(filters), '-map', '[v]', '-c:v', 'libx264', '-crf', '18', '-pix_fmt', 'yuv420p', '-fps_mode', 'vfr', '-enc_time_base', '1:1000', '-video_track_timescale', '1000', '-bf', '0', '-movflags', '+faststart', str(result)], check=True)
assert probe_timestamps(result) == [stamp - stamps[0][0] for stamp in stamps[0]]
print(json.dumps(report, indent=2))
