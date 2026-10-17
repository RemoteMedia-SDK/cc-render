"""Read-only bedroom free-space checks, separate from avatar motion acceptance."""
import argparse
import hashlib
import json
import math
from pathlib import Path

from check_balcony_clearance import clearance


def corridor(corners, obstacles):
    probes = []
    for start, end in zip(corners, corners[1:]):
        steps = math.ceil(math.dist(start, end) / .01)
        probes.extend(tuple(a + (b - a) * i / steps for a, b in zip(start, end)) for i in range(steps + 1))
    nearest = min((clearance(point, solid), solid['id']) for point in probes for solid in obstacles)
    bound = nearest[0] - .005
    assert bound >= .35, ('bedroom corridor blocked', bound, nearest[1])
    return {'probe_count':len(probes), 'minimum_sample_clearance_m':nearest[0],
            'swept_clearance_lower_bound_m':bound, 'nearest_obstacle':nearest[1]}


def check(manifest):
    raw = manifest.read_bytes()
    solids = {s['id']:s for s in json.loads(raw)['solids']}
    assert not solids['bedside_lamp']['navigation_obstacle']
    obstacles = [s for s in solids.values() if s.get('navigation_obstacle')]
    # Geometric probes only: these coordinates are never sent to an avatar.
    west = [(0,0,-.6),(2.8,0,-.6),(2.8,0,-3.4)]
    east = [(2.8,0,-1.66),(5.55,0,-1.66),(5.55,0,-3.3)]
    return {'manifest_sha256':hashlib.sha256(raw).hexdigest(), 'probe_radius_m':.35,
            'west_approach':corridor(west, obstacles), 'east_approach':corridor(east, obstacles),
            'scope':'Static XZ corridor checks only; not avatar traversal, lying, sitting, cloth or support acceptance.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest',type=Path)
    parser.add_argument('output',type=Path)
    args = parser.parse_args()
    result = check(args.manifest)
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))
