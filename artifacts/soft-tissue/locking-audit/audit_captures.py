from pathlib import Path
import sys,json,hashlib
import numpy as np
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'surface-volume-transfer'))
from audit_surface import capture,measure

def geometry(path):
 a=json.loads(path.read_text());assert a['source_pts_ms']==7633
 return next(s for s in a['surfaces'] if s['mesh_name']=='CC_Base_Body')

baseline,base_metrics=capture('lab-skin-volume-front-02')
ref=geometry(HERE.parent/'skin-volume/skin-volume-front-02-geometry.json')
rows=[]
for name,snapshot in [('lab-skin-volume-front-02',HERE.parent/'skin-volume/skin-volume-front-02-geometry.json'),('lab-nodal-volume-front-02',HERE/'nodal-front-02-geometry.json'),('lab-nodal-isochoric-front-01',HERE/'nodal-isochoric-front-01-geometry.json'),('lab-soft-nodal-isochoric-front-01',HERE/'soft-nodal-isochoric-front-01-geometry.json')]:
 trace,m=capture(name)
 m['maximum_volume_ratio']=max(g['metrics']['maximum_volume_ratio'] for r in trace for g in r['regions'])
 m['peak_source_ms']=max(trace,key=lambda r:r['maximum_surface_displacement_m'])['pts_ms']
 m['same_attachment_frames']=all(a['anchor_world_m']==b['anchor_world_m'] and a['anchor_rotation_xyzw']==b['anchor_rotation_xyzw'] for old,new in zip(baseline,trace,strict=True) for a,b in zip(old['regions'],new['regions'],strict=True))
 m['maximum_anchor_roundoff']={k:float(max(np.linalg.norm(np.asarray(a[k])-b[k]) for old,new in zip(baseline,trace,strict=True) for a,b in zip(old['regions'],new['regions'],strict=True))) for k in ['anchor_world_m','anchor_rotation_xyzw']}
 assert m['maximum_anchor_roundoff']['anchor_world_m'] < 1e-6
 assert m['maximum_anchor_roundoff']['anchor_rotation_xyzw'] < 2e-7
 m['geometry_at_7633_ms']=measure(ref,geometry(snapshot))[0]
 rows.append(m)
report={'candidates':rows,'visual_acceptance':False,'limitations':['Pressure averaging reduces small-strain locking but has excessive local volume variation; these are diagnostic candidates, not production acceptance.','Material sensitivity uses 500 Pa shear and 5 Pa s shear viscosity; this is not avatar-specific calibration.','Snapshot covers one source instant; there is no full-sequence intersection certificate.','Fixed-clock playback is offline, not evidence of real-time rendering.']}
(HERE/'capture-comparison.json').write_text(json.dumps(report,indent=2)+'\n')
for r in rows:print(r['capture'],'peak mm',round(r['maximum_displacement_mm'],2),'J',round(r['minimum_volume_ratio'],3),round(r['maximum_volume_ratio'],3),'area mean mm',round(r['geometry_at_7633_ms']['area_weighted_displacement_mm'],2),'faces over 90deg',r['geometry_at_7633_ms']['faces_rotated_over_90_degrees'])
