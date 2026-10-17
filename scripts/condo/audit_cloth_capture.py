"""Check observed live-cloth state, clock, mesh binding and unchanged root playback.

Scene-state sampling is asynchronous to GPU readback. Hash equality covers the
CPU mesh uploaded to rendering; inspect the recording separately for pixels.
"""
import bisect
import hashlib
import json
import math
from pathlib import Path
import sys


def check_observation(cloth,previous):
    assert cloth['error'] is None, cloth
    assert cloth['surface_position_sha256']==cloth['mesh_position_sha256'], cloth
    assert cloth['surface_revision']==cloth['simulated_ticks'], cloth
    if cloth['clock_origin_pts_ms'] is None:return None,0
    expected=(cloth['displayed_pts_ms']-cloth['clock_origin_pts_ms'])*60//1000
    assert cloth['simulated_ticks']==expected, cloth
    repeated=0
    if previous:
        assert cloth['simulated_ticks']>=previous['simulated_ticks']
        if cloth['simulated_ticks']==previous['simulated_ticks']:
            assert cloth['surface_position_sha256']==previous['surface_position_sha256']
            repeated=1
    return cloth,repeated


def root_error(avatar,packets,times):
    pts=avatar['displayed_pts_ms']
    if not avatar.get('native_stream') or pts<times[0]:return None
    i=max(0,bisect.bisect_right(times,pts)-1)
    a,b=packets[i],packets[min(i+1,len(packets)-1)]
    alpha=0 if a['pts_ms']==b['pts_ms'] else (pts-a['pts_ms'])/(b['pts_ms']-a['pts_ms'])
    expected=[x+(y-x)*alpha for x,y in zip(a['root_pos'],b['root_pos'])]
    return math.dist(expected,avatar['pelvis'])


def audit(folder):
    states=[json.loads(line) for line in (folder/'states.ndjson').read_text().splitlines()]
    packets=[json.loads(line) for line in (folder/'packets.ndjson').read_text().splitlines()]
    times=[p['pts_ms'] for p in packets]
    observations=[]
    root_errors=[]
    pauses=0
    previous=None
    for state in states:
        office=state['office']
        assert office and not office['motion_error'], office
        cloth=next(c for c in state['cloths'] if c['id']=='bed_top_sheet')
        observed, repeated=check_observation(cloth,previous)
        if observed:
            observations.append(observed)
            previous=observed
        pauses+=repeated
        error=root_error(office['avatar'],packets,times)
        if error is not None:root_errors.append(error)
    assert observations and root_errors
    end=observations[-1]['simulated_ticks']
    tail=[c for c in observations if c['simulated_ticks']>=end-30]
    report={'scope':__doc__,'frames':len(states),'observed_cloth_frames':len(observations),
        'simulated_ticks':end,'simulation_seconds':end/60,'unchanged_repeat_samples':pauses,
        'max_edge_stretch_ratio':max(c['max_edge_stretch_ratio'] for c in observations),
        'max_particle_penetration_m':max(c['max_particle_penetration_m'] for c in observations),
        'peak_particle_speed_m_s':max(c['max_speed_m_s'] for c in observations),
        'tail_max_speed_m_s':max(c['max_speed_m_s'] for c in tail),
        'max_root_interpolation_error_m':max(root_errors),'root_samples':len(root_errors),
        'packet_sha256':hashlib.sha256((folder/'packets.ndjson').read_bytes()).hexdigest(),
        'limits':['Fixed scene collider approximations only; no pillow, avatar or dynamic-body coupling.',
                  'No hand grasp, self-collision or continuous triangle collision acceptance.']}
    report['passes']=end>=1200 and pauses>0 and report['max_edge_stretch_ratio']<1.1 and report['max_particle_penetration_m']<.001 and report['tail_max_speed_m_s']<.05 and report['max_root_interpolation_error_m']<1e-5
    output=folder/'cloth-audit.json'
    with output.open('x') as stream:json.dump(report,stream,indent=2);stream.write('\n')
    print(json.dumps(report,indent=2))
    assert report['passes'],report


if __name__=='__main__':audit(Path(sys.argv[1]))
