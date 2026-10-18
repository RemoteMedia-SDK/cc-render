from pathlib import Path
import hashlib,json,os,subprocess,time
root=Path(__file__).resolve().parents[3]
os.chdir(root)
here=Path(__file__).resolve().parent
m=json.loads((here/'launch.json').read_text())
assert m['exit_code']==-15
for p,h in m['sources'].items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h
assert hashlib.sha256(Path(m['argv'][0]).read_bytes()).hexdigest()==m['binary_sha256']
m.pop('exit_code');m.pop('completed_unix')
m['argv'][-1]='artifacts/soft-tissue/lab-nodal-volume-front-02'
m['environment']['CC_AVATAR_TISSUE_SNAPSHOT']=str(here/'nodal-front-02-geometry.json')
m['started_unix']=time.time()
m['retry_reason']='Host GPU access after sandbox software-rendering failure'
(here/'launch-02.json').write_text(json.dumps(m,indent=2)+'\n')
with (here/'front-02.log').open('x') as log:
 p=subprocess.run(m['argv'],env={**os.environ,**m['environment']},stdout=log,stderr=subprocess.STDOUT)
m.update(exit_code=p.returncode,completed_unix=time.time())
(here/'launch-02.json').write_text(json.dumps(m,indent=2)+'\n')
p.check_returncode()
print(m['argv'][-1])
