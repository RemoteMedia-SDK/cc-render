import json, sys, numpy as np
asset = json.load(open(sys.argv[1])); trace = sys.argv[2]
rim = [[i for i, a in enumerate(r['attachments']) if a['kind'] == 'compliant'] for r in asset['regions']]
fixed = [[i for i, a in enumerate(r['attachments']) if a['kind'] == 'fixed'] for r in asset['regions']]
init = [np.array(r['initial_positions']) for r in asset['regions']]
def rot(q, v):
    x, y, z, w = q; u = np.array([x, y, z])
    return v + 2 * np.cross(u, np.cross(u, v) + w * v)
rows = []
for line in open(trace):
    r = json.loads(line)
    out = {'pts': r['pts_ms'], 'surf_mm': r['maximum_surface_displacement_m'] * 1e3, 'contacts': r['integration']['active_contact_samples']}
    for k, g in enumerate(r['regions']):
        n = np.array(g['nodes_world_m'])
        target = rot(g['anchor_rotation_xyzw'], init[k]) + np.array(g['anchor_world_m'])
        e = np.linalg.norm(n - target, axis=1)
        out[f'seam{k}_mm'] = float(e[rim[k]].max() * 1e3) if rim[k] else 0.0
        out[f'fixed_err{k}_mm'] = float(e[fixed[k]].max() * 1e3)
        out[f'vmin{k}'] = g['metrics']['minimum_volume_ratio']; out[f'vmax{k}'] = g['metrics']['maximum_volume_ratio']
        out[f'speed{k}'] = g['metrics']['maximum_speed_m_s']
    rows.append(out)
def worst(key, f=max): return f(r[key] for r in rows)
first_bad = next((r['pts'] for r in rows if min(r['vmin0'], r['vmin1']) < 0.85 or max(r['vmax0'], r['vmax1']) > 1.2), None)
print(json.dumps({'frames': len(rows), 'last_pts': rows[-1]['pts'],
  'peak_surface_mm': round(worst('surf_mm'), 2),
  'seam_mm_max': [round(worst('seam0_mm'), 3), round(worst('seam1_mm'), 3)],
  'fixed_node_error_mm_max': [worst('fixed_err0_mm'), worst('fixed_err1_mm')],
  'volume_ratio_min': [round(worst('vmin0', min), 3), round(worst('vmin1', min), 3)],
  'volume_ratio_max': [round(worst('vmax0'), 3), round(worst('vmax1'), 3)],
  'max_speed_m_s': [round(worst('speed0'), 3), round(worst('speed1'), 3)],
  'first_pts_volume_outside_0.85_1.2': first_bad,
  'max_active_contacts': worst('contacts')}))
