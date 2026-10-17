"""Create paint variants of the Blender vehicle while sharing its binary data."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import struct


PAINTS = [
    ("Slate blue", [0.19, 0.28, 0.36]),
    ("Pearl", [0.72, 0.74, 0.72]),
    ("Graphite", [0.035, 0.042, 0.05]),
    ("Burgundy", [0.28, 0.035, 0.045]),
    ("Silver", [0.38, 0.41, 0.44]),
    ("Forest", [0.055, 0.15, 0.10]),
]


def copy_painted_meshes(document, originals, old_paint, new_paint):
    mapping = {}
    for index, mesh in enumerate(originals):
        if not any(p.get("material") == old_paint for p in mesh["primitives"]):
            continue
        clone = deepcopy(mesh)
        for primitive in clone["primitives"]:
            if primitive.get("material") == old_paint:
                primitive["material"] = new_paint
        mapping[index] = len(document["meshes"])
        document["meshes"].append(clone)
    return mapping


def add_variants(source, destination):
    raw = Path(source).read_bytes()
    magic, version, size, length, kind = struct.unpack_from("<4sIIII", raw)
    assert magic == b"glTF" and version == 2 and size == len(raw)
    assert kind == 0x4E4F534A
    document = json.loads(raw[20:20 + length])
    assert len(document["scenes"]) == 1, "Expected original single-scene Blender export"
    original_nodes = deepcopy(document["nodes"])
    assert all("children" not in node and "skin" not in node for node in original_nodes)
    assert document["scenes"][0]["nodes"] == list(range(len(original_nodes)))
    paint = next(i for i, mat in enumerate(document["materials"])
                 if mat["name"] == "Vehicle paint")
    original_meshes = deepcopy(document["meshes"])
    original_paint = deepcopy(document["materials"][paint])
    document["scenes"][0]["name"] = PAINTS[0][0]
    for name, colour in PAINTS[1:]:
        material = deepcopy(original_paint)
        material["name"] = "Vehicle paint - " + name
        material["pbrMetallicRoughness"]["baseColorFactor"] = colour + [1.0]
        paint_index = len(document["materials"])
        document["materials"].append(material)
        mesh_map = copy_painted_meshes(document, original_meshes, paint, paint_index)
        roots = []
        for node in original_nodes:
            clone = deepcopy(node)
            if "mesh" in clone:
                clone["mesh"] = mesh_map.get(clone["mesh"], clone["mesh"])
            roots.append(len(document["nodes"]))
            document["nodes"].append(clone)
        document["scenes"].append({"name": name, "nodes": roots})
    encoded = json.dumps(document, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    # Keep every original BIN byte and buffer accessor unchanged.
    tail = raw[20 + length:]
    result = struct.pack("<4sIIII", b"glTF", 2, 20 + len(encoded) + len(tail),
                         len(encoded), 0x4E4F534A) + encoded + tail
    Path(destination).write_bytes(result)
    return len(PAINTS)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    print(f"Created {add_variants(args.source, args.destination)} vehicle paint scenes")
