"""Compare actual post-skinning geometry, retaining the same source-pose surface."""
import hashlib
import json
from pathlib import Path

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
ARTIFACTS = HERE.parent

def records(path):
    return [json.loads(line) for line in path.read_text().splitlines()]

def capture(name):
    path = ARTIFACTS / name
    rows = records(path / 'tissue.ndjson')
    frames = records(path / 'frames.ndjson')
    assert [r['pts_ms'] for r in rows] == [f['pts_ms'] for f in frames]
    assert len(rows) == 478 and rows[-1]['pts_ms'] == 8000
    packets = path / 'packets.ndjson'
    assert packets.read_bytes() == (ARTIFACTS / 'lab-jump-01/packets.ndjson').read_bytes()
    return rows, {
        'capture': name, 'frames': len(rows), 'last_pts_ms': rows[-1]['pts_ms'],
        'packets_sha256': hashlib.sha256(packets.read_bytes()).hexdigest(),
        'maximum_displacement_mm': max(r['maximum_surface_displacement_m'] for r in rows) * 1000,
        'minimum_volume_ratio': min(s['metrics']['minimum_volume_ratio'] for r in rows for s in r['regions']),
        'maximum_active_contacts': max(r['integration']['active_contact_samples'] for r in rows),
        'maximum_skinning_error_mm': max(r['skin_error_m'] for r in rows) * 1000,
    }

def geometry(name):
    data = json.loads((HERE / f'{name}-geometry.json').read_text())
    assert data['source_pts_ms'] == 7633
    return next(s for s in data['surfaces'] if len(s['affected_triangles']))

def measure(reference, candidate):
    assert reference['before_world'] == candidate['before_world']
    before = np.asarray(reference['before_world'])
    after = np.asarray(candidate['after_world'])
    faces = np.asarray(reference['affected_triangles'])
    pre, post = before[faces], after[faces]
    normal = np.cross(pre[:, 1] - pre[:, 0], pre[:, 2] - pre[:, 0])
    moved_normal = np.cross(post[:, 1] - post[:, 0], post[:, 2] - post[:, 0])
    area = np.linalg.norm(normal, axis=1) / 2
    moved_area = np.linalg.norm(moved_normal, axis=1) / 2
    displacement = np.linalg.norm(after - before, axis=1)
    face_displacement = displacement[faces].mean(axis=1)
    cosine = np.einsum('ij,ij->i', normal, moved_normal) / (4 * area * moved_area)
    assert np.all(np.isfinite(after)) and np.all(area > 0) and np.all(moved_area > 0)
    return {
        'source_pts_ms': 7633, 'same_undeformed_world_surface': True,
        'reference_faces': len(faces), 'reference_area_cm2': float(area.sum() * 1e4),
        'area_weighted_displacement_mm': float(np.dot(area, face_displacement) / area.sum() * 1000),
        'area_fraction_moving_over_5mm': float(area[face_displacement > .005].sum() / area.sum()),
        'area_fraction_moving_over_10mm': float(area[face_displacement > .010].sum() / area.sum()),
        'maximum_displacement_mm': float(displacement.max() * 1000),
        'area_ratio_percentiles_0_1_5_50_95_99_100': np.percentile(moved_area / area, [0, 1, 5, 50, 95, 99, 100]).tolist(),
        'minimum_normal_alignment': float(cosine.min()),
        'faces_rotated_over_90_degrees': int((cosine < 0).sum()),
        'area_fraction_rotated_over_90_degrees': float(area[cosine < 0].sum() / area.sum()),
    }, (before, faces, displacement)

def main():
    names = ['surface-old-front-01', 'surface-geometric-front-01']
    if (HERE / 'surface-rim-front-01-geometry.json').exists():
        names.append('surface-rim-front-01')
    base = geometry(names[0])
    report = {'geometry_at_source_ms': 7633, 'candidates': [], 'limitations': [
        'Geometry distribution is measured at one impact frame, not across the full sequence.',
        'Normal rotation relative to the undeformed triangle is a distortion warning, not a self-intersection proof.',
        'Boundary selection and tissue coefficients remain prototypes without anatomical calibration.',
        'These fixed-clock captures do not establish interactive performance or active contact response.'
    ]}
    baseline, _ = capture('lab-' + names[0])
    fig, axes = plt.subplots(1, len(names), figsize=(6 * len(names), 5), layout='constrained')
    for axis, name in zip(axes, names, strict=True):
        rows, summary = capture('lab-' + name)
        summary['attachment_frames_exactly_unchanged'] = all(
            a['anchor_world_m'] == b['anchor_world_m'] and a['anchor_rotation_xyzw'] == b['anchor_rotation_xyzw']
            for old, new in zip(baseline, rows, strict=True) for a, b in zip(old['regions'], new['regions'], strict=True))
        summary['physical_regions_trace_exactly_unchanged'] = all(old['regions'] == new['regions'] for old, new in zip(baseline, rows, strict=True))
        summary['geometry'], (points, faces, delta) = measure(base, geometry(name))
        report['candidates'].append(summary)
        mesh = axis.tripcolor(points[:, 0] * 100, points[:, 1] * 100, faces, delta * 1000, vmin=0, vmax=45)
        axis.set_aspect('equal'); axis.set_title(name.replace('surface-', '').replace('-01', ''))
        axis.set_xlabel('World X (cm)'); axis.set_ylabel('World Y (cm)')
    fig.colorbar(mesh, ax=axes, label='Actual skin displacement (mm)')
    fig.suptitle('Same source pose at 7.633 s · displacement over the whole original patch')
    fig.savefig(HERE / 'rendered-displacement-comparison.png', dpi=130)
    plt.close(fig)
    (HERE / 'rendered-comparison.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))

if __name__ == '__main__':
    main()
