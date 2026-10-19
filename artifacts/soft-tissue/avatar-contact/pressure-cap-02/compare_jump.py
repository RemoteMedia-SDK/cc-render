import json, sys, math
def load(p):
    return [json.loads(l) for l in open(p)]
a, b = load(sys.argv[1]), load(sys.argv[2])
timing = {'collision_ms','elasticity_ms','sampling_ms','wall_elapsed_ms','solve_ms','step_ms'}
def strip(o):
    if isinstance(o, dict): return {k: strip(v) for k, v in o.items() if k not in timing and not k.endswith('_ms') or k == 'pts_ms'}
    if isinstance(o, list): return [strip(v) for v in o]
    return o
def maxdiff(x, y):
    if isinstance(x, dict): return max([maxdiff(x[k], y[k]) for k in x] + [0.0])
    if isinstance(x, list): return max([maxdiff(p, q) for p, q in zip(x, y)] + [0.0])
    if isinstance(x, (int, float)) and isinstance(y, (int, float)): return abs(x - y)
    return 0.0 if x == y else float('inf')
sa, sb = [strip(r) for r in a], [strip(r) for r in b]
identical = sum(1 for p, q in zip(sa, sb) if p == q)
worst = max(maxdiff(p, q) for p, q in zip(sa, sb))
peak = max(r.get('maximum_surface_displacement_m', 0) for r in b)
contacts = max(r['integration']['active_contact_samples'] for r in b)
subdiv = (sum(r['integration']['subdivisions'] for r in a), sum(r['integration']['subdivisions'] for r in b))
print(json.dumps({'frames': (len(a), len(b)), 'last_pts_ms': (a[-1]['pts_ms'], b[-1]['pts_ms']),
  'frames_identical_excluding_timing': identical, 'maximum_value_difference': worst,
  'peak_surface_displacement_mm': peak * 1000, 'maximum_active_contact_samples': contacts,
  'subdivisions_baseline_new': subdiv, 'keys': sorted(sa[0].keys())}, indent=1))
