"""Author the duvet for the one- and two-cover condo beds at a finer resolution.

The layered bedding's duvet (prepare_bedding_layers.duvet_surface) is a 22 x 24 grid, ~10 x 8 cm
cells, for a 12 mm thick cover: folded over itself (the turndown), faces two rows apart across the
crease sit inside each other's contact gap and pry the fold open. Same placement and size; only the
grid is finer. Writes interaction-authoring/bedding/two-covers/bed_duvet.json and points
scene-bedding-cover.json and scene-bedding-two-covers.json at it (bake the settled scenes after).

Usage: python3 scripts/condo/prepare_cover_duvet.py [columns rows]   (default 44 48)
"""
import hashlib
import json
import sys
from pathlib import Path

SCENE = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'


def duvet_surface(columns, rows):
    points, uv, triangles = [], [], []
    for row in range(rows+1):
        for col in range(columns+1):
            u, v = col/columns, row/rows
            points.append([3.04 + 2.26*u, .94, -2.04 - 1.94*v])
            uv.append([u, v])
            if row < rows and col < columns:
                a = row*(columns+1)+col
                triangles.extend([[a, a+1, a+columns+2], [a, a+columns+2, a+columns+1]])
    return {'schema_version': 1, 'rest_positions': points, 'initial_positions': points,
            'triangles': triangles, 'material_uv': uv}


def main():
    columns, rows = (int(a) for a in sys.argv[1:3]) if len(sys.argv) > 2 else (44, 48)
    target = SCENE/'interaction-authoring/bedding/two-covers/bed_duvet.json'
    target.write_text(json.dumps(duvet_surface(columns, rows), separators=(',', ':'))+'\n')
    sha = hashlib.sha256(target.read_bytes()).hexdigest()
    for name in ('scene-bedding-cover.json', 'scene-bedding-two-covers.json'):
        path = SCENE/name
        scene = json.loads(path.read_text())
        duvet = next(c for c in scene['cloths'] if c['id'] == 'bed_duvet')
        duvet.update(surface=str(target.relative_to(SCENE)), sha256=sha)
        path.write_text(json.dumps(scene, indent=2))
    print(json.dumps({'surface': str(target.relative_to(SCENE)), 'sha256': sha,
                      'vertices': (columns+1)*(rows+1), 'triangles': 2*columns*rows}))


if __name__ == '__main__':
    main()
