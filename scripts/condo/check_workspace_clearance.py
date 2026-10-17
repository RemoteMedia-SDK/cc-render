"""Static entry/workspace free-space evidence; never sent to an avatar."""
import argparse
import hashlib
import json
from pathlib import Path

from check_bedroom_clearance import corridor


def check(manifest):
    raw = manifest.read_bytes()
    package = json.loads(raw)
    solids = {s['id']: s for s in package['solids']}
    for name in ('workspace_desk', 'workspace_chair', 'entry_storage', 'entry_bench', 'entry_door'):
        assert solids[name]['navigation_obstacle'], name
    obstacles = [s for s in solids.values() if s.get('navigation_obstacle')]
    # Free-space probes begin INSIDE the static, closed entry door. They test
    # layout clearance only and are not movement, body targets or seating goals.
    routes = {
        'entry_to_living': [(-.35, 0, -4.40), (-.70, 0, -3.9), (-.70, 0, -1.2), (0, 0, 0)],
        'cabinet_approach': [(-.70, 0, -3.9), (-1.8, 0, -3.95)],
        'desk_side_approach': [(0, 0, 0), (0, 0, -2.25), (.55, 0, -2.25)],
    }
    return {'manifest_sha256':hashlib.sha256(raw).hexdigest(), 'probe_radius_m':.35,
            'routes':{name: {'corners':corners, **corridor(corners, obstacles)}
                      for name, corners in routes.items()},
            'scope':'Static horizontal clearance only. Door remains closed/static; no avatar traversal, desk use, chair seating or support acceptance.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = check(args.manifest)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
