"""Small-strain diagnostic on the actual authored mesh; not animation or a runtime solver.

Compare element pressure with volume-weighted nodal pressure (ANP), leaving mass,
geometry, boundary and shear modulus identical. Linearization is at the unstressed
reference and therefore does not include loaded-state prestress/hardening.
"""
from pathlib import Path
import hashlib
import json
import numpy as np
from scipy.linalg import eigh, solve

ROOT = Path(__file__).resolve().parents[3]
ASSET = ROOT / 'assets/tissue/beautiful_assistant.base-skin-volume-v2-prototype.json'


def audit(region):
    x = np.asarray(region['rest_positions'])
    cells = np.asarray(region['tetrahedra'])
    free = np.array([a['kind'] == 'free' for a in region['attachments']])
    dofs = np.flatnonzero(np.repeat(free, 3))
    count = 3 * len(x)
    shear = np.zeros((count, count))
    divergence = np.zeros((len(cells), count))
    nodal_average = np.zeros((len(x), len(cells)))
    masses = np.zeros(len(x))
    volumes = []
    mu = region['material']['shear_pa']
    rho = region['material']['density_kg_m3']
    for e, ids in enumerate(cells):
        edges = (x[ids[1:]] - x[ids[0]]).T
        volume = abs(np.linalg.det(edges)) / 6
        grad = np.vstack([-np.linalg.inv(edges).sum(axis=0), np.linalg.inv(edges)])
        volumes.append(volume)
        masses[ids] += rho * volume / 4
        nodal_average[ids, e] = volume / 4
        for a, i in enumerate(ids):
            divergence[e, 3*i:3*i+3] = grad[a]
            for b, j in enumerate(ids):
                shear[3*i:3*i+3, 3*j:3*j+3] += mu * volume * (
                    np.dot(grad[a], grad[b]) * np.eye(3) + np.outer(grad[b], grad[a]))
    volumes = np.asarray(volumes)
    nodal_volumes = nodal_average.sum(axis=1)
    nodal_average /= nodal_volumes[:, None]
    div = divergence[:, dofs]
    avg_div = nodal_average @ div
    element_bulk = div.T @ (volumes[:, None] * div)
    nodal_bulk = avg_div.T @ (nodal_volumes[:, None] * avg_div)
    mass = np.repeat(masses, 3)[dofs]
    mu_matrix = shear[np.ix_(dofs, dofs)]
    rows = []
    for pressure, operator in [('element', element_bulk), ('nodal', nodal_bulk)]:
        for bulk in [15000., 50000., 500000., 5000000.]:
            matrix = mu_matrix + (bulk - 2*mu/3) * operator
            values, modes = eigh(matrix / np.sqrt(mass[:, None] * mass[None, :]), subset_by_index=(0, 5))
            mode = modes[:, 0] / np.sqrt(mass)
            eta = region['material']['viscosity']
            damping = (eta['shear_pa_s'] / mu) * mu_matrix + (eta['bulk_pa_s'] - 2*eta['shear_pa_s']/3) * operator
            zeta = float(mode @ damping @ mode / (2*np.sqrt(values[0])))
            # Unit body force, along each local axis; independent of any motion clip.
            force = (masses[:, None, None] * np.eye(3)[None, :, :]).reshape(count, 3)[dofs]
            displacement = solve(matrix, force, assume_a='pos').reshape(-1, 3, 3)
            mean_mm_per_g = (np.einsum('i,ijk->jk', masses[free], displacement) / masses[free].sum() * 9810).tolist()
            rows.append({'pressure': pressure, 'bulk_pa': bulk, 'lowest_frequencies_hz': (np.sqrt(values)/(2*np.pi)).tolist(), 'first_mode_damping_ratio': zeta, 'mass_mean_displacement_mm_per_g_by_axis': mean_mm_per_g})
    return {'region': region['name'], 'free_dofs': len(dofs), 'element_divergence_rank': int(np.linalg.matrix_rank(div)), 'nodal_divergence_rank': int(np.linalg.matrix_rank(avg_div)), 'rows': rows}


if __name__ == '__main__':
    asset = json.loads(ASSET.read_text())
    out = {'asset_sha256': hashlib.sha256(ASSET.read_bytes()).hexdigest(), 'limitation': 'Unstressed small-strain diagnostic, not loaded nonlinear runtime prediction.', 'regions': [audit(r) for r in asset['regions']]}
    path = Path(__file__).with_suffix('.json')
    path.write_text(json.dumps(out, indent=2) + '\n')
    for r in out['regions']:
        print(r['region'], 'free DOFs:', r['free_dofs'], 'element/nodal ranks:', r['element_divergence_rank'], r['nodal_divergence_rank'])
        for row in r['rows']:
            print(row['pressure'], row['bulk_pa'], 'f0 Hz',round(row['lowest_frequencies_hz'][0],3), 'damping',round(row['first_mode_damping_ratio'],3),'mm/g diagonal',np.diag(row['mass_mean_displacement_mm_per_g_by_axis']).round(3))
