"""Build the existing offline fitter against the current small tissue module."""
import hashlib
import json
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
source = ROOT / 'src/cc_render/soft_tissue'
files = [*source.glob('*.rs'), ROOT / 'examples/tissue_fit.rs']
manifest = {'files': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}}
with tempfile.TemporaryDirectory(prefix='tissue-surface-fit-') as temporary:
    scratch = Path(temporary)
    shutil.copytree(source, scratch / 'soft_tissue')
    text = (ROOT / 'examples/tissue_fit.rs').read_text().replace('//! Offline', '// Offline').replace('use cc_render_plugin::cc_render::soft_tissue::', 'use soft_tissue::')
    (scratch / 'fit.rs').write_text('#[path="soft_tissue/mod.rs"] mod soft_tissue;\n' + text)
    with tarfile.open(HERE / 'skin-v2-fit-current-source.tar.gz', 'w:gz') as archive:
        archive.add(scratch / 'soft_tissue', arcname='soft_tissue')
        archive.add(scratch / 'fit.rs', arcname='fit.rs')
    deps = ROOT / 'target/debug/deps'
    command = ['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}', str(scratch / 'fit.rs'), '-o', str(scratch / 'fit')]
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror', 'tracing']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        assert len(paths) == 1, (name, paths)
        command += ['--extern', f'{name}={paths[0]}']
    manifest['build_argv'] = command
    (HERE / 'skin-v2-fit-current-inputs.json').write_text(json.dumps(manifest, indent=2) + '\n')
    with (HERE / 'skin-v2-fit-current-build.log').open('x') as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    with (HERE / 'skin-v2-fit-current.log').open('x') as log:
        result = subprocess.run([str(scratch / 'fit'), str(HERE / 'skin-v2-unfitted.json'), str(HERE / 'skin-v2-fitted.json')], stdout=log, stderr=subprocess.STDOUT)
    manifest['run_exit_code'] = result.returncode
    (HERE / 'skin-v2-fit-current-inputs.json').write_text(json.dumps(manifest, indent=2) + '\n')
    result.check_returncode()
original = json.loads((ROOT / 'assets/tissue/beautiful_assistant.base-viscous-prototype.json').read_text())
fitted = json.loads((HERE / 'skin-v2-fitted.json').read_text())
for current, old in zip(fitted['regions'], original['regions'], strict=True):
    current['material'] = old['material']
output = ROOT / 'assets/tissue/beautiful_assistant.base-skin-volume-v2-prototype.json'
with output.open('x') as file:
    json.dump(fitted, file, separators=(',', ':'))
print('Fitted candidate:', output)
