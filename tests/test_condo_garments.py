"""The condo garments split from the v13 outfit are exact, standalone and reproducible."""
from copy import deepcopy
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts/condo"))

import check_garments  # noqa: E402
from outfit_package import read_glb  # noqa: E402
import split_outfit  # noqa: E402

OUTFIT = ROOT / "scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb"
GARMENTS = ROOT / "scenes/toronto_condo/garments"
AVATAR = ROOT.parent / "remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb"


@unittest.skipUnless(AVATAR.exists(), "clothing-free source avatar is not checked out beside cc-render")
class CondoGarmentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.avatar = read_glb(AVATAR)
        cls.outfit = read_glb(OUTFIT)

    def test_committed_garments_pass_the_audit(self):
        report = check_garments.check_garments(AVATAR, OUTFIT, GARMENTS)
        self.assertEqual(report["accounting"]["added_meshes"], 2)
        for garment in report["garments"].values():
            self.assertEqual(garment["bind_pose_max_position_error_m"], 0.0)
            self.assertEqual(garment["structure"]["max_nonzero_influences"], 4)

    def test_splitter_reproduces_committed_files(self):
        with tempfile.TemporaryDirectory() as directory:
            results = split_outfit.split_outfit(OUTFIT, directory)
            self.assertEqual([r["files"] for r in results], [["written", "written"]] * 2)
            for garment in split_outfit.GARMENTS:
                for name in (f"{garment['id']}.glb", "garment.json"):
                    self.assertEqual((Path(directory) / garment["id"] / name).read_bytes(),
                                     (GARMENTS / garment["id"] / name).read_bytes(), name)
            again = split_outfit.split_outfit(OUTFIT, directory)
            self.assertEqual([r["files"] for r in again], [["unchanged", "unchanged"]] * 2)

    def audit(self, document, binary):
        node = next(n for n, spec in enumerate(self.outfit[0]["nodes"]) if spec.get("name") == "Condo cream knit top")
        return check_garments.check_garment(*self.outfit, *self.avatar, document, binary, node)

    def test_audit_rejects_mutated_garments(self):
        document, binary = read_glb(GARMENTS / "condo_knit_tank_top/condo_knit_tank_top.glb")
        self.audit(document, binary)
        position = document["bufferViews"][document["accessors"][
            document["meshes"][0]["primitives"][0]["attributes"]["POSITION"]]["bufferView"]]["byteOffset"]
        flipped = bytearray(binary)
        flipped[position] ^= 1
        swapped = deepcopy(document)
        joints = swapped["skins"][0]["joints"]
        joints[1], joints[2] = joints[2], joints[1]
        moved = deepcopy(document)
        bone = next(n for n in moved["nodes"] if n["name"] == "CC_Base_Spine01")
        bone["translation"] = [bone["translation"][0] + 1e-5, *bone["translation"][1:]]
        for candidate, payload in [(document, bytes(flipped)), (swapped, binary), (moved, binary)]:
            with self.assertRaises(AssertionError):
                self.audit(candidate, payload)

    def test_accounting_rejects_a_missing_garment(self):
        top = next(n for n, spec in enumerate(self.outfit[0]["nodes"]) if spec.get("name") == "Condo cream knit top")
        with self.assertRaises(AssertionError):
            check_garments.check_accounting(*self.avatar, *self.outfit, [top])


if __name__ == "__main__":
    unittest.main()
