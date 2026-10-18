"""Bounded pressure-share study with refitting and unchanged attachment replay."""
from pathlib import Path
import copy
import argparse
import hashlib
import json
import shutil
import subprocess
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
ASSET = ROOT / 'assets/tissue/beautiful_assistant.base-boundary-nodal-volume-prototype.json'
TRACE = ROOT / 'artifacts/soft-tissue/lab-boundary-nodal-front-01/tissue.ndjson'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, default=HERE/'study-01')
parser.add_argument('--source-archive', type=Path, help='Reuse the preserved experimental kernel after it was removed from production.')
parser.add_argument('--shares', type=int, nargs='+', default=[0, 500, 2000, 8000])
args = parser.parse_args()
assert args.shares[0] == 0 and len(set(args.shares)) == len(args.shares)
assert all(0 <= share <= 500000 for share in args.shares)
OUT = args.output.resolve()
OUT.mkdir(exist_ok=False)
plan = {
    'baseline_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
    'shares_pa': args.shares,
    'rate_hz': 240,
    'source_archive': str(args.source_archive) if args.source_archive else None,
    'criteria': [
        'Keep the source motion, attachments, topology, surface embedding, shear material and viscosity unchanged.',
        'Refit each nonzero share to the existing 0.25 mm authored equilibrium-shape criterion.',
        'Reduce local volume-ratio extremes into [0.5, 2.0] and keep total-volume change below 1 percent.',
        'Retain at least 80 percent of baseline peak and time-RMS embedded-surface displacement.',
        'A passing CPU screen requires a Bevy capture and visual review before replacing the reviewed asset.',
    ],
    'limits': 'A numerical stabilization experiment, not anatomically measured material. Recorded attachment interpolation is not GPU or contact acceptance.',
    'inputs': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
               for p in [ASSET, TRACE, Path(__file__).resolve(), *sorted((ROOT/'src/cc_render/soft_tissue').glob('*.rs'))]},
    'runs': [],
}


def save():
    (OUT / 'manifest.json').write_text(json.dumps(plan, indent=2) + '\n')


def run(command, name, stdout_name=None):
    record = {'argv': list(map(str, command)), 'started_unix': time.time()}
    plan['runs'].append(record)
    save()
    with (OUT / (stdout_name or name + '.log')).open('x') as output, (OUT / (name + '.stderr')).open('x') as errors:
        result = subprocess.run(record['argv'], cwd=ROOT, stdout=output, stderr=errors)
    record.update(exit_code=result.returncode, completed_unix=time.time())
    save()
    print(name, result.returncode, flush=True)
    result.check_returncode()


save()
with tempfile.TemporaryDirectory(prefix='tissue-pressure-', dir=ROOT/'target') as temporary:
    scratch = Path(temporary)
    if args.source_archive:
        plan['source_archive_sha256'] = hashlib.sha256(args.source_archive.read_bytes()).hexdigest()
        with tarfile.open(args.source_archive, 'r:gz') as archive:
            for member in archive.getmembers():
                path = Path(member.name)
                if not member.isfile():
                    continue
                assert (path.parts[0] == 'soft_tissue' and path.suffix in ['.rs', '.json']) or path.name in ['fit.rs', 'replay.rs']
                assert not path.is_absolute() and '..' not in path.parts
                (scratch/path).parent.mkdir(parents=True, exist_ok=True)
                (scratch/path).write_bytes(archive.extractfile(member).read())
    else:
        shutil.copytree(ROOT/'src/cc_render/soft_tissue', scratch/'soft_tissue')
        fit = (ROOT/'examples/tissue_fit.rs').read_text().replace('//! Offline', '// Offline')
        fit = fit.replace('use cc_render_plugin::cc_render::soft_tissue::', 'use soft_tissue::')
        replay = (HERE.parent/'solver-runtime/replay.rs').read_text()
        marker = '    soft_tissue::solver::implicit::profile();\n'
        assert replay.count(marker) == 1
        replay = replay.replace(marker, '')
        for name, code in [('fit', fit), ('replay', replay)]:
            (scratch/(name+'.rs')).write_text('#[path="soft_tissue/mod.rs"] mod soft_tissue;\n'+code)
    with tarfile.open(OUT/'source.tar.gz', 'x:gz') as archive:
        archive.add(scratch/'soft_tissue', arcname='soft_tissue')
        for name in ['fit', 'replay']:
            archive.add(scratch/(name+'.rs'), arcname=name+'.rs')
    deps = ROOT/'target/debug/deps'
    externs = []
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror', 'tracing']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        assert len(paths) == 1, (name, paths)
        externs += ['--extern', f'{name}={paths[0]}']
    for name in ['fit', 'replay']:
        run(['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}',
             scratch/(name+'.rs'), '-o', scratch/name, *externs], name+'-build')
    original = json.loads(ASSET.read_text())
    for share in plan['shares_pa']:
        candidate = copy.deepcopy(original)
        for region in candidate['regions']:
            region['material']['pressure_stabilization_pa'] = share
        unfitted = OUT/f'share-{share}-input.json'
        unfitted.write_text(json.dumps(candidate, separators=(',', ':'))+'\n')
        fitted = OUT/f'share-{share}-fitted.json'
        if share:
            run([scratch/'fit', unfitted, fitted], f'share-{share}-fit')
        else:
            shutil.copyfile(unfitted, fitted)
        run([scratch/'replay', fitted, TRACE, OUT/f'share-{share}-positions.bin', '240', '50'],
            f'share-{share}-replay', f'share-{share}-run.json')
