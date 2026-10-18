"""Sequential optimized-kernel parity, timestep and live-presentation checks."""
from pathlib import Path
import hashlib,json,os,subprocess,tarfile,time
HERE=Path(__file__).resolve().parent;ROOT=HERE.parents[2];os.chdir(ROOT)
base=json.loads((HERE/'launch-boundary-01.json').read_text());assert base['exit_code']==0
for p,h in base['sources'].items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h
base['sources']['Cargo.toml']=hashlib.sha256(Path('Cargo.toml').read_bytes()).hexdigest()
with tarfile.open(HERE/'optimized-source.tar.gz','x:gz') as archive:
 for p in base['sources']:archive.add(p,arcname=p)
results=[]
for name,rate,fps,clock in [('boundary-opt240',240,60,'fixed'),('boundary-opt120',120,60,'fixed'),('boundary-live120',120,30,'live')]:
 m=json.loads(json.dumps(base));m['name']=name;m['argv'][-1]='artifacts/soft-tissue/lab-'+name+'-01'
 env=m['environment'];env.update(CC_AVATAR_TISSUE_HZ=str(rate),CC_TISSUE_LAB_FPS=str(fps),CC_TISSUE_LAB_CLOCK=clock,CC_TISSUE_LAB_TRACE_ONLY='1',CC_AVATAR_TISSUE_TRACE_NODES='1' if clock=='fixed' else '0')
 for k in ['CC_AVATAR_TISSUE_SNAPSHOT','CC_AVATAR_TISSUE_SNAPSHOT_PTS_MS']:env.pop(k,None)
 m['binary_sha256']=hashlib.sha256(Path(m['argv'][0]).read_bytes()).hexdigest()
 m.pop('exit_code');m.pop('completed_unix');m['started_unix']=time.time()
 results.append(m);(HERE/'rate-launches.json').write_text(json.dumps(results,indent=2)+'\n')
 with (HERE/(name+'.log')).open('x') as log:
  p=subprocess.run(m['argv'],env={**os.environ,**env},stdout=log,stderr=subprocess.STDOUT)
 m.update(exit_code=p.returncode,completed_unix=time.time());(HERE/'rate-launches.json').write_text(json.dumps(results,indent=2)+'\n')
 print(name,p.returncode,flush=True)
 p.check_returncode()
