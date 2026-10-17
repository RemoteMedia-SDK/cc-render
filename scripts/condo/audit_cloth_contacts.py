"""Audit clock-matched distal-capsule observations; never certifies a pinch.

The Rust/ECS tests establish that the observer cannot write poses. This capture
audit checks published geometry, freshness and unchanged root interpolation.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

from audit_cloth_capture import check_observation, root_error


def inspect_hands(cloth):
    hands = cloth['hand_contacts']
    assert {h['hand'] for h in hands} == {'left', 'right'} and len(hands) == 2
    lengths, contacts = [], 0
    for hand in hands:
        assert hand['status'] == 'observed' and hand['error'] is None, hand
        assert {f['finger'] for f in hand['fingers']} == {'Thumb','Index','Mid','Ring','Pinky'}
        assert len(hand['fingers']) == 5
        for finger in hand['fingers']:
            values = finger['segment_start'] + finger['segment_end'] + [finger['radius_m']]
            assert all(math.isfinite(v) for v in values) and finger['radius_m'] > 0
            length = math.dist(finger['segment_start'], finger['segment_end'])
            assert length > 1e-6
            lengths.append(length)
            for witness in finger['contacts']:
                assert witness['surface_revision'] == cloth['surface_revision']
                assert witness['separation_m'] <= 0
                assert all(math.isfinite(v) for v in witness['position'] + witness['normal'])
                assert all(0 <= w <= 1 for w in witness['barycentric'])
                assert abs(sum(witness['barycentric']) - 1) < 1e-5
                contacts += 1
    return lengths, contacts


def audit(folder, expect_no_contact):
    states = [json.loads(line) for line in (folder/'states.ndjson').read_text().splitlines()]
    packets = [json.loads(line) for line in (folder/'packets.ndjson').read_text().splitlines()]
    times = [p['pts_ms'] for p in packets]
    lengths, errors = [], []
    contacts, observed, previous = 0, 0, None
    for state in states:
        office = state['office']
        assert office and office['motion_error'] is None
        cloth = next(c for c in state['cloths'] if c['id'] == 'bed_top_sheet')
        previous, _ = check_observation(cloth, previous)
        avatar = office['avatar']
        if not avatar.get('native_stream'):
            continue
        assert cloth['displayed_pts_ms'] == avatar['displayed_pts_ms']
        assert cloth['session_id'] == avatar['native_stream']['session_id']
        values, count = inspect_hands(cloth)
        lengths.extend(values)
        contacts += count
        observed += 1
        error = root_error(avatar, packets, times)
        if error is not None:
            errors.append(error)
    assert lengths and errors
    report = {'scope':__doc__, 'frames':len(states), 'observed_frames':observed,
              'finger_observations':len(lengths), 'contact_witnesses':contacts,
              'distal_segment_length_min_m':min(lengths), 'distal_segment_length_max_m':max(lengths),
              'max_root_interpolation_error_m':max(errors), 'expect_no_contact':expect_no_contact,
              'packet_sha256':hashlib.sha256((folder/'packets.ndjson').read_bytes()).hexdigest()}
    report['passes'] = max(errors) < 1e-5 and (not expect_no_contact or contacts == 0)
    with (folder/'hand-contact-audit.json').open('x') as stream:
        json.dump(report, stream, indent=2); stream.write('\n')
    print(json.dumps(report, indent=2))
    assert report['passes'], report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    parser.add_argument('--expect-no-contact', action='store_true')
    args = parser.parse_args()
    audit(args.capture, args.expect_no_contact)
