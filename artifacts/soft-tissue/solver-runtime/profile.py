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
iteration_flags = [f for f in flags if f.startswith('--iterations=')]
rate_flags = [f for f in flags if f.startswith('--rate=')]
start_flags = [f for f in flags if f.startswith('--start-ms=')]
assert len(iteration_flags) <= 1
assert len(rate_flags) <= 1 and len(start_flags) <= 1
iterations = int(iteration_flags[0].split('=')[1]) if iteration_flags else None
rate = int(rate_flags[0].split('=')[1]) if rate_flags else 240
start = float(start_flags[0].split('=')[1]) if start_flags else 50.0
assert rate in [120, 240, 480] and 50 <= start <= 1000
assert iterations is None or 1 <= iterations <= 64
assert len(flags) == len(set(flags)) and set(flags) - set(iteration_flags + rate_flags + start_flags) <= {'residual', 'full-factor', 'nonlinear', 'viscosity-baseline', 'stages'}
if 'viscosity-baseline' in flags and 'history_log_volume' in (ROOT/'src/cc_render/soft_tissue/timestep.rs').read_text():
    raise SystemExit('viscosity-baseline compares the earlier coupled material only; restore its archived source checkpoint before using this flag')
out = HERE / label
out.mkdir(exist_ok=False)
source = ROOT / 'src/cc_render/soft_tissue'
asset = ROOT / 'assets/tissue/beautiful_assistant.base-viscous-prototype.json'
trace = ROOT / 'artifacts/soft-tissue/lab-front-viscous-240hz-01/tissue.ndjson'
extra = [HERE/'full-factor.rs'] if 'full-factor' in flags else []
if 'viscosity-baseline' in flags:
    extra.append(HERE/'viscosity-baseline.rs')
manifest = {'opt_level': int(level), 'experiments': flags, 'files': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
    for p in [asset, trace, HERE/'replay.rs', HERE/'profile.py', *extra, *sorted(source.glob('*.rs'))]}}
if iterations is not None:
    effective = json.loads(asset.read_text())
    for region in effective['regions']:
        region['material']['iterations'] = iterations
    asset = out / 'effective-asset.json'
    asset.write_text(json.dumps(effective, separators=(',', ':')) + '\n')
    manifest['effective_asset_sha256'] = hashlib.sha256(asset.read_bytes()).hexdigest()
(out / 'inputs.json').write_text(json.dumps(manifest, indent=2) + '\n')
with tarfile.open(out / 'source.tar.gz', 'w:gz') as archive:
    archive.add(source, arcname='soft_tissue')

def replace_once(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new, 1)

