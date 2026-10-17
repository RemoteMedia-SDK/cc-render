"""Audit recorded weather continuity and original pelvis playback; plot observations."""
import argparse
import json
from pathlib import Path

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import numpy as np


def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def motion_error(folder):
    packets = rows(folder / 'packets.ndjson')
    timestamps = [p['pts_ms'] for p in packets]
    roots = np.asarray([p['root_pos'] for p in packets])
    errors = []
    for state in rows(folder / 'states.ndjson'):
        avatar = state['office']['avatar']
        timestamp = avatar['displayed_pts_ms']
        if timestamp is None or timestamp < timestamps[0]:
            continue
        expected = [np.interp(timestamp, timestamps, roots[:, axis]) for axis in range(3)]
        errors.append(float(np.max(abs(np.asarray(avatar['pelvis']) - expected))))
    assert errors and max(errors) < 5e-6, 'Observed pelvis differs from original packet interpolation'
    return {'states_checked': len(errors), 'maximum_observed_pelvis_error_m': max(errors)}


def request_evidence(requests, observations):
    evidence = []
    for request in requests:
        applied = [row for row in observations if row['request_id'] == request['request_id']]
        assert applied and all(row['target'] == request['weather'] for row in applied)
        intermediate = sum(0 < row['progress'] < 1 for row in applied)
        if request['transition_seconds'] >= 1:
            assert intermediate > 1, 'Transition has no intermediate observations'
        progress = np.asarray([row['progress'] for row in applied])
        assert (np.diff(progress) >= 0).all() and ((0 <= progress) & (progress <= 1)).all()
        evidence.append({'request_id': request['request_id'], 'target': request['weather'],
                         'intermediate_observations': intermediate,
                         'last_progress_before_replacement_or_end': float(progress[-1])})
    if requests:
        assert observations[-1]['request_id'] == requests[-1]['request_id']
        assert observations[-1]['progress'] == 1, 'Final weather did not settle within the capture'
    return evidence


def plot_observations(folder, samples):
    seconds = [s['encoded_frame'] / 30 for s in samples]
    fig, axes = plt.subplots(3, 1, figsize=(12, 7), sharex=True)
    for field in ['cloud_cover', 'night', 'rain', 'snow']:
        axes[0].plot(seconds, [s['weather'][field] for s in samples], label=field.replace('_', ' '))
    axes[0].set(ylabel='Blend / density', ylim=(-.05, 1.05))
    axes[0].legend(ncol=4)
    axes[1].plot(seconds, [s['weather']['sun_illuminance'] for s in samples], label='Sun illuminance (lux)')
    axes[1].plot(seconds, [s['weather']['environment_intensity'] for s in samples], label='Environment intensity')
    axes[1].set(yscale='log', ylabel='Intensity (log scale)')
    axes[1].legend()
    axes[2].plot(seconds, [s['weather']['wind_time'] for s in samples], label='Integrated wind phase')
    axes[2].set(xlabel='Encoded video seconds', ylabel='Wind phase (seconds)')
    for axis in axes:
        axis.grid(alpha=.2)
    fig.suptitle('Observed atmosphere — ' + folder.name)
    fig.text(.5, .015, 'Renderer-state observations may lead captured pixels by pipeline latency; review the video separately.',
             ha='center', fontsize=9)
    fig.tight_layout(rect=(0, .04, 1, .96))
    fig.savefig(folder / 'weather-observations.png', dpi=140)
    plt.close(fig)


def audit(folder):
    samples = rows(folder / 'weather-states.ndjson')
    observations = [s['weather'] for s in samples]
    assert observations and all(observations)
    requests = rows(folder / 'weather-requests.ndjson')
    elapsed = np.asarray([row['elapsed_seconds'] for row in observations])
    phase = np.asarray([row['wind_time'] for row in observations])
    wind = np.asarray([row['wind'] for row in observations])
    assert np.isfinite(elapsed).all() and np.isfinite(phase).all()
    assert (np.diff(elapsed) >= 0).all() and (np.diff(phase) >= 0).all(), 'Wind phase reset'
    phase_error = abs(np.diff(phase) - .5 * (wind[1:] + wind[:-1]) * np.diff(elapsed))
    assert phase_error.max() < .001, 'Wind phase jumped instead of integrating speed'
    result = {'requests': request_evidence(requests, observations),
              'maximum_wind_integral_error_seconds': float(phase_error.max()),
              'final_weather': observations[-1], **motion_error(folder),
              'limits': ['State observations, not direct GPU uniform or vertex readback.',
                         'Visual quality, wet surfaces, snow accumulation and long-run performance require separate evidence.']}
    (folder / 'weather-audit.json').write_text(json.dumps(result, indent=2) + '\n')
    plot_observations(folder, samples)
    print(json.dumps(result))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    audit(parser.parse_args().capture)
