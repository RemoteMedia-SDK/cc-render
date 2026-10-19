import json, sys
rows = [json.loads(l) for l in open(sys.argv[1])]
I = lambda r, k: r['integration'].get(k, 0)
contact = [r for r in rows if I(r, 'active_contact_samples')]
out = {'frames': len(rows), 'last_pts': rows[-1]['pts_ms'],
  'first_contact_pts': contact[0]['pts_ms'] if contact else None,
  'peak_surface_mm': round(max(r['maximum_surface_displacement_m'] for r in rows) * 1e3, 2),
  'max_active_samples': max(I(r, 'active_contact_samples') for r in rows),
  'max_force_limited_samples': max(I(r, 'force_limited_contact_samples') for r in rows),
  'max_limited_overlap_mm': round(max(I(r, 'maximum_force_limited_penetration_m') for r in rows) * 1e3, 3),
  'max_normal_force_n': round(max(I(r, 'maximum_normal_force_n') for r in rows), 2),
  'volume_ratio_min': [round(min(r['regions'][k]['metrics']['minimum_volume_ratio'] for r in rows), 3) for k in (0, 1)],
  'volume_ratio_max': [round(max(r['regions'][k]['metrics']['maximum_volume_ratio'] for r in rows), 3) for k in (0, 1)],
  'max_speed_m_s': [round(max(r['regions'][k]['metrics']['maximum_speed_m_s'] for r in rows), 3) for k in (0, 1)],
  'subdivisions': sum(I(r, 'subdivisions') for r in rows)}
print(json.dumps(out))
