"""Orientation rig math; packets are generated, never rendered."""
import importlib.util
from pathlib import Path
import unittest

import numpy as np

spec = importlib.util.spec_from_file_location(
    'orientation_lab', Path(__file__).parents[1] / 'scripts/avatars/tissue_orientation_lab.py')
lab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lab)
POSE = dict(root_pos=[0.0, 0.945, 0.0], joint_quats_xyzw=[[0.0, 0.0, 0.0, 1.0]] * 22)


class OrientationRigTest(unittest.TestCase):
    def test_turns_rotate_the_body_about_the_pivot_and_return(self):
        for name in ['face-down', 'upside-down', 'side-left', 'turn', 'tumble']:
            packets = list(lab.packets(name, POSE))
            pivot = np.array(POSE['root_pos']) + lab.PIVOT_FROM_ROOT
            for packet in packets:
                root = np.array(packet['root_pos'])
                self.assertAlmostEqual(np.linalg.norm(root - pivot), np.linalg.norm(lab.PIVOT_FROM_ROOT), 9)
                self.assertEqual(packet['joint_quats_xyzw'][1:], POSE['joint_quats_xyzw'][1:])
            np.testing.assert_allclose(packets[-1]['root_pos'], POSE['root_pos'], atol=1e-9)
            self.assertAlmostEqual(abs(packets[-1]['joint_quats_xyzw'][0][3]), 1.0, 12)

    def test_face_down_points_her_face_at_the_floor(self):
        held = list(lab.packets('face-down', POSE))[round(4.0 * lab.FPS)]
        face = lab.rotate(np.array(held['joint_quats_xyzw'][0]), np.array([0.0, 0.0, 1.0]))
        np.testing.assert_allclose(face, [0, -1, 0], atol=1e-9)

    def test_side_right_puts_her_right_side_down(self):
        held = list(lab.packets('side-right', POSE))[round(4.0 * lab.FPS)]
        left = lab.rotate(np.array(held['joint_quats_xyzw'][0]), np.array([1.0, 0.0, 0.0]))
        np.testing.assert_allclose(left, [0, 1, 0], atol=1e-9)

    def test_shakes_translate_only_and_end_at_rest(self):
        packets = list(lab.packets('shake-side', POSE))
        offsets = [p['root_pos'][0] for p in packets]
        self.assertAlmostEqual(max(offsets), 0.03, 3)
        self.assertTrue(all(p['joint_quats_xyzw'] == POSE['joint_quats_xyzw'] for p in packets))
        self.assertEqual(packets[-1]['root_pos'], POSE['root_pos'])

    def test_timestamps_are_strictly_increasing_at_the_rig_rate(self):
        stamps = [p['pts_ms'] for p in lab.packets('turn', POSE)]
        self.assertEqual(stamps[0], 50)
        self.assertTrue(all(16 <= b - a <= 17 for a, b in zip(stamps, stamps[1:])))
        self.assertEqual(len(stamps), round(7.0 * lab.FPS) + 1)


if __name__ == '__main__':
    unittest.main()
