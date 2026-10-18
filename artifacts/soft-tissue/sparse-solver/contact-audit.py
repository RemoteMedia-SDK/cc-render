from pathlib import Path
import sys, json, hashlib
import numpy as np
from scipy.spatial.transform import Rotation
sys.path.insert(0,str(Path.cwd()/'scripts/avatars'))
from build_tissue_asset import matrix_accessor, accessor, read_glb, anchor_space
path=Path('scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb')
doc,blob=read_glb(path)
body=next(n for n in doc['nodes'] if 'mesh' in n and doc['meshes'][n['mesh']].get('name')=='CC_Base_Body')
skin=doc['skins'][body['skin']]; names=[doc['nodes'][i].get('name','') for i in skin['joints']]
inverse=matrix_accessor(doc,blob,skin['inverseBindMatrices']); transforms=np.linalg.inv(inverse)
parents={child:i for i,node in enumerate(doc['nodes']) for child in node.get('children',[])}
cache={}
def global_matrix(i):
    if i in cache:return cache[i]
    node=doc['nodes'][i]
    if 'matrix' in node:local=np.array(node['matrix']).reshape(4,4).T
    else:
        local=np.eye(4);local[:3,:3]=Rotation.from_quat(node.get('rotation',[0,0,0,1])).as_matrix()@np.diag(node.get('scale',[1,1,1]));local[:3,3]=node.get('translation',[0,0,0])
    cache[i]=(global_matrix(parents[i]) if i in parents else np.eye(4))@local
    return cache[i]
mesh_inverse=np.linalg.inv(global_matrix(doc['nodes'].index(body)))
bone_frames={node.get('name',''):mesh_inverse@global_matrix(i) for i,node in enumerate(doc['nodes'])}
errors=[float(np.max(np.abs(bone_frames[n]-m))) for n,m in zip(names,transforms)]
assert max(errors)<1e-4, max(errors)
primitives=doc['meshes'][body['mesh']]['primitives']
positions=np.concatenate([accessor(doc,blob,p['attributes']['POSITION']) for p in primitives]).astype(float)
joints=np.concatenate([accessor(doc,blob,p['attributes']['JOINTS_0']) for p in primitives])
weights=np.concatenate([accessor(doc,blob,p['attributes']['WEIGHTS_0']) for p in primitives]).astype(float)
weights/=weights.sum(axis=1,keepdims=True)
def capsule_distance(points,a,b,r):
    delta=b-a; fraction=np.clip((points-a)@delta/(delta@delta),0,1)
    return np.linalg.norm(points-(a+fraction[:,None]*delta),axis=1)-r
report={'source':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'method':'Bind-space reconstruction of body_colliders::fitted_capsule using inverse bind matrices; skin coverage is diagnostic, not a fitted contact asset.','capsules':[]}
for side in 'LR':
 for bone,child in [('Upperarm','Forearm'),('Forearm','Hand')]:
    name=f'CC_Base_{side}_{bone}'; child=f'CC_Base_{side}_{child}'
    a=bone_frames[name][:3,3]; b=bone_frames[child][:3,3]
    radius=float(np.clip(np.linalg.norm(b-a)*.18,.02,.12))
    chosen=np.array([n.startswith(name) for n in names]); group=(weights*chosen[joints]).sum(axis=1)>.5
    d=capsule_distance(positions[group],a,b,radius)
    report['capsules'].append({'bone':name,'radius_m':radius,'dominant_skin_vertices':int(group.sum()),'outside_fraction':float((d>0).mean()),'maximum_skin_outside_m':float(d.max()),'signed_distance_percentiles_m':np.percentile(d,[0,50,90,100]).tolist()})
asset=json.loads(Path('assets/tissue/beautiful_assistant.fitted-prototype.json').read_text())
a=bone_frames['CC_Base_Spine02'][:3,3]; b=bone_frames['CC_Base_NeckTwist01'][:3,3];r=float(np.clip(np.linalg.norm(b-a)*.18,.02,.12))
report['chest_capsule_radius_m']=r
report['chest_footprints']=[]
for region in asset['regions']:
    fixed=np.array([v['kind']=='fixed' for v in region['attachments']]); local=np.array(region['initial_positions'])
    mesh=anchor_space(local,bone_frames[region['anchor_bone']]);dist=capsule_distance(mesh[fixed],a,b,r)
    report['chest_footprints'].append({'region':region['name'],'backing_nodes':int(fixed.sum()),'signed_capsule_gap_min_m':float(dist.min()),'signed_capsule_gap_max_m':float(dist.max()),'signed_capsule_gap_median_m':float(np.median(dist))})
print(json.dumps(report,indent=2))
