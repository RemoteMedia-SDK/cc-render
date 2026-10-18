"""Read-only garment geometry audit of an actual Bevy tissue snapshot."""
from pathlib import Path
import sys,json,hashlib
import numpy as np
import trimesh
sys.path.insert(0,str(Path.cwd()/'scripts/avatars'))
from build_tissue_asset import read_glb,accessor
snapshot=json.loads(Path(sys.argv[1]).read_text())
document,binary=read_glb(Path('scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb'))

def triangles(surface):
 digest=hashlib.sha256(np.asarray(surface['base_positions'],dtype='<f4').tobytes()).hexdigest()
 for mesh in document['meshes']:
  for p in mesh['primitives']:
   values=accessor(document,binary,p['attributes']['POSITION'])
   if hashlib.sha256(values.astype('<f4').tobytes()).hexdigest()==digest:
    return accessor(document,binary,p['indices']).reshape(-1,3).astype(int)
 raise ValueError('surface fingerprint not found')

body=next(s for s in snapshot['surfaces'] if s['mesh_name']=='CC_Base_Body')
top=next(s for s in snapshot['surfaces'] if s['mesh_name']=='Condo cream knit top')
body_faces=triangles(body);top_faces=triangles(top)
active_faces=np.asarray(top['affected_triangles'],dtype=int);active=np.unique(active_faces)
edges=np.unique(np.sort(np.concatenate([active_faces[:,[0,1]],active_faces[:,[1,2]],active_faces[:,[2,0]]]),axis=1),axis=0)
report={'source_pts_ms':snapshot['source_pts_ms'],'active_top_vertices':len(active),'active_top_edges':len(edges),'trimesh_version':trimesh.__version__,'states':{}}

for state in ['before_world','after_world']:
 bp=np.asarray(body[state]);tp=np.asarray(top[state]);bm=trimesh.Trimesh(vertices=bp,faces=body_faces,process=False);tm=trimesh.Trimesh(vertices=tp,faces=top_faces,process=False)
 nearest,distance,face=trimesh.proximity.closest_point(bm,tp[active]); signed=np.sum((tp[active]-nearest)*bm.face_normals[face],axis=1)
 start=tp[edges[:,0]];segment=tp[edges[:,1]]-start;length=np.linalg.norm(segment,axis=1);direction=segment/length[:,None]
 locations,rays,faces=tm.ray.intersects_location(start,direction,multiple_hits=True)
 along=np.sum((locations-start[rays])*direction[rays],axis=1)
 valid=(along>1e-6)&(along<length[rays]-1e-6)
 valid &= np.all(top_faces[faces]!=edges[rays,0,None],axis=1)&np.all(top_faces[faces]!=edges[rays,1,None],axis=1)
 crossings=sorted(set((int(e),int(f)) for e,f in zip(rays[valid],faces[valid])))
 report['states'][state]={'body_signed_gap_percentiles_m':np.percentile(signed,[0,1,5,50,95,100]).tolist(),'top_vertices_inside_body':active[signed< -1e-5].tolist(),'self_intersection_crossings':crossings}
report['new_crossings']=sorted(set(map(tuple,report['states']['after_world']['self_intersection_crossings']))-set(map(tuple,report['states']['before_world']['self_intersection_crossings'])))
report['new_crossing_geometry']=[{'edge':edges[e].tolist(),'triangle':top_faces[f].tolist(),'rest_edge':np.asarray(top['base_positions'])[edges[e]].tolist()} for e,f in report['new_crossings'][:20]]
Path(sys.argv[2]).write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({key:{k:(len(v) if isinstance(v,list) and k!='body_signed_gap_percentiles_m' else v) for k,v in value.items()} for key,value in report['states'].items()},indent=2));print('new_crossings',len(report['new_crossings']))
