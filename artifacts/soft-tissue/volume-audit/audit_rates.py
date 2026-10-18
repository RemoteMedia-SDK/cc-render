"""Reuse the existing source-clock checks and compare every recorded volume node."""
from pathlib import Path
import json,sys,numpy as np
HERE=Path(__file__).resolve().parent;ROOT=HERE.parent
sys.path.insert(0,str(ROOT/'solver-runtime'))
from startup_cadence_audit import inspect as fixed
from live_clock_audit import inspect as live

source=(ROOT/'lab-jump-01/packets.ndjson').read_bytes()
original,a=fixed('lab-boundary-nodal-front-01',source)
opt,b=fixed('lab-boundary-opt240-01',source)
coarse,c=fixed('lab-boundary-opt120-01',source)
realtime=live('lab-boundary-live120-01',source)[0]

def compare(a,b):
 assert a.keys()==b.keys()
 node=[];surface=[];anchor=[]
 for t in a:
  surface.append(abs(a[t]['maximum_surface_displacement_m']-b[t]['maximum_surface_displacement_m']))
  for x,y in zip(a[t]['regions'],b[t]['regions'],strict=True):
   node.extend(np.linalg.norm(np.asarray(x['nodes_world_m'])-y['nodes_world_m'],axis=1))
   anchor.append(np.linalg.norm(np.asarray(x['anchor_world_m'])-y['anchor_world_m']))
 return {'frames':len(a),'max_node_difference_mm':float(max(node)*1000),'rms_node_difference_mm':float(np.sqrt(np.mean(np.asarray(node)**2))*1000),'max_surface_envelope_difference_mm':max(surface)*1000,'max_attachment_translation_difference_m':float(max(anchor))}

result={'runs':{'unoptimized_fixed240':original,'optimized_fixed240':opt,'optimized_fixed120':coarse,'live120':realtime},'optimization_parity':compare(a,b),'120_vs_240_hz':compare(b,c),'limits':['Single runs on a shared host, not a controlled performance benchmark.','Live timestamps measure GPU readback delivery, not browser presentation or end-to-end network latency.','Fixed comparisons include all volume nodes, but do not certify all intermediate source times.','No active avatar contacts in this jump; this is not a rendered compression test.']}
(HERE/'rate-comparison.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
