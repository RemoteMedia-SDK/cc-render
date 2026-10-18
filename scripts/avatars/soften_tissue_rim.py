#!/usr/bin/env python3
"""Make the skin-surface chest rim of a tissue asset compliant.

Only fixed nodes on the skin side of the boundary (a boundary face with a free
node) change; the posterior cap and walls stay fixed. Geometry, rest shape,
material and skin bindings are unchanged. The compliance is an engineering
attachment approximation, not an anatomical measurement.
"""
import argparse
import hashlib
import json
from pathlib import Path


def skin_rim(region):
    fixed = [a['kind'] == 'fixed' for a in region['attachments']]
    count = {}
    for t in region['tetrahedra']:
        for face in ((t[0], t[1], t[2]), (t[0], t[1], t[3]), (t[0], t[2], t[3]), (t[1], t[2], t[3])):
            key = tuple(sorted(face))
            count[key] = count.get(key, 0) + 1
    rim = set()
    for face, n in count.items():
        if n == 1 and not all(fixed[i] for i in face):
            rim.update(i for i in face if fixed[i])
    return sorted(rim)


def soften(source, compliance, output):
    asset = json.loads(source.read_text())
    report = {'source': str(source), 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
              'compliance_m_per_n': compliance, 'regions': []}
    for region in asset['regions']:
        rim = skin_rim(region)
        for i in rim:
            region['attachments'][i] = {'kind': 'compliant', 'compliance_m_per_n': compliance}
        report['regions'].append({'name': region['name'], 'compliant_rim_nodes': rim,
                                  'remaining_fixed': sum(a['kind'] == 'fixed' for a in region['attachments'])})
    with output.open('x') as file:
        json.dump(asset, file, separators=(',', ':'))
    output.with_suffix('.report.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('compliance_m_per_n', type=float)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    assert args.compliance_m_per_n > 0
    result = soften(args.source, args.compliance_m_per_n, args.output)
    print(json.dumps({r['name']: (len(r['compliant_rim_nodes']), r['remaining_fixed']) for r in result['regions']}))
