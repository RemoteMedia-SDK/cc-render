"""Read-only XZ obstacle checks; this does not generate or validate body motion."""
import argparse
import hashlib
import json
import math
from pathlib import Path


def clearance(point, solid):
    dx, dz = point[0] - solid['position'][0], point[2] - solid['position'][2]
    yaw = math.radians(solid.get('yaw_deg', 0))
    x = math.cos(yaw) * dx - math.sin(yaw) * dz
    z = math.sin(yaw) * dx + math.cos(yaw) * dz
    return math.hypot(max(abs(x) - solid['half_extents'][0], 0),
                      max(abs(z) - solid['half_extents'][2], 0))


def check(manifest):
    raw = manifest.read_bytes()
    package = json.loads(raw)
    solids = {s['id']: s for s in package['solids']}
    roof = solids['balcony_overhang']
    floor = solids['balcony_slab_12_x_2_4_m']
    assert not roof['navigation_obstacle'], 'Overhang blocks the entire balcony in XZ'
    floor_y = floor['position'][1] + floor['half_extents'][1]
    headroom = roof['position'][1] - roof['half_extents'][1] - floor_y
    assert headroom >= 2.8
    obstacles = [s for s in solids.values() if s.get('navigation_obstacle')]
    radius = .35  # Current controller PLANNING_ROOT_RESERVE_M; not a body pose.
    report = {'manifest_sha256': hashlib.sha256(raw).hexdigest(),
              'probe_radius_m': radius, 'balcony_headroom_m': headroom, 'approaches': []}
    for key in ('balcony_lounge_west', 'balcony_lounge_east', 'balcony_side_table'):
        solid = solids[key]
        assert solid['navigation_obstacle']
        assert abs(solid['position'][0]) - solid['half_extents'][0] > 1
        point = [a + b for a, b in zip(solid['position'], solid['approach'])]
        nearest = min((clearance(point, other), other['id']) for other in obstacles)
        assert nearest[0] >= radius, (key, nearest)
        assert abs(point[0]) + radius < 6 and 5 + radius < point[2] < 7.4 - radius
        report['approaches'].append({'id': key, 'world_point': point,
                                    'nearest_obstacle': nearest[1], 'clearance_m': nearest[0]})
    route = [(0, floor_y, 4.3 + step / 20) for step in range(55)]
    margin = min(clearance(point, solid) for point in route for solid in obstacles)
    # Distance to an obstacle is 1-Lipschitz. Every segment point is within
    # 0.025 m of a probe, giving a conservative continuous-route bound.
    swept_margin = margin - .025
    assert swept_margin >= radius, ('central_route_blocked', swept_margin)
    report['central_route'] = {'start': route[0], 'end': route[-1],
                               'samples': len(route), 'minimum_sample_clearance_m': margin,
                               'swept_clearance_lower_bound_m': swept_margin}
    report['scope'] = 'Static XZ clearance and ceiling height; not traversal, seating, support or motion acceptance.'
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = check(args.manifest)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
