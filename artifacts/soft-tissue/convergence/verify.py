"""Compare a completed production compression trace with the archived reference."""
import json
from pathlib import Path
import numpy as np
folder=Path(__file__).resolve().parent
reference=folder.parent/"contact/base-32passes-120hz-k1e10.jsonl"
a=[json.loads(s) for s in (folder/"production-contact.jsonl").read_text().splitlines()]
b=[json.loads(s) for s in reference.read_text().splitlines()]
assert len(a)==len(b)==720
for x,y in zip(a,b):
    assert (x["region"],x["seconds"])==(y["region"],y["seconds"])
report=[]
for region in sorted({x["region"] for x in a}):
    pairs=[(x,y) for x,y in zip(a,b) if x["region"]==region]
    force=max(abs(x["contact"]["normal_force_n"]-y["contact"]["normal_force_n"]) for x,y in pairs)
    error=max(float(np.linalg.norm(np.asarray(x["node_positions"])-np.asarray(y["node_positions"]),axis=1).max()) for x,y in pairs if x["node_positions"])
    overlap=max(x["contact"]["maximum_penetration_m"] for x,y in pairs)
    assert force < 0.001,(region,"force",force)
    assert error < 0.00001,(region,"position",error)
    assert overlap < 0.0001,(region,"overlap",overlap)
    assert min(x["tissue"]["minimum_volume_ratio"] for x,y in pairs)>0.0
    report.append({"region":region,"max_force_difference_n":force,"max_checked_node_difference_m":error,
                   "maximum_penetration_m":overlap,"wall_seconds":pairs[-1][0]["wall_seconds"]})
(folder/"production-comparison.json").write_text(json.dumps(report,indent=2)+"\n")
print(json.dumps(report,indent=2))
