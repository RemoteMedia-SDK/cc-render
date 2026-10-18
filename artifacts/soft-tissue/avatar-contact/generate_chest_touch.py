"""Two declared semantic-only attempts; preserve failures and unchanged packets."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT.parent/'remotemedia-sdk-office-main'
parser = argparse.ArgumentParser(description=__doc__)
mode = parser.add_mutually_exclusive_group()
mode.add_argument('--approach', action='store_true', help='One new approach-to-contact description after auditing the two original attempts.')
mode.add_argument('--continuation', action='store_true', help='Generate standing motion, then a chest-rub continuation from native history.')
args = parser.parse_args()
OUT = Path(__file__).resolve().parent/('chest-continuation-01' if args.continuation else 'chest-approach-01' if args.approach else 'chest-touch-01')
OUT.mkdir(exist_ok=False)
CASES = [(739, 'continuation', 'A person rubs their chest with both hands.')] if args.continuation else [
         (738, 'approach', 'A person raises both hands to their chest and starts rubbing it.')] if args.approach else [
         (736, 'rub', 'A person rubs their chest with both hands.'),
         (737, 'pat', 'A person pats their chest with both hands.')]
sources = [SDK/'scripts/avatars'/name for name in
           ['ardy_worker.py', 'motion_backend.py', 'motion_service.py',
            'motion_policy.py', 'ardy_adapter.py']]
sources += [ROOT/'scripts/avatars/record_tissue_jump.py', Path(__file__).resolve()]
hashes = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
with tarfile.open(OUT/'source.tar.gz', 'x:gz') as archive:
    for p in sources:
        archive.add(p, arcname=str(p.relative_to(ROOT.parent)))
environment = dict(ARDY_MODEL='core40', CUDA_VISIBLE_DEVICES='1', ARDY_GPU='1',
                   TEXT_ENCODER_DEVICE='cuda:0', ARDY_DEVICE='cuda:0',
                   TEXT_ENCODER_MODE='local', LLM2VEC_QUANTIZE='nf4',
                   HF_HOME=str(SDK/'.runtime/office/hf'), HF_HUB_OFFLINE='1',
                   TRANSFORMERS_OFFLINE='1', PYTHONPATH=str(SDK/'scripts/avatars'))
manifest = {'cases': CASES, 'sources': hashes, 'environment': environment,
            'policy': 'One eight-second attempt per semantic description. No pose modification or seed selection.',
            'runs': []}
for seed, name, description in CASES:
    folder = ROOT/f'artifacts/soft-tissue/lab-chest-{name}-{seed}-01'
    command = [str(SDK/'.runtime/office/venv/bin/python'),
               str(ROOT/'scripts/avatars/record_tissue_jump.py'), str(folder),
               '--description', description, '--seed', str(seed),
               '--session-id', f'tissue-lab-chest-{name}-{seed}']
    if args.continuation:
        command += ['--initial-description', 'A person stands calmly.']
    run = {'argv': command, 'started_unix': time.time()}
    manifest['runs'].append(run)
    (OUT/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    with (OUT/f'{name}.log').open('x') as log:
        result = subprocess.run(command, cwd=ROOT, env={**os.environ, **environment},
                                stdout=log, stderr=subprocess.STDOUT)
    run.update(exit_code=result.returncode, completed_unix=time.time())
    run['sources_unchanged'] = all(hashlib.sha256(p.read_bytes()).hexdigest()==hashes[str(p)] for p in sources)
    (OUT/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    print(name, result.returncode, flush=True)
    result.check_returncode()
    assert run['sources_unchanged']
    responses = [json.loads(s) for s in (folder/'responses.ndjson').read_text().splitlines()]
    packets = [json.loads(s) for s in (folder/'packets.ndjson').read_text().splitlines()]
    assert packets == [f for r in responses for f in r['frames']]
    assert all(r['audit']==dict(policy='model-owned-v1', future_nonzero=0, future_channels=[], exceptions=[]) for r in responses)
    (folder/'source-audit.json').write_text(json.dumps({
        'packets': len(packets), 'source_packets_unchanged': True,
        'future_channels': [], 'exceptions': [], 'sources_unchanged': True,
        'packets_sha256': hashlib.sha256((folder/'packets.ndjson').read_bytes()).hexdigest(),
    }, indent=2)+'\n')
