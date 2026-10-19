"""Water outlets in Y-up metres, matching kitchen_furniture/bathroom_furniture.

These receiving heights describe the visible basin interiors and shower tile;
conservative navigation boxes deliberately do not describe the sink cavities.
Run directly to refresh existing condo manifests without re-exporting geometry.
"""
import json
from pathlib import Path


def water_fixtures():
    return [
        dict(id='kitchen_faucet', kind='faucet', outlet=[-5.57, 1.10, -.35],
             surface_y=.765, spray_radius_m=.002, max_liters_per_minute=6),
        dict(id='bathroom_faucet', kind='faucet', outlet=[-5.51, 1.05, -2.85],
             surface_y=.707, spray_radius_m=.002, max_liters_per_minute=6),
        dict(id='shower', kind='shower', outlet=[-5.55, 2.158, -4.30],
             surface_y=.024, spray_radius_m=.105, max_liters_per_minute=9),
    ]


def shower_destination(solids):
    """Approach the existing shower-screen target from beneath the water outlet."""
    outlet = next(f for f in water_fixtures() if f['id'] == 'shower')['outlet']
    screen = next((s for s in solids if s['id'] == 'shower_screen'), None)
    if screen is None:  # Older diagnostic packages predate the detailed bathroom.
        return
    screen['approach'] = [outlet[0] - screen['position'][0], 0.,
                          outlet[2] - screen['position'][2]]


if __name__ == '__main__':
    root = Path(__file__).resolve().parents[2] / 'scenes' / 'toronto_condo'
    for path in sorted(root.glob('scene*.json')):
        package = json.loads(path.read_text())
        if package.get('schema_version') != 1 or 'geometry' not in package:
            continue
        package['water_fixtures'] = water_fixtures()
        shower_destination(package['solids'])
        path.write_text(json.dumps(package, indent=2) + '\n')
        print(path.name)
