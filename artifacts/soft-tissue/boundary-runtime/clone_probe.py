"""Measure tissue trial cloning using an archived copy of the current kernel."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
label = sys.argv[1]
assert label.replace('-', '').isalnum()
out = HERE / label
out.mkdir(exist_ok=False)
source = ROOT / 'src/cc_render/soft_tissue'
asset = ROOT / 'assets/tissue/beautiful_assistant.base-boundary-nodal-volume-prototype.json'
probe = HERE / 'clone_probe.rs'
inputs = [asset, probe, Path(__file__).resolve(), *sorted(source.glob('*.rs'))]
(out/'inputs.json').write_text(json.dumps({
    'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
    'files': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in inputs},
}, indent=2)+'\n')
with tempfile.TemporaryDirectory(prefix='tissue-clone-probe-', dir=ROOT/'target') as temporary:
    scratch = Path(temporary)
    shutil.copytree(source, scratch/'soft_tissue')
    solver = scratch/'soft_tissue/solver.rs'
    with solver.open('a') as stream:
        stream.write('''
impl Tissue {
    pub fn probe_without_pressure(&self) -> Self {
        let mut result = self.clone();
        result.pressure_patches = Default::default();
        result
    }
    pub fn probe_pressure_clone(&self) {
        std::hint::black_box(std::hint::black_box(&self.pressure_patches).clone());
    }
}
''')
    (scratch/'probe.rs').write_text('#[path="soft_tissue/mod.rs"] mod soft_tissue;\n'+probe.read_text())
    with tarfile.open(out/'probe-source.tar.gz', 'x:gz') as archive:
        archive.add(scratch/'soft_tissue', arcname='soft_tissue')
        archive.add(scratch/'probe.rs', arcname='probe.rs')
    deps = ROOT/'target/debug/deps'
    command = ['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}',
               str(scratch/'probe.rs'), '-o', str(scratch/'probe')]
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror', 'tracing']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        assert len(paths) == 1, (name, paths)
        command += ['--extern', f'{name}={paths[0]}']
    with (out/'build.log').open('x') as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    with (out/'run.json').open('x') as log, (out/'run.stderr').open('x') as error:
        subprocess.run([str(scratch/'probe'), str(asset)], stdout=log, stderr=error, check=True)
print((out/'run.json').read_text())
