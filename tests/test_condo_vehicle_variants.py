"""Check that paint variants preserve the original Blender vehicle geometry."""
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "vehicle_variants", ROOT / "scripts/condo/vehicle_variants.py")
VARIANTS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VARIANTS)


def unpack(path):
    raw = path.read_bytes()
    length = struct.unpack_from("<I", raw, 12)[0]
    return json.loads(raw[20:20 + length]), raw[20 + length:]


class VehicleVariantTests(unittest.TestCase):
    def test_paints_share_binary_geometry_and_preserve_all_node_transforms(self):
        source = ROOT / "scenes/toronto_condo/vehicle-base.glb"
        base, binary = unpack(source)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "vehicle.glb"
            count = VARIANTS.add_variants(source, output)
            result, result_binary = unpack(output)
        self.assertEqual(binary, result_binary)
        for key in ("accessors", "bufferViews", "buffers", "images", "textures", "samplers"):
            self.assertEqual(base.get(key), result.get(key), key)
        self.assertEqual(count, len(result["scenes"]))
        self.assertEqual(count, len({scene["name"] for scene in result["scenes"]}))
        colours = set()
        for scene in result["scenes"]:
            self.assertEqual(len(scene["nodes"]), len(base["nodes"]))
            for original, index in zip(base["nodes"], scene["nodes"]):
                node = result["nodes"][index]
                old_transform, new_transform = deepcopy(original), deepcopy(node)
                old_mesh, new_mesh = old_transform.pop("mesh"), new_transform.pop("mesh")
                self.assertEqual(old_transform, new_transform)
                old_primitives = base["meshes"][old_mesh]["primitives"]
                new_primitives = result["meshes"][new_mesh]["primitives"]
                self.assertEqual(len(old_primitives), len(new_primitives))
                for old, new in zip(old_primitives, new_primitives):
                    old, new = deepcopy(old), deepcopy(new)
                    old_mat, new_mat = old.pop("material"), new.pop("material")
                    self.assertEqual(old, new)
                    if base["materials"][old_mat]["name"] == "Vehicle paint":
                        colours.add(tuple(result["materials"][new_mat]
                                          ["pbrMetallicRoughness"]["baseColorFactor"]))
                    else:
                        self.assertEqual(base["materials"][old_mat], result["materials"][new_mat])
        self.assertEqual(count, len(colours))

    def test_rejects_variant_input_without_overwriting_it(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "vehicle.glb"
            VARIANTS.add_variants(ROOT / "scenes/toronto_condo/vehicle-base.glb", output)
            original = output.read_bytes()
            with self.assertRaisesRegex(AssertionError, "single-scene"):
                VARIANTS.add_variants(output, output)
            self.assertEqual(original, output.read_bytes())
