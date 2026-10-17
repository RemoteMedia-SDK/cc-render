"""Offline semantic handoff through MotionService; never edits generated poses.

The observer only decides when to request a new semantic goal. MotionService
chooses the retained native-history boundary and generates the continuation.
Every response, including superseded lookahead, remains in responses.ndjson.
"""
import argparse
import contextlib
import json
import math
import os
from pathlib import Path
import sys


def observed_turn(frames):
    """First source-time dwell facing opposite the initial direction, or None."""
    first = frames[0]['source_global_rot_mats'][0]
    initial = math.atan2(first[0][2], first[2][2])
    since = None
    for frame in frames:
        rotation = frame['source_global_rot_mats'][0]
        angle = math.atan2(rotation[0][2], rotation[2][2]) - initial
        if math.cos(angle) > -math.cos(math.radians(30)):
            since = None
            continue
        since = frame['pts_ms'] if since is None else since
        if frame['pts_ms'] - since >= 200:
            return frame['pts_ms']
    return None


def record(service, output, seed, seconds):
    request = dict(kind='motion_goal', version=2, session_id='toronto-condo-turn-conversation',
                   generation_revision=1, pts_ms=0, seed=seed,
                   goal=dict(description='A person turns around.'))
    committed, responses, observed = [], [], None
    with (output / 'requests.ndjson').open('w') as requests, (output / 'responses.ndjson').open('w') as log:
        def step():
            requests.write(json.dumps(request) + '\n'); requests.flush()
            with contextlib.redirect_stdout(sys.stderr):
                response = service.step(request)
            log.write(json.dumps(response, allow_nan=False) + '\n'); log.flush()
            if not response['frames']:
                raise RuntimeError('Motion service returned no continuation')
            responses.append(response)
            return response

        while observed is None:
            response = step()
            committed.extend(response['frames'])
            observed = observed_turn(committed)
            if observed is None and committed[-1]['pts_ms'] >= 8000:
                raise RuntimeError('Turn observer did not complete within the declared 8-second budget')
            request['pts_ms'] = observed if observed is not None else committed[-1]['pts_ms']

        # Existing service semantics need generated lookahead beyond displayed time.
        # The observer is read-only; no pose or boundary is chosen by this script.
        if committed[-1]['pts_ms'] < observed + 600:
            committed.extend(step()['frames'])
        request['generation_revision'] = 2
        request['goal'] = dict(description='A person chats casually while standing in place.')
        continuation = step()
        anchor = continuation['provenance']['history_end_pts_ms']
        committed = [frame for frame in committed if frame['pts_ms'] <= anchor]
        committed.extend(continuation['frames'])
        (output / 'handoff.json').write_text(json.dumps({
            'source_observed_completion_pts_ms': observed,
            'service_history_end_pts_ms': anchor,
            'continuation_artifact': continuation['artifact_id'],
            'scope': 'Source observer only; no rendered completion or contact claim.'}, indent=2) + '\n')
        while committed[-1]['pts_ms'] < seconds * 1000:
            request['pts_ms'] = committed[-1]['pts_ms']
            committed.extend(step()['frames'])
    (output / 'packets.ndjson').write_text(''.join(json.dumps(f, allow_nan=False) + '\n' for f in committed))
    return {'packets': len(committed), 'responses': len(responses),
            'end_pts_ms': committed[-1]['pts_ms'], 'handoff_pts_ms': anchor}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--seed', type=int, default=731)
    parser.add_argument('--seconds', type=int, default=20)
    args = parser.parse_args()
    if not 10 <= args.seconds <= 120:
        parser.error('seconds must be between 10 and 120')
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / 'attempt-plan.json').write_text(json.dumps({
        'seed': args.seed, 'seconds': args.seconds, 'attempts': 1,
        'turn_budget_seconds': 8, 'observer_cone_degrees': 30, 'observer_dwell_ms': 200,
        'goal_sequence': ['A person turns around.', 'A person chats casually while standing in place.'],
        'policy': 'Semantic goals only; observer values never condition the model.'}, indent=2) + '\n')
    os.environ['ARDY_JOURNAL_DIR'] = str(args.output / 'journal')
    try:
        with contextlib.redirect_stdout(sys.stderr):
            from ardy_worker import ArdySession, load_office_model
            service = ArdySession(load_office_model())
        print(json.dumps(record(service, args.output, args.seed, args.seconds)), flush=True)
    except Exception as error:
        (args.output / 'failure.json').write_text(json.dumps({'error': str(error)}, indent=2) + '\n')
        raise


if __name__ == '__main__':
    main()
