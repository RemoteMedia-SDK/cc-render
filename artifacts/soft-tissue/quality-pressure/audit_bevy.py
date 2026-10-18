"""Check unchanged source sampling, broad surface response and candidate volume."""
from pathlib import Path
import json
import sys
import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent/'solver-runtime'))
from startup_cadence_audit import inspect as fixed
from live_clock_audit import inspect as live


def surface_statistics(path):
    snapshot = json.loads(path.read_text())
    surface = next(s for s in snapshot['surfaces'] if s['mesh_name']=='CC_Base_Body')
    before, after = np.asarray(surface['before_world']), np.asarray(surface['after_world'])
    triangles = np.asarray(surface['affected_triangles'])
    base, moved = before[triangles], after[triangles]
    normal = np.cross(base[:, 1]-base[:, 0], base[:, 2]-base[:, 0])
    changed = np.cross(moved[:, 1]-moved[:, 0], moved[:, 2]-moved[:, 0])
    area = np.linalg.norm(normal, axis=1)/2
    displacement = np.linalg.norm(after-before, axis=1)
    face_displacement = displacement[triangles].mean(axis=1)
    alignment = np.sum(normal*changed, axis=1)/(np.linalg.norm(normal, axis=1)*np.linalg.norm(changed, axis=1))
    return {'pts_ms': snapshot['source_pts_ms'], 'reference_faces': len(triangles),
            'reference_area_cm2': float(area.sum()*1e4),
            'area_weighted_displacement_mm': float(np.average(face_displacement, weights=area)*1000),
            'area_fraction_triangles_mean_displacement_over_10mm': float(area[face_displacement>.01].sum()/area.sum()),
            'maximum_displacement_mm': float(displacement.max()*1000),
            'minimum_normal_alignment': float(alignment.min()),
            'faces_rotated_over_90_degrees': int((alignment<0).sum())}, before


packets = (HERE.parent/'lab-jump-01/packets.ndjson').read_bytes()
baseline, old = fixed('lab-boundary-nodal-front-01', packets)
candidate, new = fixed('lab-quality-front-01', packets)
assert old.keys() == new.keys()
translation, rotation, node_difference = [], [], []
for time in old:
    for a, b in zip(old[time]['regions'], new[time]['regions'], strict=True):
        translation.append(np.linalg.norm(np.asarray(a['anchor_world_m'])-b['anchor_world_m']))
        rotation.append(np.linalg.norm(np.asarray(a['anchor_rotation_xyzw'])-b['anchor_rotation_xyzw']))
        node_difference.extend(np.linalg.norm(np.asarray(a['nodes_world_m'])-b['nodes_world_m'], axis=1))
old_surface, old_before = surface_statistics(HERE.parent/'volume-audit/boundary-front-01-geometry.json')
new_surface, new_before = surface_statistics(HERE/'bevy-01/geometry.json')
volume = json.loads((HERE/'bevy-01/volume-report.json').read_text())
min_j = min(region['metrics']['minimum_volume_ratio'] for row in new.values() for region in row['regions'])
max_j = max(region['metrics']['maximum_volume_ratio'] for row in new.values() for region in row['regions'])
report = {'fixed_baseline': baseline, 'fixed_candidate': candidate,
          'live_with_node_tracing': live('lab-quality-live120-01', packets)[0],
          'live_without_node_tracing': live('lab-quality-live120-timing-01', packets)[0],
          'source_fidelity': {'frames': len(old), 'maximum_anchor_translation_difference_m': float(max(translation)),
                              'maximum_anchor_quaternion_difference': float(max(rotation)),
                              'maximum_undeformed_surface_difference_m': float(np.linalg.norm(new_before-old_before, axis=1).max())},
          'maximum_node_change_m': float(max(node_difference)),
          'surface_baseline': old_surface, 'surface_candidate': new_surface,
          'sampled_local_volume_range': [min_j, max_j],
          'limits': ['Fixed capture includes PNG writing; its wall time is not a like-for-like throughput benchmark.',
                     'Live captures are single runs on a shared host; readbacks are not browser presentations.',
                     'The reviewed jump has no active contact. Positive face alignment is not a self-intersection guarantee.']}
(HERE/'bevy-01/comparison.json').write_text(json.dumps(report, indent=2)+'\n')
assert max(translation)==0 and max(rotation)==0
assert np.array_equal(new_before, old_before)
assert min_j>=.5 and max_j<=2.
assert all(all(abs(v-1)<.01 for v in region['global_volume_range']) for region in volume['regions'])
assert new_surface['area_weighted_displacement_mm']>=.8*old_surface['area_weighted_displacement_mm']
assert candidate['peak_surface_displacement_mm']>=.8*baseline['peak_surface_displacement_mm']
assert new_surface['faces_rotated_over_90_degrees']==0
print(json.dumps({'surface': new_surface, 'sampled_local_volume_range': [min_j, max_j],
                  'source_fidelity': report['source_fidelity']}, indent=2))
