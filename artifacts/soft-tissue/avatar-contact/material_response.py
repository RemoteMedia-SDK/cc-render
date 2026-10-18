"""Audit the current small-strain material response; no parameter fitting."""
from pathlib import Path
import json, hashlib
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
root = Path.cwd(); out = root / 'artifacts/soft-tissue/avatar-contact'
asset_path = root / 'assets/tissue/beautiful_assistant.base-viscous-prototype.json'
asset = json.loads(asset_path.read_text())
m = asset['regions'][0]['material']
assert all(r['material'] == m for r in asset['regions'])
mu, bulk, eta = m['shear_pa'], m['bulk_pa'], m['viscosity']['shear_pa_s']
f = np.geomspace(.001, 500, 1000)
loss = 2 * np.pi * f * eta
report = {'asset_sha256': hashlib.sha256(asset_path.read_bytes()).hexdigest(), 'scope': 'Analytical small-strain linearization of the implemented homogeneous Kelvin-Voigt material; not measured anatomical response or a calibration fit.', 'shear_storage_pa': mu, 'shear_viscosity_pa_s': eta, 'young_modulus_pa': 9*bulk*mu/(3*bulk+mu), 'poisson_ratio': (3*bulk-2*mu)/(2*(3*bulk+mu)), 'shear_retardation_s': eta/mu, 'samples': [{'frequency_hz': v, 'shear_loss_pa': 2*np.pi*v*eta, 'loss_to_storage_ratio': 2*np.pi*v*eta/mu} for v in [.2,1,2,5,10]], 'literature_frequency_windows_hz': {'Sridhar_Insana_2007': [.01/(2*np.pi), 1/(2*np.pi)], 'Kumar_et_al_2018': [50,400]}, 'sources': ['https://ws.engr.illinois.edu/blogs/getfile/151/30849', 'https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0205717'], 'interpretation': 'The cited studies concern different loading, frequency and observation regimes. Neither directly calibrates the current jump response. No material values were changed.'}
(out / 'material-response.json').write_text(json.dumps(report, indent=2)+'\n')
fig, ax = plt.subplots(figsize=(10,5.5),layout='constrained')
ax.loglog(f, np.full_like(f,mu),label="Current small-strain storage modulus G′")
ax.loglog(f,loss,label="Current small-strain loss modulus G″")
ax.axvspan(.01/(2*np.pi),1/(2*np.pi),color='grey',alpha=.13,label='2007 quasi-static measurement band')
ax.axvspan(50,400,color='orange',alpha=.13,label='2018 shear-wave measurement band')
ax.set(xlabel='Frequency (Hz)',ylabel='Shear modulus (Pa)',title='Current material assumptions — analytical, not an anatomical fit')
ax.grid(which='both',alpha=.15);ax.legend(loc='upper left',fontsize=9)
fig.savefig(out/'material-response.png',dpi=150)
print(json.dumps(report,indent=2))
