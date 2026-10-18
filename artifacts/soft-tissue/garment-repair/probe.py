"""Offline bind-mesh fairing experiment; never changes source motion or GLBs."""
from pathlib import Path
from collections import deque
import sys,json,numpy as np
from scipy.sparse import coo_matrix
sys.path.insert(0,str(Path.cwd()/'scripts/avatars'))
from build_tissue_asset import surfaces

root=Path('artifacts/soft-tissue');source=Path('scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb')
surface=next(s for s in surfaces(source) if s['name']=='Condo cream knit top')
p=surface['positions'].astype(float);faces=surface['triangles'];snapshot=json.loads((root/'surface-snapshot-11.json').read_text());top=next(s for s in snapshot['surfaces'] if s['mesh_name']==surface['name'])
bind=json.loads((root/'garment-bind-snapshot.json').read_text());bt=next(s for s in bind['surfaces'] if s['mesh_name']==surface['name']);active=np.asarray(bt['affected_triangles'],int)
def triangle_edges(tri):return np.unique(np.sort(np.concatenate([tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]]]),axis=1),axis=0)
edges=triangle_edges(active);all_edges=triangle_edges(faces)
audit=json.loads((root/'garment-bind-audit.json').read_text());seed=set()
for edge,face in audit['states']['before_world']['self_intersection_crossings']:
 seed.update(edges[edge]);seed.update(faces[face])
rows=np.concatenate([all_edges[:,0],all_edges[:,1]]);cols=np.concatenate([all_edges[:,1],all_edges[:,0]]);adj=coo_matrix((np.ones(len(rows)),(rows,cols)),shape=(len(p),len(p))).tocsr();degree=np.asarray(adj.sum(axis=1)).ravel();distance=np.full(len(p),1000);queue=deque(sorted(seed));distance[list(seed)]=0
while queue:
 i=queue.popleft()
 if distance[i]>=8:continue
 for j in adj.indices[adj.indptr[i]:adj.indptr[i+1]]:
  if distance[j]>distance[i]+1:distance[j]=distance[i]+1;queue.append(j)
w=np.clip(1-distance/8,0,1);w=w*w*(3-2*w);candidate=p.copy();cycles=int(sys.argv[1]) if len(sys.argv)>1 else 8
for _ in range(cycles):
 for factor in [0.5,-0.53]:candidate+=factor*w[:,None]*(adj@candidate/degree[:,None]-candidate)
candidate=candidate.astype(np.float32);delta=np.linalg.norm(candidate-p,axis=1)
palette=np.asarray(top['skin_palette'],dtype=np.float32).reshape(-1,4,4).transpose(0,2,1);weights=np.asarray(top['skin_weights'],np.float32);indices=np.asarray(top['skin_indices'],int);skin=(palette[indices]*weights[:,:,None,None]).sum(axis=1)
world=np.einsum('nij,nj->ni',skin,np.column_stack([candidate,np.ones(len(candidate),np.float32)]))[:,:3]
# This probe assesses authored geometry in the unchanged final pose, before tissue.
for s in snapshot['surfaces']:s['after_world']=s['before_world']
top['after_world']=world.tolist();snapshot['diagnostic']='Offline bind-mesh fairing evaluated under captured skinning matrices. No motion or GLB was modified.'
prefix=root/'garment-repair'/f'fair-{cycles}';Path(str(prefix)+'.snapshot.json').write_text(json.dumps(snapshot,separators=(',',':'))+'\n');np.save(str(prefix)+'.positions.npy',candidate)
report={'cycles':cycles,'seed_vertices':len(seed),'modified_vertices':int((delta>1e-9).sum()),'maximum_displacement_m':float(delta.max()),'displacement_percentiles_m':np.percentile(delta[delta>1e-9],[50,95,100]).tolist()};Path(str(prefix)+'.report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
