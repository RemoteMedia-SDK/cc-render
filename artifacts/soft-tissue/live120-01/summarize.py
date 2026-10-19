"""Live 120 Hz timing per run, with the metrics of solver-runtime/live_clock_audit.py
plus per-frame tissue stage costs (surface_ms is the displayed skin update)."""
import json, sys
from pathlib import Path
import numpy as np

HERE = Path(__file__).resolve().parent
lines = lambda p: [json.loads(l) for l in p.read_text().splitlines()]
source = (HERE.parents[0] / 'lab-jump-01/packets.ndjson').read_bytes()
rows = {}
for folder in sorted(p for p in HERE.iterdir() if p.is_dir()):
    config, capture = json.loads((folder/'config.json').read_text()), json.loads((folder/'capture.json').read_text())
    assert config['clock'] == 'Bevy realtime' and not config['uncapped_scheduler'] and config['tissue_hz'] == '120'
    assert (folder/'packets.ndjson').read_bytes() == source
    frames, trace = lines(folder/'frames.ndjson'), lines(folder/'tissue.ndjson')
    stamps = np.array([r['pts_ms'] for r in frames]); wall = np.array([r['wall_elapsed_ms'] for r in frames])
    gaps = np.diff(stamps); drift = (wall - wall[0]) - (stamps - stamps[0])
    assert stamps[-1] == 8000 and [r['pts_ms'] for r in trace] == stamps.tolist()
    stage = lambda k: np.array([r['integration'][k] for r in trace])
    rows[folder.name] = {
        'readbacks': len(frames), 'playback_wall_s': (wall[-1] - wall[0]) / 1000,
        'gap_p50_p95_max_ms': np.quantile(gaps, [.5, .95, 1]).round(1).tolist(), 'gaps_over_50ms': int((gaps > 50).sum()),
        'drift_max_end_ms': [round(float(drift.max()), 1), round(float(drift[-1]), 1)],
        'surface_ms_mean_p95_max': np.quantile(stage('surface_ms'), [.5, .95, 1]).round(2).tolist() if 'surface_ms' in trace[0]['integration'] else None,
        'surface_ms_mean': float(stage('surface_ms').mean()),
        'elasticity_s': float(stage('elasticity_ms').sum() / 1000), 'sampling_s': float(stage('sampling_ms').sum() / 1000),
        'subdivisions': int(stage('subdivisions').sum()),
        'peak_skin_mm': max(r['maximum_surface_displacement_m'] for r in trace) * 1000,
        'max_skin_error_mm': max(r['skin_error_m'] for r in trace) * 1000,
    }
json.dump(rows, open(HERE/'summary.json', 'w'), indent=1)
print(f"{'run':20} {'reads':>5} {'wall s':>6} {'gap p50/p95/max':>18} {'>50':>4} {'drift max/end':>14} {'surf ms p50/p95/max':>20} {'elast s':>7} {'subdiv':>6} {'skin mm':>7}")
for n, r in rows.items():
    print(f"{n:20} {r['readbacks']:5d} {r['playback_wall_s']:6.2f} {str(r['gap_p50_p95_max_ms']):>18} {r['gaps_over_50ms']:4d} {str(r['drift_max_end_ms']):>14} {str(r['surface_ms_mean_p95_max']):>20} {r['elasticity_s']:7.2f} {r['subdivisions']:6d} {r['peak_skin_mm']:7.1f}")
print()
for case in ['selected-linear', 'selected-mls4', 'v4-linear', 'v4-mls4']:
    rs = [r for n, r in rows.items() if n.rsplit('-r', 1)[0] == case]
    m = lambda f: np.mean([f(r) for r in rs])
    print(f"{case:16} readbacks {m(lambda r: r['readbacks']):5.1f}  p95 gap {m(lambda r: r['gap_p50_p95_max_ms'][1]):5.1f} ms  end drift {m(lambda r: r['drift_max_end_ms'][1]):6.1f} ms  surface {m(lambda r: r['surface_ms_mean']):5.2f} ms/frame  elasticity {m(lambda r: r['elasticity_s']):5.2f} s")
