"""Compare an unchanged kernel with a translated-coordinate solve; no tolerance changes."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess

BASE = Path(__file__).resolve().parent
ROOT = BASE.parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('label')
parser.add_argument('--oblique', action='store_true')
args = parser.parse_args()
assert args.label.replace('-', '').isalnum()
HERE = BASE/args.label
HERE.mkdir(exist_ok=False)
wrapper = '''    pub(super) fn solve_implicit(
        &self, x: &mut Vec<DVec3>, predicted: &[DVec3], frame: Frame,
        dt: f64, contacts: &[PlaneContact], damping: &Dissipation,
    ) -> Result<(), TissueError> {
        let origin = frame.translation;
        let mut local: Vec<_> = x.iter().map(|p| *p - origin).collect();
        let target: Vec<_> = predicted.iter().map(|p| *p - origin).collect();
        let planes: Vec<_> = contacts.iter().map(|c| {
            let mut shifted = *c;
            let weight: f64 = c.point.barycentric.iter().map(|w| w*c.point.influence).sum();
            shifted.point.offset += origin * (weight - 1.0);
            shifted.surface_point -= origin;
            shifted
        }).collect();
        let local_frame = Frame { translation: DVec3::ZERO, ..frame };
        self.solve_implicit_local(&mut local, &target, local_frame, dt, &planes, damping)?;
        for (i, point) in local.into_iter().enumerate() {
            if self.inverse_mass[i] > 0.0 { x[i] = point + origin; }
        }
        Ok(())
    }

'''
manifest = {'production_commit': subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
            'oblique':args.oblique, 'experiment':'Only translate the solve coordinates; unchanged physical equations and convergence limits.', 'runs':[]}
for label in ['baseline', 'centered']:
    folder = HERE/label
    folder.mkdir(exist_ok=False)
    shutil.copytree(BASE/'baseline/soft_tissue', folder/'soft_tissue')
    if label == 'centered':
        p=folder/'soft_tissue/implicit.rs';s=p.read_text();needle='    pub(super) fn solve_implicit('
        assert s.count(needle)==1
        s=s.replace(needle,wrapper+'    fn solve_implicit_local(',1);p.write_text(s)
    source=(BASE/'probe.rs').read_text().replace('SOFT_TISSUE_PATH',str(folder/'soft_tissue/mod.rs'))
    if args.oblique:
        source=source.replace('let contact = PlaneContact {', 'let normal = DVec3::new(0.18800741146825625, -0.46979147680973504, -0.862524887495969).normalize();\n                let contact = PlaneContact {')
        source=source.replace('initial - DVec3::Z * compression', 'initial + normal * compression').replace('normal: DVec3::NEG_Z', 'normal')
    (folder/'probe.rs').write_text(source)
    command=['rustc','--edition=2021','-C','opt-level=3','-L',f'dependency={ROOT}/target/debug/deps',str(folder/'probe.rs'),'-o',str(folder/'probe')]
    for name in ['bevy','serde','serde_json','thiserror','anyhow','tracing']:
        libs=list((ROOT/'target/debug/deps').glob(f'lib{name}-*.rlib'));assert len(libs)==1,(name,libs)
        command += ['--extern',f'{name}={libs[0]}']
    record={'label':label,'argv':command,'sources':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in folder.rglob('*.rs')}}
    manifest['runs'].append(record);(HERE/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    with (folder/'build.log').open('x') as log:
        built=subprocess.run(command,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,env={**os.environ,'CARGO_MANIFEST_DIR':str(ROOT)})
    record['build_exit']=built.returncode;built.check_returncode()
    with (folder/'results.ndjson').open('x') as out:
        ran=subprocess.run([str(folder/'probe')],cwd=ROOT,stdout=out,check=True)
    record['binary_sha256']=hashlib.sha256((folder/'probe').read_bytes()).hexdigest()
    (HERE/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    rows=[json.loads(line) for line in (folder/'results.ndjson').read_text().splitlines()]
    print(json.dumps({'label':label,'cases':len(rows),'failures':sum(r['error'] is not None for r in rows)}),flush=True)
