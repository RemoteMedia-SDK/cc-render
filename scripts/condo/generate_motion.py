"""Capture one fixed-seed semantic Ardy run, preserving every native response.

Run with the existing SDK managed Python, its cached models and upstream on
PYTHONPATH. This neither relocates motion nor edits generated roots/joints.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import sys

parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--seconds', type=int, default=20)
parser.add_argument('--seed', type=int, default=731)
parser.add_argument('--description', default='A person turns around and chats casually while standing in place.')
args = parser.parse_args()
if not 2 <= args.seconds <= 120:
    parser.error('seconds must be between 2 and 120')
args.output.mkdir(parents=True, exist_ok=True)
os.environ['ARDY_JOURNAL_DIR'] = str(args.output / 'journal')
request = dict(kind='motion_goal', version=2, session_id='toronto-condo-visual-731',
               generation_revision=1, pts_ms=0, seed=args.seed, goal=dict(description=args.description))
(args.output/'request.json').write_text(json.dumps(request, indent=2))
with contextlib.redirect_stdout(sys.stderr):
    from ardy_worker import ArdySession, load_office_model
    service = ArdySession(load_office_model())
with (args.output/'responses.ndjson').open('w') as responses, (args.output/'packets.ndjson').open('w') as packets:
    while request['pts_ms'] < args.seconds*1000:
        try:
            with contextlib.redirect_stdout(sys.stderr):
                response = service.step(request)
            responses.write(json.dumps(response, allow_nan=False)+'\n'); responses.flush()
            if not response['frames']:
                raise RuntimeError('motion service returned no continuation')
            for frame in response['frames']:
                packets.write(json.dumps(frame, allow_nan=False)+'\n')
            packets.flush()
            request['pts_ms'] = response['frames'][-1]['pts_ms']
            print(json.dumps({'generated_until_ms':request['pts_ms'],'artifact':response['artifact_id']}),flush=True)
        except Exception as error:
            (args.output/'failure.json').write_text(json.dumps({'error':str(error),'request':request},indent=2))
            raise