with tempfile.TemporaryDirectory(prefix='tissue-runtime-') as temporary:
    scratch = Path(temporary)
    shutil.copytree(source, scratch/'soft_tissue')
    if 'full-factor' in flags:
        shutil.copyfile(HERE/'full-factor.rs', scratch/'soft_tissue/preconditioner.rs')
    if 'viscosity-baseline' in flags:
        shutil.copyfile(HERE/'viscosity-baseline.rs', scratch/'soft_tissue/viscosity.rs')
    p = scratch/'soft_tissue/implicit.rs'
    code = p.read_text()
    code = replace_once(code, 'use bevy::math::DMat3;', '''use bevy::math::DMat3;
use std::sync::atomic::{AtomicU64, Ordering};
static COUNTERS: [AtomicU64; 8] = [const {AtomicU64::new(0)}; 8];
static RESIDUALS: std::sync::Mutex<Vec<f64>> = std::sync::Mutex::new(Vec::new());
static NONLINEAR: std::sync::Mutex<Vec<[f64; 2]>> = std::sync::Mutex::new(Vec::new());
struct Timer(usize, std::time::Instant);
impl Timer { fn new(i:usize)->Self { Self(i,std::time::Instant::now()) } }
impl Drop for Timer { fn drop(&mut self) { COUNTERS[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed); } }
pub fn profile() {
    let mut residuals=RESIDUALS.lock().unwrap();
    residuals.sort_by(f64::total_cmp);
    let residual=if residuals.is_empty() {None} else {Some(serde_json::json!({
        "count":residuals.len(),"median":residuals[residuals.len()/2],
        "p95":residuals[residuals.len()*95/100],"maximum":residuals.last(),
        "above_1e_minus_7":residuals.iter().filter(|r|**r>1e-7).count(),
        "above_1e_minus_9":residuals.iter().filter(|r|**r>1e-9).count()}))};
    eprintln!("{}", serde_json::json!({"true_relative_residual":residual,
    "nonlinear_force_n_correction_m": &*NONLINEAR.lock().unwrap(),
    "direction_ns":COUNTERS[0].load(Ordering::Relaxed),
    "linearize_ns":COUNTERS[1].load(Ordering::Relaxed),
    "descent_ns":COUNTERS[2].load(Ordering::Relaxed),
    "linear_solves":COUNTERS[3].load(Ordering::Relaxed),
    "cg_steps":COUNTERS[4].load(Ordering::Relaxed),
    "multiply_ns":COUNTERS[5].load(Ordering::Relaxed),
    "elastic_assembly_ns":COUNTERS[6].load(Ordering::Relaxed),
    "viscous_assembly_ns":COUNTERS[7].load(Ordering::Relaxed)})); }
''')
    for old, new in [
        ('fn direction(&self) -> Result<Vec<DVec3>, TissueError> {', 'fn direction(&self) -> Result<Vec<DVec3>, TissueError> {\nlet _timer=Timer::new(0); COUNTERS[3].fetch_add(1, Ordering::Relaxed);'),
        (") -> System<'_> {", ") -> System<'_> {\nlet _timer=Timer::new(1);"),
        ('objective: impl Fn(&[DVec3]) -> f64,\n    ) -> Result<Vec<DVec3>, TissueError> {', 'objective: impl Fn(&[DVec3]) -> f64,\n    ) -> Result<Vec<DVec3>, TissueError> {\nlet _timer=Timer::new(2);'),
        ('let product = self.multiply(&direction);', 'COUNTERS[4].fetch_add(1, Ordering::Relaxed);\nlet product = {let _timer=Timer::new(5); self.multiply(&direction)};'),
    ]:
        code = replace_once(code, old, new)
    if 'stages' in flags:
        for old, new in [
            ('        for tet in &self.tets {', '        { let _timer=Timer::new(6);\n        for tet in &self.tets {'),
            ('        for contact in contacts {', '        }\n        for contact in contacts {'),
            ('        for element in &damping.elements {', '        { let _timer=Timer::new(7);\n        for element in &damping.elements {'),
            ('        // Eliminate fixed degrees of freedom', '        }\n        // Eliminate fixed degrees of freedom'),
        ]:
            code = replace_once(code, old, new)
    if 'residual' in flags:
        code = replace_once(code, '                    return Ok(result);',
            '                    RESIDUALS.lock().unwrap().push((dot(&residual,&residual)/norm_squared).sqrt());\n                    return Ok(result);')
        code = replace_once(code, '        if relative <= 1e-9 {',
            '        RESIDUALS.lock().unwrap().push(relative);\n        if relative <= 1e-9 {')
    if 'nonlinear' in flags:
        # The strict solver already checks final force and correction. Record
        # accepted states only; failed trials follow the normal bounded retry.
        for condition, value in [
            ('force_squared == 0.0', '[0.0, 0.0]'),
            ('force_squared <= 1e-8 && correction_squared <= 1e-14',
             '[force_squared.sqrt(), correction_squared.sqrt()]'),
        ]:
            old = 'if ' + condition + ' {\n                return Ok(());'
            new = 'if ' + condition + ' {\n                NONLINEAR.lock().unwrap().push(' + value + ');\n                return Ok(());'
            code = replace_once(code, old, new)
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
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror', 'tracing']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        assert len(paths) == 1, (name, paths)
        command += ['--extern', f'{name}={paths[0]}']
    with (out/'build.log').open('w') as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    with (out/'run.json').open('w') as log, (out/'counters.json').open('w') as counters:
        subprocess.run([str(scratch/'replay'), str(asset), str(trace), str(out/'positions.bin'), str(rate), str(start)], stdout=log, stderr=counters, check=True)
print((out/'run.json').read_text())
counters = json.loads((out/'counters.json').read_text())
counters['nonlinear_observations'] = len(counters.pop('nonlinear_force_n_correction_m'))
print(json.dumps(counters))
