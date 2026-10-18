"""Meshing diagnostic on preserved shells; no runtime or authored-body changes."""
from pathlib import Path
import json,numpy as np,pyvista as pv,tetgen
from analyze import volumes,quality
HERE=Path(__file__).resolve().parent
rows=[]
for side in range(2):
 p=HERE.parent/'skin-volume'/f'skin-v2-unfitted-shell-{side}.ply'
 shell=pv.read(p)
 mesher=tetgen.TetGen(shell)
 nodes,cells=mesher.tetrahedralize(quality=True,minratio=1.8,mindihedral=10.,nobisect=False,steinerleft=256,quiet=True)
 v=volumes(nodes,cells);q=quality(nodes,cells)
 np.savez(HERE/f'quality-mesh-{side}.npz',nodes=nodes,cells=cells)
 rows.append({'side':side,'nodes':len(nodes),'tetrahedra':len(cells),'volume_m3':float(abs(v).sum()),'quality_percentiles':np.percentile(q,[0,1,5,50,95,100]).tolist(),'minimum_volume_mm3':float(abs(v).min()*1e9)})
 print(rows[-1],flush=True)
(HERE/'quality-mesh.json').write_text(json.dumps(rows,indent=2)+'\n')
