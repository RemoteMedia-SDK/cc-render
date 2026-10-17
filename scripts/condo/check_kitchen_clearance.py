"""Read-only kitchen corridor checks; no motion is generated or submitted."""
import argparse
import hashlib
import json
import math
from pathlib import Path

from check_balcony_clearance import clearance


def check(manifest):
    raw = manifest.read_bytes()
    solids = {s['id']: s for s in json.loads(raw)['solids']}
    obstacles = [s for s in solids.values() if s.get('navigation_obstacle')]
    island = solids['island_cabinet']
    wall = solids['bathroom_south_wall_a']
    north_gap = island['position'][2] - island['half_extents'][2] - (wall['position'][2] + wall['half_extents'][2])
    assert north_gap >= .75, ('north entrance too narrow', north_gap)
    for name in ('kitchen_upper', 'kitchen_hood', 'kitchen_backsplash', 'kitchen_faucet'):
        assert not solids[name]['navigation_obstacle'], name
    # A static free-space corridor, not an avatar root trajectory or motion request.
    corners = [(0, 0, 0), (-2.5, 0, 0), (-2.5, 0, -1.54), (-4.6, 0, -1.54), (-4.6, 0, 0)]
    probes = []
    for start, end in zip(corners, corners[1:]):
        length = math.dist(start, end)
        steps = math.ceil(length / .05)
        probes.extend(tuple(a + (b - a) * i / steps for a, b in zip(start, end)) for i in range(steps + 1))
    nearest = min((clearance(point, solid), solid['id']) for point in probes for solid in obstacles)
    bound = nearest[0] - .025  # Half the maximum probe spacing; distance is 1-Lipschitz.
    assert bound >= .35, ('corridor blocked', bound, nearest[1])
    return {'manifest_sha256': hashlib.sha256(raw).hexdigest(), 'north_gap_m': north_gap,
            'probe_radius_m': .35, 'probe_count': len(probes), 'minimum_sample_clearance_m': nearest[0],
            'swept_clearance_lower_bound_m': bound, 'nearest_obstacle': nearest[1],
            'scope': 'Static corridor feasibility only. No avatar traversal, reach, support or interaction acceptance.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = check(args.manifest)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
