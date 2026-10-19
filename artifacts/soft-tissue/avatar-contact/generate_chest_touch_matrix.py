"""Preregistered semantic chest-touch matrix after seed 739 wedged a hand
between both breasts. Every attempt is kept and scored with the same criteria;
nothing is modified. Selection among scored attempts is reported with its rate.

    python generate_chest_touch_matrix.py generate   # ARDY on GPU 1, ~1 min each
    python generate_chest_touch_matrix.py evaluate   # tissue lab per attempt
"""
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
OUT = Path(__file__).resolve().parent/'chest-matrix-01'
DESCRIPTIONS = {
    'rest': 'A person rests both hands flat on their chest.',
    'gentle': 'A person gently rubs their chest with both hands.',
    'rub': 'A person rubs their chest with both hands.',
}
SEEDS = [740, 741, 742, 743]
INITIAL = 'A person stands calmly.'
CRITERIA = {
    'completes': 'tissue replay reaches 8000 ms',
    'contact': 'at least one frame with active tissue contact',
    'ribcage_supported': 'no unsupported ribcage-clamp frames',
    'rest_supported': 'no unsupported breast-skin rest frames (corrections <= 50 mm)',
    'fingers_rest': 'finger residual <= 1 mm in every frame',
    'rank': 'passing attempts ordered by smallest maximum rest correction',
}
CASES = [(name, seed) for name in DESCRIPTIONS for seed in SEEDS]


def lab(name, seed):
    return ROOT/f'artifacts/soft-tissue/lab-chest-{name}-{seed}-01'


def generate():
    OUT.mkdir(exist_ok=False)
    sources = [SDK/'scripts/avatars'/n for n in
               ['ardy_worker.py', 'motion_backend.py', 'motion_service.py', 'motion_policy.py', 'ardy_adapter.py']]
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
    manifest = {'descriptions': DESCRIPTIONS, 'seeds': SEEDS, 'initial': INITIAL, 'criteria': CRITERIA,
                'sources': hashes, 'environment': environment,
                'policy': 'Preregistered matrix; one attempt per cell; semantic only; every attempt kept.',
                'runs': []}
    (OUT/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    for name, seed in CASES:
        command = [str(SDK/'.runtime/office/venv/bin/python'), str(ROOT/'scripts/avatars/record_tissue_jump.py'),
                   str(lab(name, seed)), '--description', DESCRIPTIONS[name], '--seed', str(seed),
                   '--session-id', f'tissue-lab-chest-{name}-{seed}', '--initial-description', INITIAL]
        run = {'case': [name, seed], 'argv': command, 'started_unix': time.time()}
        manifest['runs'].append(run)
        with (OUT/f'{name}-{seed}.log').open('x') as log:
            result = subprocess.run(command, cwd=ROOT, env={**os.environ, **environment},
                                    stdout=log, stderr=subprocess.STDOUT)
        run.update(exit_code=result.returncode, completed_unix=time.time(),
                   sources_unchanged=all(hashlib.sha256(p.read_bytes()).hexdigest() == hashes[str(p)] for p in sources))
        if result.returncode == 0:
            folder = lab(name, seed)
            responses = [json.loads(s) for s in (folder/'responses.ndjson').read_text().splitlines()]
            packets = [json.loads(s) for s in (folder/'packets.ndjson').read_text().splitlines()]
            run['packets_unchanged'] = packets == [f for r in responses for f in r['frames']]
            run['audit_clean'] = all(r['audit'] == dict(policy='model-owned-v1', future_nonzero=0,
                                                        future_channels=[], exceptions=[]) for r in responses)
            run['packets_sha256'] = hashlib.sha256((folder/'packets.ndjson').read_bytes()).hexdigest()
        (OUT/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
        print(name, seed, result.returncode, flush=True)


def evaluate():
    manifest = json.loads((OUT/'manifest.json').read_text())
    results = []
    for run in manifest['runs']:
        name, seed = run['case']
        row = {'case': f'{name}-{seed}', 'generated': run['exit_code'] == 0 and run.get('audit_clean', False)}
        if row['generated']:
            label = f'eval-{name}-{seed}'
            out = OUT/label
            env = {**os.environ, 'CC_AVATAR_HAND_RIBCAGE_CLAMP': '1'}
            subprocess.run(['python', str(ROOT/'artifacts/soft-tissue/quality-pressure/run_bevy.py'), '--fixed-only',
                            '--trace-only', '--label', label, '--output', str(out),
                            '--packets', str(lab(name, seed)/'packets.ndjson')],
                           cwd=ROOT, env=env, stdout=(OUT/f'{label}.driver.log').open('x'), stderr=subprocess.STDOUT)
            trace = ROOT/f'artifacts/soft-tissue/lab-{label}-front-01/tissue.ndjson'
            rows = [json.loads(s) for s in trace.read_text().splitlines()] if trace.exists() else []
            report = lambda r, k: r['integration'].get(k) or [{}, {}]
            row.update(
                last_pts_ms=rows[-1]['pts_ms'] if rows else 0,
                contact_frames=sum(1 for r in rows if r['integration']['active_contact_samples']),
                ribcage_unsupported=sum(any(c.get('unsupported') for c in report(r, 'hand_clamp')) for r in rows),
                rest_unsupported=sum(any(c.get('unsupported') for c in report(r, 'hand_rest')) for r in rows),
                max_rest_correction_mm=round(max([c.get('correction_m', 0) for r in rows for c in report(r, 'hand_rest')] or [0]) * 1e3, 1),
                max_palm_penetration_mm=round(max([c.get('penetration_m', 0) for r in rows for c in report(r, 'hand_rest')] or [0]) * 1e3, 1),
                max_finger_residual_mm=round(max([c.get('finger_residual_m', 0) for r in rows for c in report(r, 'hand_rest')] or [0]) * 1e3, 2),
            )
            row['passes'] = (row['last_pts_ms'] >= 8000 and row['contact_frames'] > 0 and row['ribcage_unsupported'] == 0
                             and row['rest_unsupported'] == 0 and row['max_finger_residual_mm'] <= 1.0)
        results.append(row)
        print(json.dumps(row), flush=True)
    passing = sorted((r for r in results if r.get('passes')), key=lambda r: r['max_rest_correction_mm'])
    (OUT/'evaluation.json').write_text(json.dumps({
        'criteria': CRITERIA, 'results': results, 'passing': [r['case'] for r in passing],
        'pass_rate': f"{len(passing)}/{len(results)}"}, indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('step', choices=['generate', 'evaluate'])
    {'generate': generate, 'evaluate': evaluate}[parser.parse_args().step]()
