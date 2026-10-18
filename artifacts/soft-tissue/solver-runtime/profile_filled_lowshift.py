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
assert len(flags) == len(set(flags)) and set(flags) <= {'residual'}
out = HERE / label
out.mkdir(exist_ok=False)
source = ROOT / 'src/cc_render/soft_tissue'
asset = ROOT / 'assets/tissue/beautiful_assistant.base-viscous-prototype.json'
trace = ROOT / 'artifacts/soft-tissue/lab-front-viscous-240hz-01/tissue.ndjson'
manifest = {'opt_level': int(level), 'experiments': flags, 'files': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
    for p in [asset, trace, HERE/'replay.rs', Path(__file__).resolve(), *sorted(source.glob('*.rs'))]}}
(out / 'inputs.json').write_text(json.dumps(manifest, indent=2) + '\n')
with tarfile.open(out / 'source.tar.gz', 'w:gz') as archive:
    archive.add(source, arcname='soft_tissue')

def replace_once(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new, 1)

with tempfile.TemporaryDirectory(prefix='tissue-runtime-') as temporary:
    scratch = Path(temporary)
    shutil.copytree(source, scratch/'soft_tissue')
    p = scratch/'soft_tissue/preconditioner.rs'
    code = p.read_text()
    code = replace_once(code, 'source: usize,', 'source: Option<usize>,')
    code = replace_once(code, 'for shift in [0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4, 12.8]', 'for shift in [0.01, 0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4, 12.8]')
    code = replace_once(code, 'let mut entries: Vec<_> = signature', 'let original: std::collections::BTreeMap<_, _> = signature.iter().copied().enumerate().map(|(i,e)|(e,i)).collect();\n        let mut filled: std::collections::BTreeSet<_> = signature.iter().copied().collect();\n        for pivot in 0..count {\n            let later: Vec<_> = signature.iter().filter(|e|e[0]==pivot).map(|e|e[1]).collect();\n            for (i, a) in later.iter().enumerate() {\n                for b in &later[i+1..] { filled.insert([(*a).min(*b), (*a).max(*b)]); }\n            }\n        }\n        let extended: Vec<_> = filled.into_iter().collect();\n        let mut entries: Vec<_> = extended')
    code = replace_once(code, '                    source,', '                    source: original.get(&[*a, *b]).copied(),')
    code = replace_once(code, 'let mut s = block(entry.source).transpose();', 'let mut s = entry.source.map(&block).unwrap_or(DMat3::ZERO).transpose();')
    p.write_text(code)
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
        "above_1e_minus_7":residuals.iter().filter(|r|**r>1e-7).count(),
        "above_1e_minus_9":residuals.iter().filter(|r|**r>1e-9).count()}))};
    eprintln!("{}", serde_json::json!({"true_relative_residual":residual,
    "direction_ns":COUNTERS[0].load(Ordering::Relaxed),
    "linearize_ns":COUNTERS[1].load(Ordering::Relaxed),
    "descent_ns":COUNTERS[2].load(Ordering::Relaxed),
    "linear_solves":COUNTERS[3].load(Ordering::Relaxed),
    "cg_steps":COUNTERS[4].load(Ordering::Relaxed),
    "multiply_ns":COUNTERS[5].load(Ordering::Relaxed)})); }
''')
    for old, new in [
        ('fn direction(&self) -> Result<Vec<DVec3>, TissueError> {', 'fn direction(&self) -> Result<Vec<DVec3>, TissueError> {\nlet _timer=Timer::new(0); COUNTERS[3].fetch_add(1, Ordering::Relaxed);'),
        (") -> System<'_> {", ") -> System<'_> {\nlet _timer=Timer::new(1);"),
        ('objective: impl Fn(&[DVec3]) -> f64,\n    ) -> Result<Vec<DVec3>, TissueError> {', 'objective: impl Fn(&[DVec3]) -> f64,\n    ) -> Result<Vec<DVec3>, TissueError> {\nlet _timer=Timer::new(2);'),
        ('let product = self.multiply(&direction);', 'COUNTERS[4].fetch_add(1, Ordering::Relaxed);\nlet product = {let _timer=Timer::new(5); self.multiply(&direction)};'),
    ]:
        code = replace_once(code, old, new)
    if 'residual' in flags:
        code = replace_once(code, '                    return Ok(result);',
            '                    RESIDUALS.lock().unwrap().push((dot(&residual,&residual)/norm_squared).sqrt());\n                    return Ok(result);')
        code = replace_once(code, '        if relative <= 1e-9 {',
            '        RESIDUALS.lock().unwrap().push(relative);\n        if relative <= 1e-9 {')
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
        subprocess.run([str(scratch/'replay'), str(asset), str(trace), str(out/'positions.bin')], stdout=log, stderr=counters, check=True)
print((out/'run.json').read_text())
print((out/'counters.json').read_text())
