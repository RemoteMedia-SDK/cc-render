"""A tissue volume carries its surface and attaches at the surrounding chest."""
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np
import pyvista as pv

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts/avatars'))
from build_skin_tissue_volume import boundary_edges, close_and_mesh, skin_patch
from build_tissue_asset import embed


def dome():
    points = [[0., -.06, 0.]]
    angles = np.arange(16) * (2 * np.pi / 16)
    for radius in [.02, .04, .06, .08]:
        points.extend([[radius * np.cos(a), -.06 * np.sqrt(1 - (radius / .08) ** 2),
                        radius * np.sin(a)] for a in angles])
    faces = [[0, 1 + i, 1 + (i + 1) % 16] for i in range(16)]
    for ring in range(3):
        for i in range(16):
            a, b = 1 + ring * 16 + i, 1 + ring * 16 + (i + 1) % 16
            faces.extend([[a, a + 16, b + 16], [a, b + 16, b]])
    return pv.PolyData(np.array(points), np.column_stack([np.full(len(faces), 3), faces]))


class SkinVolumeTests(unittest.TestCase):
    def test_closed_volume_keeps_dome_free_and_chest_boundary_fixed(self):
        with tempfile.TemporaryDirectory() as directory:
            nodes, cells, fixed, shell, report = close_and_mesh(dome(), Path(directory) / 'shell.ply')
        self.assertTrue(shell.is_watertight)
        self.assertGreater(report['volume_m3'], 0)
        self.assertLessEqual(len(nodes), 512)
        self.assertFalse(fixed[nodes[:, 1].argmin()])
        self.assertTrue(np.all(fixed[nodes[:, 1] >= -1e-10]))
        tetra = nodes[cells]
        self.assertTrue(np.all(np.linalg.det((tetra[:, 1:] - tetra[:, :1]).transpose(0, 2, 1)) > 0))

    def test_disconnected_boundaries_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'multiple boundary loops'):
            boundary_edges(np.array([[0, 1, 2], [3, 4, 5]]))

    def test_collar_preserves_facial_morphs_and_their_normal_stencil(self):
        points = np.array([[.01 + x * .01, -.05, y * .01] for y in range(9) for x in range(9)])
        faces = []
        for y in range(8):
            for x in range(8):
                a = y * 9 + x
                faces.extend([[a, a + 1, a + 10], [a, a + 10, a + 9]])
        faces = np.asarray(faces)
        weights, morphed = np.zeros(81), np.zeros(81, dtype=bool)
        weights[4 * 9 + 4], morphed[4 * 9 + 7] = .1, True
        _, membership = skin_patch({'positions': points, 'triangles': faces,
                                     'weights': [weights], 'position_morphed': morphed}, 0)
        self.assertEqual(membership[4 * 9 + 4], 1.)
        stencil = np.unique(faces[np.any(membership[faces] > 0, axis=1)])
        self.assertFalse(np.any(morphed[stencil]))

    def test_direct_volume_binding_does_not_attenuate_low_bone_weights(self):
        positions = np.array([[.1, .1, .1], [.2, .1, .1]])
        body = {'positions': positions, 'anchor_inverse': np.eye(4),
                'weights': [np.array([.002, .9])]}
        region = {'initial_positions': [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                  'tetrahedra': [[0, 1, 2, 3]]}
        bindings, residual = embed(body, region, 0, influence=np.ones(2))
        self.assertEqual(residual, 0)
        np.testing.assert_array_equal([b['influence'] for b in bindings], [1., 1.])
        for binding, position in zip(bindings, positions):
            reconstructed = np.asarray(binding['barycentric']) @ np.asarray(region['initial_positions'])
            np.testing.assert_allclose(reconstructed, position)


if __name__ == '__main__':
    unittest.main()
