"""Capture the integrated pressure candidate with unchanged reviewed motion/camera."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--timing-only', action='store_true')
parser.add_argument('--label', default='quality')
parser.add_argument('--output', type=Path)
parser.add_argument('--trace-only', action='store_true', help='Omit fixed-clock PNGs; retain all node traces.')
args = parser.parse_args()
assert args.label.replace('-', '').isalnum()
OUT = args.output.resolve() if args.output else HERE/('bevy-timing-01' if args.timing_only else 'bevy-01')
assert OUT.is_relative_to(HERE.parent)
OUT.mkdir(exist_ok=False)
os.chdir(ROOT)
baseline = json.loads((HERE.parent/'volume-audit/launch-boundary-01.json').read_text())
asset = Path('assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json')
sources = sorted(set(baseline['sources']) | {'Cargo.toml', str(Path(__file__).relative_to(ROOT))}
                 | {str(p) for p in Path('src/cc_render/soft_tissue').glob('*.rs')}
                 | {str(p) for p in Path('src/cc_render/bevy_app').glob('tissue*.rs')})
hashes = {p: hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in sources}
with tarfile.open(OUT/'source.tar.gz', 'x:gz') as archive:
    for path in sources:
        archive.add(path, arcname=path)
runs = []
cases = [(args.label+'-live120-timing', 120, 30, 'live')] if args.timing_only else [
    (args.label+'-front', 240, 60, 'fixed'), (args.label+'-live120', 120, 30, 'live')]
for name, rate, fps, clock in cases:
    command = baseline['argv'].copy()
    command[4] = str(asset)
    command[-1] = 'artifacts/soft-tissue/lab-'+name+'-01'
    assert not Path(command[-1]).exists()
    environment = baseline['environment'].copy()
    for key in ['CC_AVATAR_TISSUE_SNAPSHOT', 'CC_AVATAR_TISSUE_SNAPSHOT_PTS_MS']:
        environment.pop(key, None)
    environment.update(CC_AVATAR_TISSUE_HZ=str(rate), CC_TISSUE_LAB_FPS=str(fps),
                       CC_TISSUE_LAB_CLOCK=clock,
                       CC_TISSUE_LAB_TRACE_ONLY='0' if clock=='fixed' and not args.trace_only else '1',
                       CC_AVATAR_TISSUE_TRACE_NODES='0' if args.timing_only or (args.trace_only and clock=='live') else '1')
    if clock == 'fixed':
        environment.update(CC_AVATAR_TISSUE_SNAPSHOT=str(OUT/'geometry.json'),
                           CC_AVATAR_TISSUE_SNAPSHOT_PTS_MS='7633')
    record = {'argv': command, 'environment': environment, 'sources': hashes,
              'binary_sha256': hashlib.sha256(Path(command[0]).read_bytes()).hexdigest(),
              'asset_sha256': hashlib.sha256(asset.read_bytes()).hexdigest(),
              'started_unix': time.time()}
    runs.append(record)
    (OUT/'launches.json').write_text(json.dumps(runs, indent=2)+'\n')
    with (OUT/(name+'.log')).open('x') as log:
        result = subprocess.run(command, env={**os.environ, **environment}, stdout=log, stderr=subprocess.STDOUT)
    record.update(exit_code=result.returncode, completed_unix=time.time())
    (OUT/'launches.json').write_text(json.dumps(runs, indent=2)+'\n')
    print(name, result.returncode, flush=True)
    result.check_returncode()
