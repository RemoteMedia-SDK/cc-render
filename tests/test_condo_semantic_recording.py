"""Observer and provenance-selection tests; fixtures are never rendered."""
import copy
import importlib.util
import json
import math
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('condo_turn', Path(__file__).parents[1] / 'scripts/condo/generate_turn_conversation.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def frame(pts, angle, revision=1):
    c, s = math.cos(math.radians(angle)), math.sin(math.radians(angle))
    return dict(pts_ms=pts, generation_revision=revision,
                source_global_rot_mats=[[[c, 0, s], [0, 1, 0], [-s, 0, c]]])


class SemanticRecordingTests(unittest.TestCase):
    def test_observer_requires_dwell_and_does_not_modify_frames(self):
        frames = [frame(50, 0), frame(100, 170), frame(200, -175), frame(250, 0),
                  frame(300, 175), frame(400, -175), frame(500, 180)]
        original = copy.deepcopy(frames)
        self.assertIsNone(module.observed_turn(frames[:6]))
        self.assertEqual(module.observed_turn(frames), 500)
        self.assertEqual(frames, original)

    def test_service_anchor_selects_prefix_without_rewriting_any_frame(self):
        class Service:
            def __init__(self):
                self.end, self.requests, self.responses = 0, [], []

            def step(self, request):
                self.requests.append(copy.deepcopy(request))
                anchor = 2200 if request['generation_revision'] == 2 and len(self.requests) == 3 else self.end
                frames = [frame(t, 180 if t >= 1500 else 0, request['generation_revision'])
                          for t in range(anchor + 50, anchor + 2001, 50)]
                self.end = frames[-1]['pts_ms']
                result = dict(frames=frames, provenance=dict(history_end_pts_ms=anchor), artifact_id=str(len(self.requests)))
                self.responses.append(copy.deepcopy(result))
                return result

        service = Service()
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            module.record(service, out, 731, 10)
            actual = [json.loads(s) for s in (out / 'packets.ndjson').read_text().splitlines()]
            originals = [f for response in service.responses for f in response['frames']]
            self.assertTrue(all(f in originals for f in actual))
            self.assertEqual([f['pts_ms'] for f in actual], list(range(50, service.end + 1, 50)))
            self.assertEqual([f for f in actual if f['generation_revision'] == 1], originals[:44])
            self.assertEqual(len((out / 'responses.ndjson').read_text().splitlines()), len(service.responses))
        self.assertEqual(service.requests[1]['pts_ms'], 1700)
        self.assertEqual(service.requests[2]['pts_ms'], 1700)
        self.assertEqual(service.requests[2]['goal']['description'], 'A person chats casually while standing in place.')


if __name__ == '__main__':
    unittest.main()
