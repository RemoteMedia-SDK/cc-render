"""Profile a preserved kernel copy on the reviewed boundary asset and jump."""
from pathlib import Path
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
label = sys.argv[1]
assert label.replace('-', '').isalnum()
flags = sys.argv[2:]
rates = [flag for flag in flags if flag.startswith('--rate=')]
assert len(rates) <= 1
rate = int(rates[0].split('=', 1)[1]) if rates else 240
assert rate in (120, 240, 480)
flags = [flag for flag in flags if not flag.startswith('--rate=')]
scales = [flag for flag in flags if flag.startswith('--nonlinear-scale=')]
assert len(scales) <= 1
nonlinear_scale = int(scales[0].split('=', 1)[1]) if scales else 1
assert nonlinear_scale in (1, 10, 100)
flags = [flag for flag in flags if not flag.startswith('--nonlinear-scale=')]
assert flags in ([], ['--reuse-factor'])
out = HERE / label
out.mkdir(exist_ok=False)
source = ROOT / 'src/cc_render/soft_tissue'
asset = ROOT / 'assets/tissue/beautiful_assistant.base-boundary-nodal-volume-prototype.json'
trace = ROOT / 'artifacts/soft-tissue/lab-boundary-nodal-front-01/tissue.ndjson'
replay = HERE.parent / 'solver-runtime/replay.rs'
inputs = [asset, trace, replay, Path(__file__).resolve(), *sorted(source.glob('*.rs'))]
(out/'inputs.json').write_text(json.dumps({
    'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
    'rate_hz': rate,
    'nonlinear_tolerance_scale': nonlinear_scale,
    'experiments': flags,
    'files': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in inputs},
}, indent=2)+'\n')


def replace(code, old, new):
    assert code.count(old) == 1, old
    return code.replace(old, new, 1)


