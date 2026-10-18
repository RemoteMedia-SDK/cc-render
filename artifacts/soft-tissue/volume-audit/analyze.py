"""Audit actual rendered node positions; no replay approximation or pose mutation."""
from pathlib import Path
import json,argparse
import numpy as np

HERE=Path(__file__).resolve().parent
ROOT=HERE.parent
ASSET=ROOT.parent.parent/'assets/tissue/beautiful_assistant.base-soft-nodal-isochoric-volume-prototype.json'


def volumes(points, cells):
    p=points[cells]
    return np.linalg.det((p[:,1:]-p[:,:1]).transpose(0,2,1))/6


def quality(points, cells):
    p=points[cells]
    edge_sum=sum(np.sum((p[:,a]-p[:,b])**2,axis=1) for a in range(4) for b in range(a+1,4))
    return 12*(3*abs(volumes(points,cells)))**(2/3)/edge_sum


def analyze(region, index, traces, bindings):
    rest=np.asarray(region['rest_positions']);initial=np.asarray(region['initial_positions'])
    cells=np.asarray(region['tetrahedra']);v=volumes(rest,cells)
    assert np.all(v>0)
    node_volume=np.zeros(len(rest))
    for k in range(4):np.add.at(node_volume,cells[:,k],v/4)
    used=np.bincount([b['tetrahedron'] for b in bindings if b['region']==index],minlength=len(cells))
    initial_quality=quality(initial,cells);rest_quality=quality(rest,cells)
    worst_score=np.zeros(len(cells));worst_ratio=np.ones(len(cells));worst_time=np.zeros(len(cells),dtype=int)
    frames=[]
    for row in traces:
        r=row['regions'][index];points=np.asarray(r['nodes_world_m'])
        current=volumes(points,cells);j=current/v
        assert abs(j.min()-r['metrics']['minimum_volume_ratio'])<1e-8
        assert abs(j.max()-r['metrics']['maximum_volume_ratio'])<1e-8
        node_current=np.zeros(len(rest))
        for k in range(4):np.add.at(node_current,cells[:,k],current/4)
        nodal_j=node_current/node_volume
        score=abs(np.log(j));changed=score>worst_score
        worst_score[changed]=score[changed];worst_ratio[changed]=j[changed];worst_time[changed]=row['pts_ms']
        frames.append({'pts_ms':row['pts_ms'],'total_volume_ratio':float(current.sum()/v.sum()),'element_J_rms':float(np.sqrt(np.average((j-1)**2,weights=v))),'reference_volume_fraction_outside_20_percent':float(v[abs(j-1)>.2].sum()/v.sum()),'reference_volume_fraction_outside_50_percent':float(v[abs(j-1)>.5].sum()/v.sum()),'minimum_nodal_J':float(nodal_j.min()),'maximum_nodal_J':float(nodal_j.max())})
    ranked=np.argsort(worst_score)[-12:][::-1]
    worst=[{'tetrahedron':int(i),'worst_J':float(worst_ratio[i]),'pts_ms':int(worst_time[i]),'rest_volume_mm3':float(v[i]*1e9),'initial_quality':float(initial_quality[i]),'fitted_rest_quality':float(rest_quality[i]),'bound_surface_vertices':int(used[i]),'fixed_nodes':sum(region['attachments'][n]['kind']=='fixed' for n in cells[i]),'nodes':cells[i].tolist(),'initial_centroid_m':initial[cells[i]].mean(axis=0).tolist()} for i in ranked]
    return {'region':region['name'],'rest_volume_m3':float(v.sum()),'quality_percentiles':[np.percentile(rest_quality,[0,1,5,50,95,100]).tolist()],'max_rms_error':max(frames,key=lambda r:r['element_J_rms']),'max_volume_fraction_error':max(frames,key=lambda r:r['reference_volume_fraction_outside_20_percent']),'global_volume_range':[min(r['total_volume_ratio'] for r in frames),max(r['total_volume_ratio'] for r in frames)],'worst_elements':worst,'frames':frames}


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--asset',type=Path,default=ASSET)
    parser.add_argument('--capture',default='lab-soft-volume-nodes-01')
    parser.add_argument('--output',default='dynamic-volume-report.json')
    parser.add_argument('--unchanged-deformation',action='store_true')
    args=parser.parse_args()
    asset=json.loads(args.asset.read_text());folder=ROOT/args.capture
    traces=[json.loads(l) for l in (folder/'tissue.ndjson').read_text().splitlines()]
    baseline=[json.loads(l) for l in (ROOT/'lab-soft-nodal-isochoric-front-01/tissue.ndjson').read_text().splitlines()]
    assert len(traces)==len(baseline)==478
    assert [r['pts_ms'] for r in traces]==[r['pts_ms'] for r in baseline]
    delta=max(abs(a['maximum_surface_displacement_m']-b['maximum_surface_displacement_m']) for a,b in zip(traces,baseline))
    if args.unchanged_deformation:assert delta<1e-6
    assert (folder/"packets.ndjson").read_bytes()==(ROOT/"lab-soft-nodal-isochoric-front-01/packets.ndjson").read_bytes()
    out={'frames':len(traces),'maximum_surface_displacement_difference_m':delta,'regions':[analyze(r,i,traces,asset['surfaces'][0]['vertices']) for i,r in enumerate(asset['regions'])]}
    (HERE/args.output).write_text(json.dumps(out,indent=2)+'\n')
    for r in out['regions']:
        print(r['region'],'global volume range',r['global_volume_range'],'worst volume fraction',r['max_volume_fraction_error'])
        print('worst elements',r['worst_elements'][:3])
