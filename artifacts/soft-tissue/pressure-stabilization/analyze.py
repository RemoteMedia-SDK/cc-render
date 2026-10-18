"""Evaluate the declared local-volume and displacement screen on CPU replays."""
from pathlib import Path
import argparse
import json
import numpy as np
from scipy.spatial.transform import Rotation, Slerp

HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--input', type=Path, default=HERE/'study-01')
OUT = parser.parse_args().input.resolve()
ROOT = HERE.parents[2]
trace = [json.loads(line) for line in (HERE.parent/'lab-boundary-nodal-front-01/tissue.ndjson').read_text().splitlines()]
times = np.asarray([row['pts_ms'] for row in trace])
plan = json.loads((OUT/'manifest.json').read_text())
quality_study = 'quality_cutoffs' in plan
cases = plan.get('quality_cutoffs', plan.get('shares_pa', []))
prefix = 'quality' if quality_study else 'share'
parameter = 'quality_cutoff' if quality_study else 'share_pa'
if 'cases' in plan:
    cases = [case['id'] for case in plan['cases']]
    prefix, parameter = 'case', 'case'
assert cases and cases[0] in (0, 'reference') and len(set(cases)) == len(cases)
rows = []
baseline_surface = None


def volumes(points, cells):
    p = points[..., cells, :]
    return np.linalg.det(np.swapaxes(p[..., 1:, :] - p[..., :1, :], -1, -2))/6.


for share in cases:
    asset = json.loads((OUT/f'{prefix}-{share}-fitted.json').read_text())
    run = json.loads((OUT/f'{prefix}-{share}-run.json').read_text())
    counts = [len(region['rest_positions']) for region in asset['regions']]
    positions = np.fromfile(OUT/f'{prefix}-{share}-positions.bin', dtype='<f8').reshape(run['steps'], sum(counts), 3)
    sample_times = 50. + np.arange(1, run['steps']+1)*1000./run['rate_hz']
    offset = 0
    surfaces, regions = [], []
    for i, region in enumerate(asset['regions']):
        nodes = positions[:, offset:offset+counts[i]]
        offset += counts[i]
        cells = np.asarray(region['tetrahedra'])
        rest_volume = volumes(np.asarray(region['rest_positions']), cells)
        current = volumes(nodes, cells)
        ratio = current/rest_volume
        minimum_step, minimum_tet = np.unravel_index(ratio.argmin(), ratio.shape)
        translations = np.asarray([row['regions'][i]['anchor_world_m'] for row in trace])
        interpolated = np.stack([np.interp(sample_times, times, translations[:, axis]) for axis in range(3)], axis=-1)
        rotations = Rotation.from_quat([row['regions'][i]['anchor_rotation_xyzw'] for row in trace])
        matrices = Slerp(times, rotations)(sample_times).as_matrix()
        reference = np.einsum('tij,nj->tni', matrices, region['initial_positions']) + interpolated[:, None, :]
        bindings = [binding for surface in asset['surfaces'] for binding in surface['vertices'] if binding['region']==i]
        ids = cells[[binding['tetrahedron'] for binding in bindings]]
        weights = np.asarray([binding['barycentric'] for binding in bindings])
        influence = np.asarray([binding['influence'] for binding in bindings])
        surface = np.einsum('tnki,nk->tni', (nodes-reference)[:, ids], weights)*influence[None, :, None]
        surfaces.append(surface)
        regions.append({'name': region['name'], 'volume_ratio_min_max': [float(ratio.min()), float(ratio.max())],
                        'minimum_volume_element': int(minimum_tet), 'minimum_volume_time_ms': float(sample_times[minimum_step]),
                        'minimum_element_rest_volume_mm3': float(rest_volume[minimum_tet]*1e9),
                        'minimum_element_fixed_nodes': sum(region['attachments'][node]['kind']=='fixed' for node in cells[minimum_tet]),
                        'maximum_total_volume_change': float(abs(current.sum(axis=1)/rest_volume.sum()-1.).max()),
                        'maximum_volume_fraction_outside_50_percent': float(((abs(ratio-1.)>.5)*rest_volume).sum(axis=1).max()/rest_volume.sum()),
                        'rest_volume_m3': float(rest_volume.sum())})
    surface = np.concatenate(surfaces, axis=1)
    displacement = np.linalg.norm(surface, axis=2)
    if baseline_surface is None:
        baseline_surface = surface
    baseline = np.linalg.norm(baseline_surface, axis=2)
    peak_ratio = float(displacement.max()/baseline.max())
    rms_ratio = float(np.sqrt(np.mean(displacement**2)/np.mean(baseline**2)))
    passes = all(region['volume_ratio_min_max'][0]>=.5 and region['volume_ratio_min_max'][1]<=2.
                 and region['maximum_total_volume_change']<.01 for region in regions)
    rows.append({parameter: share, 'regions': regions,
                 'surface_peak_m': float(displacement.max()), 'surface_time_rms_m': float(np.sqrt(np.mean(displacement**2))),
                 'peak_ratio_to_baseline': peak_ratio, 'rms_ratio_to_baseline': rms_ratio,
                 'maximum_surface_change_m': float(np.linalg.norm(surface-baseline_surface, axis=2).max()),
                 'passes_declared_screen': passes and peak_ratio>=.8 and rms_ratio>=.8,
                 'solve_seconds': run['solve_seconds'], 'subdivisions': run['subdivisions']})
report = {'cases': rows, 'limits': ['Equal-weight embedded vertices, not area-weighted rendered pixels.',
                                  'Recorded attachment interpolation; no contact or fractional display solve.',
                                  'CPU timing is diagnostic only; study-01 overlapped a crate compilation.']}
(OUT/'comparison.json').write_text(json.dumps(report, indent=2)+'\n')
for row in rows:
    print(row[parameter], [r['volume_ratio_min_max'] for r in row['regions']],
          'peak/rms', row['peak_ratio_to_baseline'], row['rms_ratio_to_baseline'], 'pass', row['passes_declared_screen'])