with tempfile.TemporaryDirectory(prefix='tissue-profile-', dir=ROOT/'target') as temporary:
    scratch = Path(temporary)
    shutil.copytree(source, scratch/'soft_tissue')
    p = scratch/'soft_tissue/implicit.rs'
    code = p.read_text()
    if nonlinear_scale != 1:
        criterion = re.findall(r'force_squared <= ([\deE.+-]+) && correction_squared <= ([\deE.+-]+)', code)
        assert len(criterion) == 1
        force, correction = criterion[0]
        code = replace(code, f'force_squared <= {force} && correction_squared <= {correction}',
            f'force_squared <= {float(force) * nonlinear_scale**2!r} && correction_squared <= {float(correction) * nonlinear_scale**2!r}')
    code = replace(code, 'use bevy::math::DMat3;', '''use bevy::math::DMat3;
use std::sync::atomic::{AtomicU64, Ordering};
static COUNTERS: [AtomicU64; 14] = [const {AtomicU64::new(0)}; 14];
struct Timer(usize, std::time::Instant);
impl Timer { fn new(i: usize) -> Self { Self(i, std::time::Instant::now()) } }
impl Drop for Timer { fn drop(&mut self) {
    COUNTERS[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed);
} }
pub fn profile() {
    let names = ["direction_ns", "linearize_ns", "descent_ns", "linear_solves",
        "cg_steps", "multiply_ns", "elastic_assembly_ns", "viscous_assembly_ns",
        "pressure_assembly_ns", "factor_ns", "precondition_ns", "linearizations",
        "objective_ns", "objective_calls"];
    let values: std::collections::BTreeMap<_, _> = names.iter().enumerate()
        .map(|(i, n)| (*n, COUNTERS[i].load(Ordering::Relaxed))).collect();
    eprintln!("{}", serde_json::to_string(&values).unwrap());
}
''')
    cached_kernel = 'fn direction_preconditioned(' in code
    assert not (cached_kernel and flags), 'The production kernel already reuses factors'
    direction_entry = ('cache: &mut Option<super::preconditioner::Factor<\'a>>,\n'
                       '    ) -> Result<Vec<DVec3>, TissueError> {'
                       if cached_kernel else
                       'fn direction(&self) -> Result<Vec<DVec3>, TissueError> {')
    factor_call = 'super::preconditioner::Factor::new(self.pattern, &self.diagonal, |i| self.couplings[i].block)'
    for old, new in [
        (direction_entry, direction_entry+'\nlet _timer=Timer::new(0); COUNTERS[3].fetch_add(1, Ordering::Relaxed);'),
        (") -> System<'_> {", ") -> System<'_> {\nlet _timer=Timer::new(1); COUNTERS[11].fetch_add(1, Ordering::Relaxed);"),
        ('        let mut scale = 1.0;',
         '        let _timer=Timer::new(2);\n        let mut scale = 1.0;'),
        ('fn objective(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> f64 {',
         'fn objective(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> f64 {\nlet _timer=Timer::new(12); COUNTERS[13].fetch_add(1, Ordering::Relaxed);'),
        ('let product = self.multiply(&direction);',
         'COUNTERS[4].fetch_add(1, Ordering::Relaxed);\nlet product = {let _timer=Timer::new(5); self.multiply(&direction)};'),
        (factor_call, '{let _timer=Timer::new(9); '+factor_call+'}'),
        ('let precondition = |r: &[DVec3]| -> Vec<_> {',
         'let precondition = |r: &[DVec3]| -> Vec<_> {\nlet _timer=Timer::new(10);'),
        ('        for (index, tet) in self.tets.iter().enumerate() {',
         '        { let _timer=Timer::new(6);\n        for (index, tet) in self.tets.iter().enumerate() {'),
        ('        for (p, patch) in self.pressure_patches.iter().enumerate() {',
         '        }\n        { let _timer=Timer::new(8);\n        for (p, patch) in self.pressure_patches.iter().enumerate() {'),
        ('        for contact in contacts {', '        }\n        for contact in contacts {'),
        ('        for element in &damping.elements {',
         '        { let _timer=Timer::new(7);\n        for element in &damping.elements {'),
        ('        // Eliminate fixed degrees of freedom',
         '        }\n        // Eliminate fixed degrees of freedom'),
    ]:
        code = replace(code, old, new)
    if '--reuse-factor' in flags:
        code = replace(code, "impl System<'_> {", "impl<'a> System<'a> {")
        code = replace(code, 'fn direction(&self) -> Result<Vec<DVec3>, TissueError> {',
            '''fn direction(&self) -> Result<Vec<DVec3>, TissueError> {
        self.direction_cached(&mut None)
    }
    fn direction_cached(&self, cache: &mut Option<super::preconditioner::Factor<'a>>) -> Result<Vec<DVec3>, TissueError> {''')
        code = replace(code,
            'let factor = {let _timer=Timer::new(9); super::preconditioner::Factor::new(self.pattern, &self.diagonal, |i| self.couplings[i].block)};',
            '''if cache.is_none() {
            let _timer=Timer::new(9);
            *cache = super::preconditioner::Factor::new(self.pattern, &self.diagonal, |i| self.couplings[i].block);
        }
        let factor = cache.as_ref();''')
        code = replace(code, '        for iteration in 0..=self.material.iterations {',
            '        let mut factor = None;\n        for iteration in 0..=self.material.iterations {')
        code = replace(code, 'let delta = system.direction().map_err(|error| {',
            'let delta = system.direction_cached(&mut factor).map_err(|error| {')
    p.write_text(code)
    for name, old, new in [('solver.rs', 'mod implicit;', 'pub mod implicit;'),
                          ('mod.rs', 'mod solver;', 'pub mod solver;')]:
        p = scratch/'soft_tissue'/name
        p.write_text(replace(p.read_text(), old, new))
    (scratch/'replay.rs').write_text('#[path="soft_tissue/mod.rs"] mod soft_tissue;\n'+replay.read_text())
    with tarfile.open(out/'instrumented-source.tar.gz', 'x:gz') as archive:
        archive.add(scratch/'soft_tissue', arcname='soft_tissue')
        archive.add(scratch/'replay.rs', arcname='replay.rs')
    deps = ROOT/'target/debug/deps'
    command = ['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}',
               str(scratch/'replay.rs'), '-o', str(scratch/'replay')]
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror', 'tracing']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        assert len(paths) == 1, (name, paths)
        command += ['--extern', f'{name}={paths[0]}']
    with (out/'build.log').open('x') as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    with (out/'run.json').open('x') as log, (out/'counters.json').open('x') as counters:
        subprocess.run([str(scratch/'replay'), str(asset), str(trace),
                        str(out/'positions.bin'), str(rate), '50'],
                       stdout=log, stderr=counters, check=True)
print((out/'run.json').read_text())
print((out/'counters.json').read_text())
