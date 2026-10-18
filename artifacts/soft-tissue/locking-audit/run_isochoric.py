"""Refit and render the pressure-discretization comparison; preserve every attempt."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
os.chdir(ROOT)

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

manifest = {'purpose': 'Average nodal pressure with isochoric shear at unchanged mesh, material constants and ARDY source motion', 'base_commit': subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()}
source = [*Path('src/cc_render/soft_tissue').glob('*.rs'),Path('examples/tissue_fit.rs'),Path('examples/tissue_lab.rs')]
manifest['sources'] = {str(p):digest(p) for p in source}
with tarfile.open(HERE/'nodal-isochoric-source.tar.gz','x:gz') as archive:
    for p in source:archive.add(p,arcname=str(p))
asset = Path('assets/tissue/beautiful_assistant.base-nodal-isochoric-volume-prototype.json')
fit = ['target/debug/examples/tissue_fit',str(HERE/'nodal-unfitted.json'),str(asset)]
manifest['fit_argv'] = fit
manifest['unfitted_sha256'] = digest(HERE/'nodal-unfitted.json')
manifest['fit_binary_sha256'] = digest(Path(fit[0]))
with (HERE/'fit-isochoric-01.log').open('x') as log:
    result = subprocess.run(fit,stdout=log,stderr=subprocess.STDOUT)
manifest['fit_exit_code'] = result.returncode
(HERE/'launch-isochoric-01.json').write_text(json.dumps(manifest,indent=2)+'\n')
result.check_returncode()
output = Path('artifacts/soft-tissue/lab-nodal-isochoric-front-01')
command = ['target/debug/examples/tissue_lab','../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb','../remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json','artifacts/soft-tissue/lab-jump-01/packets.ndjson',str(asset),'front',str(output)]
env = {'CC_RENDER_FAST':'0','CC_RENDER_CLOCK_TRACE':'0','CC_AVATAR_TISSUE_HZ':'240','CC_AVATAR_TISSUE_ITERATIONS':'32','CC_TISSUE_LAB_FPS':'60','CC_TISSUE_LAB_CLOCK':'fixed','CC_TISSUE_LAB_TRACE_ONLY':'0','RUST_LOG':'info','CC_AVATAR_TISSUE_SNAPSHOT':str(HERE/'nodal-isochoric-front-01-geometry.json'),'CC_AVATAR_TISSUE_SNAPSHOT_PTS_MS':'7633'}
manifest.update(argv=command,environment=env,binary_sha256=digest(Path(command[0])),asset_sha256=digest(asset),started_unix=time.time())
(HERE/'launch-isochoric-01.json').write_text(json.dumps(manifest,indent=2)+'\n')
with (HERE/'front-isochoric-01.log').open('x') as log:
    result = subprocess.run(command,env={**os.environ,**env},stdout=log,stderr=subprocess.STDOUT)
manifest.update(exit_code=result.returncode,completed_unix=time.time())
(HERE/'launch-isochoric-01.json').write_text(json.dumps(manifest,indent=2)+'\n')
result.check_returncode()
print(json.dumps({'output':str(output),'exit_code':result.returncode}))
