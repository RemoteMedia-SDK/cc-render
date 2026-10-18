"""Profile an isolated copy of the current kernel; preserve source and inputs."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
label, level, *flags = sys.argv[1:]
assert label.replace('-', '').isalnum() and level in ('1', '3')
assert len(flags) == len(set(flags)) and set(flags) <= {'residual', 'forcing', 'ic0', 'icfast', 'reference'}
assert 'icfast' not in flags or 'ic0' in flags
out = HERE / label
out.mkdir(exist_ok=False)
source = ROOT / 'src/cc_render/soft_tissue'
asset = ROOT / 'assets/tissue/beautiful_assistant.base-viscous-prototype.json'
trace = ROOT / 'artifacts/soft-tissue/lab-front-viscous-240hz-01/tissue.ndjson'
manifest = {'opt_level': int(level), 'experiments': flags, 'files': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
    for p in [asset, trace, HERE/'replay.rs', HERE/'profile.py', HERE/'ic0.rs', *sorted(source.glob('*.rs'))]}}
(out / 'inputs.json').write_text(json.dumps(manifest, indent=2) + '\n')
with tarfile.open(out / 'source.tar.gz', 'w:gz') as archive:
    archive.add(source, arcname='soft_tissue')

def replace_once(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new, 1)

with tempfile.TemporaryDirectory(prefix='tissue-runtime-') as temporary:
    scratch = Path(temporary)
    shutil.copytree(source, scratch/'soft_tissue')
    p = scratch/'soft_tissue/implicit.rs'
    code = p.read_text()
    code = replace_once(code, 'use bevy::math::DMat3;', '''use bevy::math::DMat3;
use std::sync::atomic::{AtomicU64, Ordering};
static COUNTERS: [AtomicU64; 6] = [const {AtomicU64::new(0)}; 6];
static RESIDUALS: std::sync::Mutex<Vec<f64>> = std::sync::Mutex::new(Vec::new());
struct Timer(usize, std::time::Instant);
impl Timer { fn new(i:usize)->Self { Self(i,std::time::Instant::now()) } }
impl Drop for Timer { fn drop(&mut self) { COUNTERS[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed); } }
pub fn profile() {
    let mut residuals=RESIDUALS.lock().unwrap();
    residuals.sort_by(f64::total_cmp);
    let residual=if residuals.is_empty() {None} else {Some(serde_json::json!({
        "count":residuals.len(),"median":residuals[residuals.len()/2],
        "p95":residuals[residuals.len()*95/100],"maximum":residuals.last(),
        "above_1e_minus_7":residuals.iter().filter(|r|**r>1e-7).count()}))};
    eprintln!("{}", serde_json::json!({"true_relative_residual":residual,
    "direction_ns":COUNTERS[0].load(Ordering::Relaxed),
    "linearize_ns":COUNTERS[1].load(Ordering::Relaxed),
    "descent_ns":COUNTERS[2].load(Ordering::Relaxed),
    "linear_solves":COUNTERS[3].load(Ordering::Relaxed),
    "cg_steps":COUNTERS[4].load(Ordering::Relaxed),
    "multiply_ns":COUNTERS[5].load(Ordering::Relaxed)})); }
''')
    for old, new in [
        ('fn direction(&self) -> Vec<DVec3> {', 'fn direction(&self) -> Vec<DVec3> {\nlet _timer=Timer::new(0); COUNTERS[3].fetch_add(1, Ordering::Relaxed);'),
        (') -> System {', ') -> System {\nlet _timer=Timer::new(1);'),
        (') -> Result<Vec<DVec3>, TissueError> {', ') -> Result<Vec<DVec3>, TissueError> {\nlet _timer=Timer::new(2);'),
        ('let product = self.multiply(&direction);', 'COUNTERS[4].fetch_add(1, Ordering::Relaxed);\nlet product = {let _timer=Timer::new(5); self.multiply(&direction)};'),
    ]:
        code = replace_once(code, old, new)
    if 'residual' in flags:
        code = replace_once(code, '        result\n    }\n}', '''        let actual:Vec<_>=self.multiply(&result).iter().zip(&self.gradient).map(|(a,b)|*a+*b).collect();
        RESIDUALS.lock().unwrap().push((dot(&actual,&actual)/dot(&self.gradient,&self.gradient)).sqrt());
        result
    }
}''')
    if 'forcing' in flags:
        code = replace_once(code, 'fn direction(&self) -> Vec<DVec3>', 'fn direction(&self, relative_tolerance: f64) -> Vec<DVec3>')
        code = replace_once(code, 'let threshold = dot(&residual, &residual) * 1e-14;', 'let threshold = dot(&residual, &residual) * relative_tolerance.powi(2);')
        code = code.replace('system.direction()', 'system.direction(1e-7)')
        prefix, body = code.split('    pub(super) fn solve_implicit(', 1)
        body = replace_once(body, '        for _ in 0..self.material.iterations {', '''        let mut initial_force = 0.0;
        let minimum_mass = self.masses.iter().zip(&self.inverse_mass)
            .filter(|(_,w)| **w > 0.0).map(|(m,_)|*m).fold(f64::INFINITY, f64::min);
        for _ in 0..self.material.iterations {''')
        body = replace_once(body, '            let delta = system.direction(1e-7);', '''            let force = dot(&system.gradient, &system.gradient).sqrt();
            if initial_force == 0.0 { initial_force = force; }
            let eta = (force / initial_force).sqrt().clamp(1e-7, 0.1);
            let delta = system.direction(eta);''')
        body = replace_once(body, '&& delta.iter().all(|d| d.length_squared() <= 1e-14)', '''&& {
                    let actual:Vec<_> = system.multiply(&delta).iter().zip(&system.gradient)
                        .map(|(a,b)| *a+*b).collect();
                    let missing_step = dot(&actual,&actual).sqrt()*dt*dt/minimum_mass;
                    delta.iter().map(|d|d.length()).fold(0.0_f64,f64::max)+missing_step <= 1e-7
                }''')
        code = prefix + '    pub(super) fn solve_implicit(' + body
    if 'ic0' in flags:
        code = replace_once(code, '        let precondition = |r: &[DVec3]| -> Vec<_> {', '''        let factor = IcFactor::new(self);
        let precondition = |r: &[DVec3]| -> Vec<_> {
            if let Some(factor) = &factor { return factor.apply(r); }''')
        code = replace_once(code, '"true_relative_residual":residual,', '''"ic_shift_counts":IC_STATS.iter().map(|v|v.load(Ordering::Relaxed)).collect::<Vec<_>>(),
        "true_relative_residual":residual,''')
        experiment = (HERE/'ic0.rs').read_text()
        if 'icfast' in flags:
            experiment = replace_once(experiment,
                '[0.0, 1e-5, 1e-4, 1e-3, 1e-2, 0.1, 1.0, 10.0]',
                '[0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4, 12.8]')
        (out/'ic0-experiment.rs').write_text(experiment)
        code += '\n' + experiment
    if 'reference' in flags:
        code = replace_once(code, 'for _ in 0..128 {', 'for _ in 0..1024 {')
    p.write_text(code)
    for filename, old, new in [('solver.rs', 'mod implicit;', 'pub mod implicit;'), ('mod.rs', 'mod solver;', 'pub mod solver;')]:
        p = scratch/'soft_tissue'/filename
        p.write_text(replace_once(p.read_text(), old, new))
    (scratch/'replay.rs').write_text('#[path="soft_tissue/mod.rs"] mod soft_tissue;\n' + (HERE/'replay.rs').read_text())
    with tarfile.open(out/'instrumented-source.tar.gz', 'w:gz') as archive:
        archive.add(scratch/'soft_tissue', arcname='soft_tissue')
        archive.add(scratch/'replay.rs', arcname='replay.rs')
    deps = ROOT/'target/debug/deps'
    command = ['rustc', '--edition=2021', '-C', f'opt-level={level}', '-L', f'dependency={deps}', str(scratch/'replay.rs'), '-o', str(scratch/'replay')]
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        assert len(paths) == 1, (name, paths)
        command += ['--extern', f'{name}={paths[0]}']
    with (out/'build.log').open('w') as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    with (out/'run.json').open('w') as log, (out/'counters.json').open('w') as counters:
        subprocess.run([str(scratch/'replay'), str(asset), str(trace), str(out/'positions.bin')], stdout=log, stderr=counters, check=True)
print((out/'run.json').read_text())
print((out/'counters.json').read_text())
