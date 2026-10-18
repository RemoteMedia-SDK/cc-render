"""Read-only nodal-state capture at the same source times as the soft material render."""
from pathlib import Path
import hashlib,json,os,subprocess,tarfile,time
HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[2]
os.chdir(ROOT)
m=json.loads((HERE.parent/'locking-audit/launch-soft-isochoric-01.json').read_text())
assert m['exit_code']==0
for p,h in m['sources'].items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h
m['sources']['src/cc_render/bevy_app/tissue_trace.rs']=hashlib.sha256(Path('src/cc_render/bevy_app/tissue_trace.rs').read_bytes()).hexdigest()
m['purpose']='Read-only dynamic node/volume audit; unchanged soft nodal material and source motion'
m['argv'][-1]='artifacts/soft-tissue/lab-soft-volume-nodes-01'
m['environment']['CC_TISSUE_LAB_TRACE_ONLY']='1'
m['environment']['CC_AVATAR_TISSUE_TRACE_NODES']='1'
for k in ['CC_AVATAR_TISSUE_SNAPSHOT','CC_AVATAR_TISSUE_SNAPSHOT_PTS_MS']:m['environment'].pop(k,None)
m['binary_sha256']=hashlib.sha256(Path(m['argv'][0]).read_bytes()).hexdigest()
m.pop('exit_code');m.pop('completed_unix');m['started_unix']=time.time()
with tarfile.open(HERE/'source.tar.gz','x:gz') as archive:
 for p in m['sources']:archive.add(p,arcname=p)
(HERE/'launch.json').write_text(json.dumps(m,indent=2)+'\n')
with (HERE/'capture.log').open('x') as log:
 p=subprocess.run(m['argv'],env={**os.environ,**m['environment']},stdout=log,stderr=subprocess.STDOUT)
m.update(exit_code=p.returncode,completed_unix=time.time())
(HERE/'launch.json').write_text(json.dumps(m,indent=2)+'\n')
p.check_returncode()
