from pathlib import Path
import shutil, subprocess
root=Path.cwd(); source=root/'src/cc_render/soft_tissue'; dest=Path('/tmp/cc-tissue-profile-source')
shutil.copytree(source,dest,dirs_exist_ok=True)
p=dest/'implicit.rs'; s=p.read_text()
s=s.replace('use bevy::math::DMat3;', '''use bevy::math::DMat3;
use std::sync::atomic::{AtomicU64, Ordering};
static TIMES: [AtomicU64; 6] = [const {AtomicU64::new(0)}; 6];
struct Timer(usize, std::time::Instant);
impl Timer { fn new(i:usize)->Self { Self(i,std::time::Instant::now()) } }
impl Drop for Timer { fn drop(&mut self) { TIMES[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed); } }
pub fn profile() { eprintln!("PROFILE multiply_ns={} direction_ns={} linearize_ns={} objective_ns={} cg_steps={} solves={}", TIMES[0].load(Ordering::Relaxed),TIMES[1].load(Ordering::Relaxed),TIMES[2].load(Ordering::Relaxed),TIMES[3].load(Ordering::Relaxed),TIMES[4].load(Ordering::Relaxed),TIMES[5].load(Ordering::Relaxed)); }
''')
s=s.replace('fn multiply(&self, input: &[DVec3]) -> Vec<DVec3> {','fn multiply(&self, input: &[DVec3]) -> Vec<DVec3> {\n let _timer=Timer::new(0);')
s=s.replace('fn direction(&self) -> Vec<DVec3> {','fn direction(&self) -> Vec<DVec3> {\n let _timer=Timer::new(1); TIMES[5].fetch_add(1,Ordering::Relaxed);')
s=s.replace('let product = self.multiply(&direction);','TIMES[4].fetch_add(1,Ordering::Relaxed);\n let product = self.multiply(&direction);')
s=s.replace('fn linearize(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> System {','fn linearize(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> System {\n let _timer=Timer::new(2);')
s=s.replace('fn objective(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> f64 {','fn objective(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> f64 {\n let _timer=Timer::new(3);')
p.write_text(s)
p=dest/'solver.rs'; p.write_text(p.read_text().replace('mod implicit;', 'pub mod implicit;'))
p=dest/'mod.rs'; p.write_text(p.read_text().replace('mod solver;', 'pub mod solver;'))
p=Path('/tmp/cc-tissue-timed-probe.rs'); s=Path('/tmp/cc-soft-tissue-optimized-probe.rs').read_text().replace(str(source/'mod.rs'),str(dest/'mod.rs')); s=s.replace('    Ok(())\n}', '    soft_tissue::solver::implicit::profile();\n    Ok(())\n}'); p.write_text(s)
deps=root/'target/debug/deps'; cmd=['rustc','--edition=2021','-C','opt-level=3','-L',f'dependency={deps}',str(p),'-o','/tmp/cc-tissue-timed-probe']
for name in ['bevy','serde','serde_json','anyhow','thiserror']:
    paths=list(deps.glob(f'lib{name}-*.rlib')); assert len(paths)==1
    cmd += ['--extern',f'{name}={paths[0]}']
with open('/tmp/cc-tissue-timed-build.log','w') as out: subprocess.run(cmd,stdout=out,stderr=subprocess.STDOUT,check=True)
