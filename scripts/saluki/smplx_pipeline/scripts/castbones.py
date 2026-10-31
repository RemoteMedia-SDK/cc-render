"""Report skeleton/bone/mesh counts for LOD0 .cast files under a folder."""
import os
import struct
import sys

SIZES = {b"b": 1, b"h": 2, b"i": 4, b"l": 8, b"f": 4, b"d": 8, b"2v": 8, b"3v": 12, b"4v": 16}
NODE_NAMES = {0x6C646F6D: "modl", 0x6C656B73: "skel", 0x656E6F62: "bone", 0x6873656D: "mesh"}


def read_node(f, counts, bone_names):
    ident, size, _hash, nprops, nchild = struct.unpack("<IIQII", f.read(24))
    kind = NODE_NAMES.get(ident)
    if kind:
        counts[kind] = counts.get(kind, 0) + 1
    for _ in range(nprops):
        pid, name_len, arr_len = struct.unpack("<HHI", f.read(8))
        pid = pid.to_bytes(2, "little").rstrip(b"\0")
        pname = f.read(name_len)
        if pid == b"s":
            s = b"".join(iter(lambda: f.read(1), b"\0"))
            if kind == "bone" and pname == b"n":
                bone_names.append(s.decode(errors="replace"))
        else:
            f.seek(SIZES[pid] * arr_len, 1)
    for _ in range(nchild):
        read_node(f, counts, bone_names)


def inspect(path):
    counts, bones = {}, []
    with open(path, "rb") as f:
        magic, _ver, nroot, _flags = struct.unpack("<IIII", f.read(16))
        if magic != 0x74736163:
            return None
        for _ in range(nroot):
            read_node(f, counts, bones)
    return counts, bones


root = sys.argv[1]
verbose = len(sys.argv) > 2
for name in sorted(os.listdir(root)):
    lod0 = os.path.join(root, name, f"{name}_LOD0.cast")
    if not os.path.isfile(lod0):
        print(f"{name:45} (no LOD0 yet)")
        continue
    try:
        counts, bones = inspect(lod0)
    except Exception as e:  # partially written file
        print(f"{name:45} read error: {e}")
        continue
    print(f"{name:45} bones={counts.get('bone', 0):4} meshes={counts.get('mesh', 0):3}  {', '.join(bones[:6])}")
    if verbose:
        print("   ", bones)
