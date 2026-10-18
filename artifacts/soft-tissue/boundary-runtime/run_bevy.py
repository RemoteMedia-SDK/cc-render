"""Check the optimized solver against the reviewed Bevy capture, then live timing."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import tarfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
os.chdir(ROOT)
baseline = json.loads((HERE.parent/'volume-audit/launch-boundary-01.json').read_text())
asset = Path(baseline['argv'][4])
assert hashlib.sha256(asset.read_bytes()).hexdigest() == baseline['asset_sha256']
sources = sorted(set(baseline['sources']) | {'Cargo.toml'}
                 | {str(p) for p in Path('src/cc_render/bevy_app').glob('tissue*.rs')})
hashes = {p: hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in sources}
with tarfile.open(HERE/'bevy-source.tar.gz', 'x:gz') as archive:
    for path in sources:
        archive.add(path, arcname=path)
runs = []
for name, rate, fps, clock in [('boundary-cache240', 240, 60, 'fixed'),
                                ('boundary-cache-live120', 120, 30, 'live')]:
    args = baseline['argv'].copy()
    args[-1] = 'artifacts/soft-tissue/lab-'+name+'-01'
    assert not Path(args[-1]).exists()
    environment = baseline['environment'].copy()
    for key in ['CC_AVATAR_TISSUE_SNAPSHOT', 'CC_AVATAR_TISSUE_SNAPSHOT_PTS_MS']:
        environment.pop(key, None)
    environment.update(CC_AVATAR_TISSUE_HZ=str(rate), CC_TISSUE_LAB_FPS=str(fps),
                       CC_TISSUE_LAB_CLOCK=clock, CC_TISSUE_LAB_TRACE_ONLY='1',
                       CC_AVATAR_TISSUE_TRACE_NODES='1' if clock == 'fixed' else '0')
    run = {'name': name, 'argv': args, 'environment': environment, 'sources': hashes,
           'binary_sha256': hashlib.sha256(Path(args[0]).read_bytes()).hexdigest(),
           'asset_sha256': baseline['asset_sha256'], 'started_unix': time.time()}
    runs.append(run)
    (HERE/'bevy-launches.json').write_text(json.dumps(runs, indent=2)+'\n')
    with (HERE/(name+'.log')).open('x') as log:
        result = subprocess.run(args, env={**os.environ, **environment}, stdout=log,
                                stderr=subprocess.STDOUT)
    run.update(exit_code=result.returncode, completed_unix=time.time())
    (HERE/'bevy-launches.json').write_text(json.dumps(runs, indent=2)+'\n')
    print(name, result.returncode, flush=True)
    result.check_returncode()
