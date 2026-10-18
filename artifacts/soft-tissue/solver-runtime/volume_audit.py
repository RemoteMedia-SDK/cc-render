"""Measure total and local cage volume in the preserved full-node CPU replay."""
from pathlib import Path
import hashlib,json
import numpy as np
root=Path.cwd(); folder=root/'artifacts/soft-tissue/solver-runtime'
asset_path=root/'assets/tissue/beautiful_assistant.base-viscous-prototype.json'
positions_path=folder/'restart-opt3-01/positions.bin'
asset=json.loads(asset_path.read_text()); counts=[len(r['rest_positions']) for r in asset['regions']]
assert len(set(counts))==1
positions=np.fromfile(positions_path,dtype='<f8').reshape(-1,len(counts),counts[0],3)
def volume(points,tets):
 p=points[...,tets,:];return np.linalg.det(np.stack([p[...,i,:]-p[...,0,:] for i in [1,2,3]],axis=-1))/6
report={'scope':'Recorded-attachment CPU replay, 1908 steps; not exact GPU node output. Total volume is relative to the authored loaded shape; element J is relative to fitted unloaded rest.', 'asset_sha256':hashlib.sha256(asset_path.read_bytes()).hexdigest(),'positions_sha256':hashlib.sha256(positions_path.read_bytes()).hexdigest(),'regions':[]}
for i,r in enumerate(asset['regions']):
 tets=np.array(r['tetrahedra']);ref=volume(np.array(r['rest_positions']),tets);initial=volume(np.array(r['initial_positions']),tets)
 minimum=(float('inf'),None,None); totals=[]; weighted_rms=[]
 for start in range(0,len(positions),64):
  v=volume(positions[start:start+64,i],tets);j=v/ref;totals.extend(v.sum(axis=1)/initial.sum());weighted_rms.extend(np.sqrt(((j-1)**2*ref).sum(axis=1)/ref.sum()));loc=np.unravel_index(j.argmin(),j.shape)
  if j[loc]<minimum[0]:minimum=(float(j[loc]),int(start+loc[0]),int(loc[1]))
 j,step,tet=minimum
 report['regions'].append({'name':r['name'],'initial_total_volume_m3':float(initial.sum()),'rest_total_volume_m3':float(ref.sum()),'minimum_total_volume_ratio_to_initial':float(min(totals)),'maximum_total_volume_ratio_to_initial':float(max(totals)),'maximum_volume_weighted_rms_J_error':float(max(weighted_rms)),'minimum_element_J':j,'minimum_J_step':step+1,'minimum_J_tetrahedron':tet,'minimum_J_element_initial_J':float(initial[tet]/ref[tet]),'minimum_J_element_rest_volume_m3':float(ref[tet]),'minimum_J_element_fixed_nodes':sum(r['attachments'][n]['kind']=='fixed' for n in tets[tet]),'inverted_elements':False if j>0 else True})
(folder/'volume-audit.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
