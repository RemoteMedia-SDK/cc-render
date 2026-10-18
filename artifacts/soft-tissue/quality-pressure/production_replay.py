"""Verify the integrated candidate against the archived prototype and at three rates."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
OUT = HERE/'production-replay-01'
OUT.mkdir(exist_ok=False)
ASSET = ROOT/'assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json'
TRACE = HERE.parent/'lab-boundary-nodal-front-01/tissue.ndjson'
manifest = {'base_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            'inputs': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                       for p in [ASSET, TRACE, Path(__file__).resolve(), *sorted((ROOT/'src/cc_render/soft_tissue').glob('*.rs'))]},
            'runs': []}
with tempfile.TemporaryDirectory(prefix='tissue-quality-validation-', dir=ROOT/'target') as temporary:
    scratch = Path(temporary)
    shutil.copytree(ROOT/'src/cc_render/soft_tissue', scratch/'soft_tissue')
    replay = (HERE.parent/'solver-runtime/replay.rs').read_text()
    marker = '    soft_tissue::solver::implicit::profile();\n'
    assert replay.count(marker) == 1
    (scratch/'replay.rs').write_text('#[path="soft_tissue/mod.rs"] mod soft_tissue;\n'+replay.replace(marker, ''))
    with tarfile.open(OUT/'source.tar.gz', 'x:gz') as archive:
        archive.add(scratch/'soft_tissue', arcname='soft_tissue')
        archive.add(scratch/'replay.rs', arcname='replay.rs')
    deps = ROOT/'target/debug/deps'
    command = ['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}',
               str(scratch/'replay.rs'), '-o', str(scratch/'replay')]
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror', 'tracing']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        assert len(paths) == 1, (name, paths)
        command += ['--extern', f'{name}={paths[0]}']
    with (OUT/'build.log').open('x') as log:
        subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    manifest['build_argv'] = command
    for rate in [240, 120, 480]:
        command = [str(scratch/'replay'), str(ASSET), str(TRACE), str(OUT/f'positions-{rate}.bin'), str(rate), '50']
        run = {'argv': command, 'started_unix': time.time()}
        manifest['runs'].append(run)
        (OUT/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
        with (OUT/f'run-{rate}.json').open('x') as output, (OUT/f'run-{rate}.stderr').open('x') as errors:
            result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=errors)
        run.update(exit_code=result.returncode, completed_unix=time.time())
        (OUT/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
        result.check_returncode()
        print(rate, result.returncode, flush=True)
        if rate == 240:
            prototype = HERE/'combined-01/case-combined-8000-positions.bin'
            assert (OUT/'positions-240.bin').read_bytes() == prototype.read_bytes()
            print('All 240 Hz position bytes match the prototype.', flush=True)
