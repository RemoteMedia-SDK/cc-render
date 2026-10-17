"""Read-only fixture approaches, including non-navigation collider obstructions."""
import argparse
import hashlib
import json
from pathlib import Path

from check_bedroom_clearance import corridor


def check(manifest):
    raw = manifest.read_bytes()
    package = json.loads(raw)
    solids = {s['id']:s for s in package['solids']}
    for name in ('vanity', 'shower_screen', 'bathroom_toilet'):
        assert solids[name]['navigation_obstacle'], name
    # Include every collider intersecting this vertical clearance envelope,
    # even wall fittings that do not participate in the navigation grid.
    obstacles = [s for s in solids.values()
                 if s['position'][1] + s['half_extents'][1] > .05
                 and s['position'][1] - s['half_extents'][1] < 2.0]
    entry = [(-1.8, 0, -.6), (-2.45, 0, -1.54), (-3.78, 0, -1.54), (-3.78, 0, -2.83)]
    routes = {'vanity': entry + [(-4.75, 0, -2.83)],
              'shower': entry + [(-4.05, 0, -3.4), (-4.05, 0, -4.25), (-5.05, 0, -4.25)],
              'toilet': entry + [(-3.35, 0, -3.35)]}
    evidence = {name: {'corners':points, **corridor(points, obstacles)} for name, points in routes.items()}
    # Negative control: a solid enclosing the shower interior must be rejected.
    blocker = {'id':'negative_control_shower_block', 'position':[-5.05, 1, -4.25],
               'half_extents':[.2, 1, .2]}
    try:
        corridor(routes['shower'], obstacles + [blocker])
    except AssertionError:
        rejected = True
    else:
        raise AssertionError('Clearance check accepted the blocked shower')
    return {'manifest_sha256':hashlib.sha256(raw).hexdigest(), 'probe_radius_m':.35,
            'vertical_envelope_m':[.05, 2.0], 'routes':evidence, 'negative_control_rejected':rejected,
            'scope':'Static free-space envelope, not an avatar trajectory or acceptance of traversal, stepping, bathing, seating or fixture use.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = check(args.manifest)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
