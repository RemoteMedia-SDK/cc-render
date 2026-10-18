"""Rebuild both kernels from the preserved reference and current worktree source."""
from pathlib import Path
import subprocess
import tarfile
import tempfile

here = Path(__file__).resolve().parent
root = here.parents[2]
deps = root / 'target/debug/deps'
with tempfile.TemporaryDirectory(prefix='tissue-comparison-') as scratch:
    scratch = Path(scratch)
    with tarfile.open(here / 'reference-source.tar.gz') as archive:
        archive.extractall(scratch, filter='data')
    source = (here / 'compare.rs').read_text()
    source = source.replace('/tmp/cc-tissue-reference-source/mod.rs', str(scratch / 'reference/mod.rs'))
    source = source.replace('/home/acidhax/dev/personal/remotemedia/cc-render-soft-tissue/src/cc_render/soft_tissue/mod.rs', str(root / 'src/cc_render/soft_tissue/mod.rs'))
    (scratch / 'compare.rs').write_text(source)
    cmd = ['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}', str(scratch / 'compare.rs'), '-o', str(scratch / 'compare')]
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        if len(paths) != 1:
            raise RuntimeError(f'Expected one built dependency for {name}; found {paths}')
        cmd.extend(['--extern', f'{name}={paths[0]}'])
    subprocess.run(cmd, check=True)
    subprocess.run([str(scratch / 'compare')], cwd=root, check=True)
