"""Fixed, preregistered native shower cases. No post-generation pose edits.
Uses the existing owner-approved session-start coordinate calibration.
"""
import contextlib
import json
import os
from pathlib import Path
import sys

output=Path(sys.argv[1])
output.mkdir(parents=True,exist_ok=False)
cases=[
    dict(name="standing",start="-5.5,-4.3,90",description="A person stands comfortably, moving their arms naturally while washing their hair.",seconds=8,seed=731),
]
if len(sys.argv) > 2:
    raise ValueError("Shower entry uses the normal navigate command; this generator records standing only")
(output/"attempt-plan.json").write_text(json.dumps(cases,indent=2)+"\n")
with contextlib.redirect_stdout(sys.stderr):
    from ardy_worker import ArdySession,load_office_model
    model=load_office_model()
for case in cases:
    folder=output/case["name"];folder.mkdir()
    os.environ["AVATAR_START"]=case["start"]
    os.environ["ARDY_JOURNAL_DIR"]=str(folder/"journal")
    with contextlib.redirect_stdout(sys.stderr):
        service=ArdySession(model)
    request=dict(kind="motion_goal",version=2,session_id="condo-water-"+case["name"],generation_revision=1,pts_ms=0,seed=case["seed"],goal=dict(description=case["description"]))
    (folder/"request.json").write_text(json.dumps(request,indent=2)+"\n")
    with (folder/"responses.ndjson").open("w") as responses,(folder/"packets.ndjson").open("w") as packets:
        while request["pts_ms"]<case["seconds"]*1000:
            with contextlib.redirect_stdout(sys.stderr):
                response=service.step(request)
            responses.write(json.dumps(response,allow_nan=False)+"\n");responses.flush()
            if not response["frames"]:raise RuntimeError("No native continuation")
            for frame in response["frames"]:packets.write(json.dumps(frame,allow_nan=False)+"\n")
            packets.flush();request["pts_ms"]=response["frames"][-1]["pts_ms"]
            print(json.dumps(dict(case=case["name"],generated_until=request["pts_ms"])),flush=True)
