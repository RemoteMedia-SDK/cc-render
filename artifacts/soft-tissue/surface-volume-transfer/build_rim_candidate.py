"""Prototype a chest-attached volume boundary without editing source animation."""
import copy
import hashlib
import json
import sys
from pathlib import Path

import numpy as np
from scipy.spatial import ConvexHull

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / 'scripts/avatars'))
from build_tissue_asset import anchor_space, surfaces

source = ROOT / 'assets/tissue/beautiful_assistant.base-geometric-transfer-prototype.json'
original = json.loads(source.read_text())
candidate = copy.deepcopy(original)
body = next(s for s in surfaces(ROOT / '../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb') if s['name'] == 'CC_Base_Body')
report = {'source_asset_sha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'regions': []}
for region in candidate['regions']:
    points = anchor_space(np.asarray(region['initial_positions']), np.linalg.inv(body['anchor_inverse']))
    # This candidate deliberately retains the existing four-layer 64-point cage.
    assert len(points) == 256
    np.testing.assert_allclose(points[:, [0, 2]].reshape(4, 64, 2), np.broadcast_to(points[:64, [0, 2]], (4, 64, 2)), atol=1e-10)
    rim = set(map(int, ConvexHull(points[:64, [0, 2]]).vertices))
    region['attachments'] = [{'kind': 'fixed' if i < 64 or i % 64 in rim else 'free'} for i in range(256)]
    region['rest_positions'] = copy.deepcopy(region['initial_positions'])
    report['regions'].append({'name': region['name'], 'rim_columns': sorted(rim), 'fixed_nodes': sum(a['kind'] == 'fixed' for a in region['attachments'])})
output = HERE / 'rim-unfitted.json'
with output.open('x') as f:
    json.dump(candidate, f, separators=(',', ':'))
report['unfitted_asset_sha256'] = hashlib.sha256(output.read_bytes()).hexdigest()
report['fit_binary_sha256'] = hashlib.sha256((ROOT / 'target/debug/examples/tissue_fit').read_bytes()).hexdigest()
(HERE / 'rim-authoring.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
