"""Screen geometry-selected element pressure in an archived copy of the kernel."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--combined', action='store_true')
parser.add_argument('--output', type=Path)
args = parser.parse_args()
OUT = args.output.resolve() if args.output else HERE/('combined-01' if args.combined else 'study-01')
BASELINE = '91246ac'
OUT.mkdir(exist_ok=False)
ASSET = ROOT/'assets/tissue/beautiful_assistant.base-boundary-nodal-volume-prototype.json'
TRACE = HERE.parent/'lab-boundary-nodal-front-01/tissue.ndjson'
plan = {
    'baseline_commit': subprocess.check_output(['git', 'rev-parse', BASELINE], cwd=ROOT, text=True).strip(),
    'workspace_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
    'quality_cutoffs': [0, 0.1, 0.15, 0.2], 'rate_hz': 240,
    'definition': 'Mean-ratio quality in unchanged authored initial coordinates; 1 is equilateral. Select fixed-edge cells or cells below the cutoff once at construction. No motion-dependent switching.',
    'criteria': [
        'Unchanged source motion, attachment positions, topology, surface embeddings and material coefficients.',
        'Refit each candidate to the existing 0.25 mm equilibrium-shape criterion.',
        'Every sampled local volume ratio in [0.5, 2.0]; total-volume change below 1 percent.',
        'At least 80 percent of baseline peak and time-RMS embedded-surface displacement.',
        'A passing CPU screen requires production derivative tests, Bevy capture and visual comparison.',
    ],
    'inputs': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
               for p in [ASSET, TRACE, Path(__file__).resolve(), *sorted((ROOT/'src/cc_render/soft_tissue').glob('*.rs'))]},
    'runs': [],
}
if args.combined:
    plan.pop('quality_cutoffs')
    plan['cases'] = [
        {'id': 'reference', 'quality_cutoff': 0, 'pressure_share_pa': 0},
        {'id': 'quality', 'quality_cutoff': 0.15, 'pressure_share_pa': 0},
        {'id': 'combined-2000', 'quality_cutoff': 0.15, 'pressure_share_pa': 2000},
        {'id': 'combined-8000', 'quality_cutoff': 0.15, 'pressure_share_pa': 8000},
    ]
    plan['basis'] = 'Quality-only treatment satisfies right-side volume bounds while retaining 96 percent peak and 98 percent RMS movement. A small variance penalty targets the remaining regular left element; original acceptance limits remain unchanged.'


def save():
    (OUT/'manifest.json').write_text(json.dumps(plan, indent=2)+'\n')


def run(command, name, cutoff=0, output=None):
    environment = {'TISSUE_STUDY_QUALITY_CUTOFF': str(cutoff)}
    record = {'argv': list(map(str, command)), 'environment': environment, 'started_unix': time.time()}
    plan['runs'].append(record)
    save()
    with (OUT/(output or name+'.log')).open('x') as log, (OUT/(name+'.stderr')).open('x') as errors:
        result = subprocess.run(record['argv'], cwd=ROOT, env={**os.environ, **environment}, stdout=log, stderr=errors)
    record.update(exit_code=result.returncode, completed_unix=time.time())
    save()
    print(name, result.returncode, flush=True)
    return result


save()
with tempfile.TemporaryDirectory(prefix='tissue-quality-', dir=ROOT/'target') as temporary:
    scratch = Path(temporary)
    if args.combined:
        saved = HERE.parent/'pressure-stabilization/study-01/source.tar.gz'
        plan['base_kernel_archive'] = str(saved.relative_to(ROOT))
        plan['base_kernel_sha256'] = hashlib.sha256(saved.read_bytes()).hexdigest()
        with tarfile.open(saved) as archive:
            for member in archive.getmembers():
                path = Path(member.name)
                if not member.isfile() or path.parts[0] != 'soft_tissue':
                    continue
                assert not path.is_absolute() and '..' not in path.parts
                (scratch/path).parent.mkdir(parents=True, exist_ok=True)
                (scratch/path).write_bytes(archive.extractfile(member).read())
    else:
        paths = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', BASELINE, 'src/cc_render/soft_tissue'], cwd=ROOT, text=True).splitlines()
        for source_path in paths:
            relative = Path(source_path).relative_to('src/cc_render')
            (scratch/relative).parent.mkdir(parents=True, exist_ok=True)
            (scratch/relative).write_bytes(subprocess.check_output(['git', 'show', BASELINE+':'+source_path], cwd=ROOT))
    path = scratch/'soft_tissue/solver.rs'
    source = path.read_text()
    marker = '        let element_pressure: Vec<_> = tets.iter().map(|tet| match material.pressure_discretization {'
    assert source.count(marker) == 1
    source = source.replace(marker, '''        let cutoff: f64 = std::env::var("TISSUE_STUDY_QUALITY_CUTOFF").unwrap_or_else(|_| "0".into()).parse().unwrap();
        assert!(cutoff.is_finite() && (0.0..=1.0).contains(&cutoff));
'''+marker)
    marker = '.filter(|&&i| matches!(attachments[i], Attachment::Fixed)).count() >= 2,'
    assert source.count(marker) == 1
    source = source.replace(marker, '''.filter(|&&i| matches!(attachments[i], Attachment::Fixed)).count() >= 2
                || pressure::mean_ratio_quality(tet.ids.map(|i| initial[i])) < cutoff,''')
    path.write_text(source)
    path = scratch/'soft_tissue/pressure.rs'
    path.write_text(path.read_text()+'''
pub(super) fn mean_ratio_quality(points: [DVec3; 4]) -> f64 {
    let six_volume = super::material::edges(points).determinant().abs();
    let edge_sum: f64 = super::material::EDGE_PAIRS.iter()
        .map(|[a, b]| (points[*a]-points[*b]).length_squared()).sum();
    12. * (six_volume / 2.).powf(2./3.) / edge_sum
}
''')
    fit = (ROOT/'examples/tissue_fit.rs').read_text().replace('//! Offline', '// Offline')
    fit = fit.replace('use cc_render_plugin::cc_render::soft_tissue::', 'use soft_tissue::')
    replay = (HERE.parent/'solver-runtime/replay.rs').read_text()
    marker = '    soft_tissue::solver::implicit::profile();\n'
    assert replay.count(marker) == 1
    replay = replay.replace(marker, '')
    for name, code in [('fit', fit), ('replay', replay)]:
        (scratch/(name+'.rs')).write_text('#[path="soft_tissue/mod.rs"] mod soft_tissue;\n'+code)
    with tarfile.open(OUT/'source.tar.gz', 'x:gz') as archive:
        archive.add(scratch/'soft_tissue', arcname='soft_tissue')
        for name in ['fit', 'replay']:
            archive.add(scratch/(name+'.rs'), arcname=name+'.rs')
    deps = ROOT/'target/debug/deps'
    externs = []
    for name in ['bevy', 'serde', 'serde_json', 'anyhow', 'thiserror', 'tracing']:
        paths = list(deps.glob(f'lib{name}-*.rlib'))
        assert len(paths) == 1, (name, paths)
        externs += ['--extern', f'{name}={paths[0]}']
    for name in ['fit', 'replay']:
        run(['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}',
             scratch/(name+'.rs'), '-o', scratch/name, *externs], name+'-build').check_returncode()
    cases = plan['cases'] if args.combined else [
        {'id': str(cutoff), 'quality_cutoff': cutoff, 'pressure_share_pa': 0}
        for cutoff in plan['quality_cutoffs']]
    prefix = 'case' if args.combined else 'quality'
    for case in cases:
        label, cutoff = case['id'], case['quality_cutoff']
        original = json.loads(ASSET.read_text())
        if args.combined:
            for region in original['regions']:
                region['material']['pressure_stabilization_pa'] = case['pressure_share_pa']
        unfitted = OUT/f'{prefix}-{label}-input.json'
        unfitted.write_text(json.dumps(original, separators=(',', ':'))+'\n')
        fitted = OUT/f'{prefix}-{label}-fitted.json'
        if cutoff or case['pressure_share_pa']:
            result = run([scratch/'fit', unfitted, fitted], f'{prefix}-{label}-fit', cutoff)
            if result.returncode:
                continue
        else:
            shutil.copyfile(unfitted, fitted)
        run([scratch/'replay', fitted, TRACE, OUT/f'{prefix}-{label}-positions.bin', '240', '50'],
            f'{prefix}-{label}-replay', cutoff, f'{prefix}-{label}-run.json').check_returncode()
