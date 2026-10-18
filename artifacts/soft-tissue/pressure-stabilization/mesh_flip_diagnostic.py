"""Check internal 2-to-3 flips without moving nodes or changing the asset."""
from pathlib import Path
import itertools
import json
import sys
import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent/'volume-audit'))
from analyze import volumes, quality

asset = json.loads((HERE.parents[2]/'assets/tissue/beautiful_assistant.base-boundary-nodal-volume-prototype.json').read_text())
report = []
for region in asset['regions']:
    rest = np.asarray(region['rest_positions'])
    initial = np.asarray(region['initial_positions'])
    cells = np.asarray(region['tetrahedra'])
    faces, edges = {}, set()
    for index, tet in enumerate(cells):
        for face in itertools.combinations(tet, 3):
            faces.setdefault(tuple(sorted(face)), []).append(index)
        for edge in itertools.combinations(tet, 2):
            edges.add(tuple(sorted(edge)))
    candidates = []
    for face, adjacent in faces.items():
        if len(adjacent) != 2:
            continue
        a, b = [next(v for v in cells[e] if v not in face) for e in adjacent]
        if tuple(sorted((a, b))) in edges:
            continue
        f0, f1, f2 = face
        new = np.asarray([[a, b, f0, f1], [a, b, f1, f2], [a, b, f2, f0]])
        for i, volume in enumerate(volumes(rest, new)):
            if volume < 0:
                new[i, [2, 3]] = new[i, [3, 2]]
        if any(np.min(volumes(p, new)) <= 1e-14 or
               abs(volumes(p, new).sum()-volumes(p, cells[adjacent]).sum()) > 1e-12
               for p in [rest, initial]):
            continue
        before = min(quality(p, cells[adjacent]).min() for p in [rest, initial])
        after = min(quality(p, new).min() for p in [rest, initial])
        candidates.append({'old_tetrahedra': adjacent, 'new_tetrahedra': new.tolist(),
                           'before_quality': float(before), 'after_quality': float(after),
                           'gain': float(after-before)})
    candidates.sort(key=lambda row: row['gain'], reverse=True)
    report.append({'region': region['name'], 'feasible_count': len(candidates),
                   'improving_count': sum(c['gain'] > 1e-5 for c in candidates),
                   'top_candidates': candidates[:12],
                   'right_minimum_pair': [c for c in candidates if set(c['old_tetrahedra']) == {282, 435}]})
(HERE/'mesh-flip-diagnostic.json').write_text(json.dumps(report, indent=2)+'\n')
for row in report:
    print(row['region'], row['feasible_count'], 'feasible;', row['improving_count'], 'improve quality')
