"""Isolated source-copy experiment; leaves production solver unchanged."""
from pathlib import Path
import shutil, subprocess, json, tarfile
root=Path.cwd(); folder=root/'artifacts/soft-tissue/convergence'
source=root/'src/cc_render/soft_tissue'
archive=folder/'reference-source.tar.gz'
if not archive.exists():
    with tarfile.open(archive,'w:gz') as tar: tar.add(source,arcname='soft_tissue')
for case,tolerance in [('warm-start-strict-bounded',None)]:
    dest=Path('/tmp')/f'cc-tissue-convergence-{case}'
    shutil.copytree(source,dest,dirs_exist_ok=False)
    p=dest/'implicit.rs';s=p.read_text()
    s=s.replace('use bevy::math::DMat3;', '''use bevy::math::DMat3;
static NONLINEAR: std::sync::Mutex<Vec<Vec<f64>>> = std::sync::Mutex::new(Vec::new());
static LINEAR: std::sync::Mutex<Vec<(usize,f64)>> = std::sync::Mutex::new(Vec::new());
pub fn profile() { eprintln!("{}",serde_json::json!({"nonlinear_force_norms_n":*NONLINEAR.lock().unwrap(),"linear_iterations_and_relative_residual":*LINEAR.lock().unwrap()})); }
''')
    s=s.replace('        for _ in 0..128 {','        let mut iterations=0;\n        for _ in 0..128 {')
    s=s.replace('            let product = self.multiply(&direction);','            iterations+=1;\n            let product = self.multiply(&direction);')
    s=s.replace('        result\n    }\n}', '''        let actual:Vec<_>=self.multiply(&result).into_iter().zip(&self.gradient).map(|(a,b)|a+*b).collect();
        LINEAR.lock().unwrap().push((iterations,(dot(&actual,&actual)/dot(&self.gradient,&self.gradient)).sqrt()));
        result
    }
}''',1)
    prefix,body=s.split('    pub(super) fn solve_implicit(',1)
    body=body.replace('        for _ in 0..self.material.iterations {','        let mut residuals=Vec::new();\n        for _ in 0..self.material.iterations {',1)
    body=body.replace('            if dot(&system.gradient, &system.gradient) < 1e-16 {','            residuals.push(dot(&system.gradient,&system.gradient).sqrt());\n            if dot(&system.gradient, &system.gradient) < 1e-16 {',1)
    if tolerance is not None: body=body.replace('< 1e-16 {',f'< {tolerance} {{',1)
    body=body.replace('        Ok(())','        NONLINEAR.lock().unwrap().push(residuals);\n        Ok(())',1)
    if case.endswith('bounded'):
        body=body.replace('            let delta = system.direction();',
            '            let delta = system.direction();\n            if dot(&system.gradient,&system.gradient) <= 1e-8 && delta.iter().all(|d|d.length_squared() <= 1e-14) { break; }',1)
    p.write_text(prefix+'    pub(super) fn solve_implicit('+body)
    p=dest/'solver.rs';p.write_text(p.read_text().replace('mod implicit;','pub mod implicit;').replace('*p + dt * *v + dt * dt * gravity','*p + dt * *v').replace('let predicted = next.clone();','let predicted:Vec<_> = next.iter().map(|x| *x + dt * dt * gravity).collect();'))
    p=dest/'mod.rs';p.write_text(p.read_text().replace('mod solver;','pub mod solver;'))
    probe=Path('/tmp')/f'cc-tissue-convergence-{case}.rs'
    code=(root/'examples/tissue_contact_probe.rs').read_text().replace('use cc_render_plugin::cc_render::soft_tissue::','use soft_tissue::')
    code=code.replace('    Ok(())\n}', '    soft_tissue::solver::implicit::profile();\n    Ok(())\n}')
    probe.write_text(f'#[path="{dest}/mod.rs"] mod soft_tissue;\n'+code.replace('//! Numerical indentation fixture on the base skin embedding, not avatar motion.','// Numerical indentation fixture'))
    exe=probe.with_suffix('.bin');deps=root/'target/debug/deps'
    cmd=['rustc','--edition=2021','-C','opt-level=1','-L',f'dependency={deps}',str(probe),'-o',str(exe)]
    for name in ['bevy','serde','serde_json','anyhow','thiserror','hmac_sha256']:
        paths=list(deps.glob(f'lib{name}-*.rlib'));assert len(paths)==1,(name,paths)
        cmd+=['--extern',f'{name}={paths[0]}']
    with (folder/f'{case}-build.log').open('x') as log:subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,check=True)
    with (folder/f'{case}.jsonl').open('x') as out,(folder/f'{case}-profile.json').open('x') as err:
        subprocess.run([str(exe),'assets/tissue/beautiful_assistant.base-fitted-prototype.json','artifacts/soft-tissue/contact/base-skin-samples.json','8','120','1e10'],stdout=out,stderr=err,check=True)
    print(case,flush=True)
