from pathlib import Path
import sys,json
import numpy as np
sys.path.insert(0,str(Path.cwd()/'scripts/avatars'))
from build_tissue_asset import read_glb,accessor,surfaces
path=Path('scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb');doc,blob=read_glb(path);asset=json.loads(Path('assets/tissue/beautiful_assistant.fitted-prototype.json').read_text());result=[]
for surface in surfaces(path):
 binding=next(b for b in asset['surfaces'] if b['positions_sha256']==surface['positions_sha256'])
 positions=surface['positions'];_,groups=np.unique(np.round(positions,6),axis=0,return_inverse=True)
 node_count=sum(len(r['initial_positions']) for r in asset['regions']);coeff=np.zeros((len(positions),node_count)); offsets=np.cumsum([0]+[len(r['initial_positions']) for r in asset['regions']])
 for b in binding['vertices']:
  ids=asset['regions'][b['region']]['tetrahedra'][b['tetrahedron']]
  for i,w in zip(ids,b['barycentric']):coeff[b['vertex'],offsets[b['region']]+i]+=w*b['influence']
 duplicated=active=0;maximum=0.;bad=[]
 for group in range(int(groups.max())+1):
  ids=np.flatnonzero(groups==group)
  if len(ids)<2:continue
  duplicated+=1
  if np.any(coeff[ids]):
   active+=1;difference=float(np.max(np.abs(coeff[ids]-coeff[ids[0]])))
   maximum=max(maximum,difference)
   if difference>1e-6:bad.append({'vertices':ids.tolist(),'difference':difference,'position_spread_m':float(np.max(np.linalg.norm(positions[ids]-positions[ids[0]],axis=1)))})
 result.append({'surface':surface['name'],'vertices':len(positions),'coincident_groups_1um':duplicated,'active_groups':active,'maximum_embedding_coefficient_difference':maximum,'different_groups':bad[:10]})
print(json.dumps(result,indent=2))
