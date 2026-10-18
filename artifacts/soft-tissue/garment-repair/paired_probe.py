"""Repair probe using a shared middle surface and a reconstructed 2 mm shell."""
from pathlib import Path
from collections import deque
import sys,json,numpy as np
from scipy.sparse import coo_matrix
from scipy.spatial import cKDTree
sys.path.insert(0,str(Path.cwd()/'scripts/avatars'))
from build_tissue_asset import surfaces

folder=Path('artifacts/soft-tissue/garment-repair');root=folder.parent
shell=np.load(folder/'blender-shell.npz');p=shell['positions'];faces=shell['triangles'];n=len(p)//2;middle=(p[:n]+p[n:])*0.5;mid_faces=faces[faces.max(axis=1)<n]
source=next(s for s in surfaces(Path('scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb')) if s['name']=='Condo cream knit top');distance,index=cKDTree(p).query(source['positions']);assert distance.max()<1e-7
# Verify this correspondence reproduces the exact source positions, including UV splits.
snapshot=json.loads((root/'surface-snapshot-11.json').read_text());top=next(s for s in snapshot['surfaces'] if s['mesh_name']==source['name'])
bind=json.loads((root/'garment-bind-snapshot.json').read_text());bt=next(s for s in bind['surfaces'] if s['mesh_name']==source['name']);active=np.asarray(bt['affected_triangles'],int)
def edges_of(tri):return np.unique(np.sort(np.concatenate([tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]]]),axis=1),axis=0)
edges=edges_of(active);seed=set();audit=json.loads((root/'garment-bind-audit.json').read_text())
for edge,face in audit['states']['before_world']['self_intersection_crossings']:
 seed.update(index[edges[edge]]%n);seed.update(index[source['triangles'][face]]%n)
all_edges=edges_of(mid_faces);rows=np.concatenate([all_edges[:,0],all_edges[:,1]]);cols=np.concatenate([all_edges[:,1],all_edges[:,0]]);adj=coo_matrix((np.ones(len(rows)),(rows,cols)),shape=(n,n)).tocsr();degree=np.asarray(adj.sum(axis=1)).ravel();distance=np.full(n,1000);queue=deque(sorted(seed));distance[list(seed)]=0
while queue:
 i=queue.popleft()
 if distance[i]>=8:continue
 for j in adj.indices[adj.indptr[i]:adj.indptr[i+1]]:
  if distance[j]>distance[i]+1:distance[j]=distance[i]+1;queue.append(j)
w=np.clip(1-distance/8,0,1);w=w*w*(3-2*w);candidate=middle.copy();cycles=int(sys.argv[1]) if len(sys.argv)>1 else 8
for _ in range(cycles):
 for factor in [0.5,-0.53]:candidate+=factor*w[:,None]*(adj@candidate/degree[:,None]-candidate)
# Differential reconstruction keeps the original thickness direction outside
# the repair and uses the repaired surface normal within it.
def normals(points):
 face=np.cross(points[mid_faces[:,1]]-points[mid_faces[:,0]],points[mid_faces[:,2]]-points[mid_faces[:,0]]);normal=np.zeros_like(points)
 for j in range(3):np.add.at(normal,mid_faces[:,j],face)
 return normal/np.linalg.norm(normal,axis=1)[:,None]
normal=normals(candidate);original_direction=p[:n]-p[n:];original_direction/=np.linalg.norm(original_direction,axis=1)[:,None]
sign=np.sign(np.median(np.sum(normals(middle)*original_direction,axis=1)));normal*=sign
thickness_direction=(1-w[:,None])*original_direction+w[:,None]*normal;thickness_direction/=np.linalg.norm(thickness_direction,axis=1)[:,None]
paired=np.concatenate([candidate+.001*thickness_direction,candidate-.001*thickness_direction]);new=paired[index].astype(np.float32);delta=np.linalg.norm(new-source['positions'],axis=1)
palette=np.asarray(top['skin_palette'],np.float32).reshape(-1,4,4).transpose(0,2,1);weights=np.asarray(top['skin_weights'],np.float32);indices=np.asarray(top['skin_indices'],int);skin=(palette[indices]*weights[:,:,None,None]).sum(axis=1);world=np.einsum('nij,nj->ni',skin,np.column_stack([new,np.ones(len(new),np.float32)]))[:,:3]
for surface in snapshot['surfaces']:surface['after_world']=surface['before_world']
top['after_world']=world.tolist();snapshot['diagnostic']='Offline paired bind-shell repair under unchanged captured skinning matrices; no motion or GLB was modified.'
prefix=folder/f'paired-{cycles}';Path(str(prefix)+'.snapshot.json').write_text(json.dumps(snapshot,separators=(',',':'))+'\n');np.save(str(prefix)+'.positions.npy',new);np.save(str(prefix)+'.blender-positions.npy',paired)
report={'cycles':cycles,'middle_seed_vertices':len(seed),'modified_vertices':int((delta>1e-7).sum()),'maximum_displacement_m':float(delta.max()),'paired_thickness_min_max_m':[float(np.linalg.norm(paired[:n]-paired[n:],axis=1).min()),float(np.linalg.norm(paired[:n]-paired[n:],axis=1).max())]};Path(str(prefix)+'.report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
