"""Read-only, conservative hinge sweep against authored condo collision boxes."""
import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np


def yaw(angle):
    c,s = math.cos(angle),math.sin(angle)
    return np.array([[c,0,s],[0,1,0],[-s,0,c]])


def separation(box, other):
    """Largest separating-axis gap for two upright oriented cuboids."""
    a,ah,rotation=box
    b,bh,other_rotation=other
    axes=[np.array([0.,1.,0.]),rotation[:,0],rotation[:,2],other_rotation[:,0],other_rotation[:,2]]
    return max(abs(np.dot(b-a,n))-np.dot(ah,abs(rotation.T@n))-
               np.dot(bh,abs(other_rotation.T@n)) for n in axes)


def obstacles(mechanism, scene):
    result=[s for s in scene['solids'] if s['id'] != mechanism['replaces_solid']]
    base=np.array(mechanism['placement'])
    for part in mechanism['parts']:
        if part['mass_kg'] is None:
            for collider in part['colliders']:
                result.append({**collider,'position':(base+part['position']+np.array(collider['position'])).tolist()})
    return result


def door_samples(mechanism):
    """Yield world boxes and conservative inter-sample movement bounds."""
    base=np.array(mechanism['placement'])
    for part in mechanism['parts']:
        if 'joint' not in part:continue
        lo,hi=part['joint']['limits_rad']
        intervals=max(1,math.ceil((hi-lo)/math.radians(.5)))
        step=(hi-lo)/intervals
        for angle in np.linspace(lo,hi,intervals+1):
            rotation=yaw(angle)
            for collider in part['colliders']:
                local=np.array(collider['position']);half=np.array(collider['half_extents'])
                position=base+part['position']+rotation@local
                # Every point's travel to the nearest sample is bounded by r*dt.
                radius=np.linalg.norm(local[[0,2]])+np.linalg.norm(half[[0,2]])
                margin=radius*step/2+0.001
                identity={'part':part['id'],'mesh':collider['id'],'degrees':math.degrees(angle)}
                yield identity,(position,half,rotation),margin


def sweep(mechanism, obstacles):
    hits=[];tested=0;minimum=float('inf');nearest=None
    boxes=[(o['id'],(np.array(o['position']),np.array(o['half_extents']),
            yaw(math.radians(o.get('yaw_deg',0))))) for o in obstacles]
    for identity,box,margin in door_samples(mechanism):
        for name,(position,half,rotation) in boxes:
            gap=separation(box,(position,half+margin,rotation))
            tested+=1
            if gap<minimum:minimum=gap;nearest=name
            if gap<=0 and len(hits)<16:
                hits.append({**identity,'obstacle':name,'separation_m':gap})
    return {'pass':not hits,'pair_checks':tested,'minimum_separating_gap_m':minimum,
            'nearest_obstacle':nearest,'first_collisions':hits,
            'limits':'Upright collider boxes with angular sampling movement bound plus 1 mm reserve; no avatar motion, grasp, contents or dynamic solver validation.'}


def check(root, scene):
    mechanism=json.loads((root/'mechanism.json').read_text())
    for part in mechanism['parts']:
        assert hashlib.sha256((root/part['geometry']).read_bytes()).hexdigest()==part['sha256']
        assert all(all(v>0 for v in c['half_extents']) for c in part['colliders'])
        if 'joint' in part:
            joint=part['joint']
            assert joint['axis']==[0,1,0] and joint['child_anchor']==[0,0,0]
            assert joint['parent_anchor']==part['position'] and joint['motor'] is None
    report=sweep(mechanism,obstacles(mechanism,scene))
    # A block in the actual moving-door volume must be detected.
    part=mechanism['parts'][-1];collider=part['colliders'][0]
    centre=np.array(mechanism['placement'])+part['position']+yaw(.5)@np.array(collider['position'])
    blocker={'id':'negative_control_blocker','position':centre.tolist(),'half_extents':[.06,.06,.06]}
    negative=sweep(mechanism,[blocker])
    assert not negative['pass'],'Obstacle negative control was missed'
    report['negative_control_rejected']=True
    return report


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mechanism_dir',type=Path);parser.add_argument('scene',type=Path);parser.add_argument('report',type=Path)
    args=parser.parse_args()
    report=check(args.mechanism_dir,json.loads(args.scene.read_text()))
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
    raise SystemExit(0 if report['pass'] else 2)
