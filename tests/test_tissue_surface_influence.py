"""A volumetric surface patch must not inherit bone-weight amplitude falloff."""
import sys
import unittest
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts/avatars"))
from build_tissue_asset import surface_influence


def patch():
    points = np.array([[x, y, 0.] for y in range(9) for x in range(9)]) * 0.01
    triangles = []
    for y in range(8):
        for x in range(8):
            a = y * 9 + x
            triangles.extend([[a, a + 1, a + 10], [a, a + 10, a + 9]])
    active = np.array([0 < x < 8 and 0 < y < 8 for y in range(9) for x in range(9)])
    return {"positions": points, "triangles": np.array(triangles), "weights": [active.astype(float)]}


class TissueSurfaceInfluenceTests(unittest.TestCase):
    def test_interior_follows_volume_and_boundary_is_continuous(self):
        surface = patch()
        w = surface_influence(surface, 0)
        self.assertEqual(w[4 * 9 + 4], 1.)
        self.assertEqual(w[4 * 9 + 1], 0.)
        self.assertEqual(w[4 * 9], 0.)
        self.assertAlmostEqual(w[4 * 9 + 2], 0.5)
        self.assertTrue(np.all((w >= 0) & (w <= 1)))

    def test_bone_weight_magnitudes_do_not_change_transfer(self):
        surface = patch()
        before = surface_influence(surface, 0)
        surface["weights"][0] *= np.linspace(0.002, 0.8, len(before))
        np.testing.assert_array_equal(before, surface_influence(surface, 0))

    def test_uv_seam_duplicates_share_geometry_influence(self):
        surface = patch()
        before = surface_influence(surface, 0)
        # Split one interior vertex into a second coincident index in one face.
        vertex = 4 * 9 + 2
        surface["positions"] = np.vstack([surface["positions"], surface["positions"][vertex]])
        surface["weights"][0] = np.append(surface["weights"][0], 0.)
        triangle = np.flatnonzero(np.any(surface["triangles"] == vertex, axis=1))[0]
        row = surface["triangles"][triangle]
        row[row == vertex] = len(before)
        after = surface_influence(surface, 0)
        np.testing.assert_array_equal(before, after[:-1])
        self.assertEqual(after[-1], before[vertex])


if __name__ == "__main__":
    unittest.main()
