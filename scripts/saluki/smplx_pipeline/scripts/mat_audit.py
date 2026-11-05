"""Audit materials/textures referenced by Saluki .cast exports: missing files, empty materials, resolutions.

usage: python -I mat_audit.py <models_dir> [name_filter]
"""
import json
import os
import struct
import sys
from collections import Counter, defaultdict

from PIL import Image

SIZES = {b"b": 1, b"h": 2, b"i": 4, b"l": 8, b"f": 4, b"d": 8, b"2v": 8, b"3v": 12, b"4v": 16}
MATL, FILE = 0x6C74616D, 0x656C6966


def read_node(f, out):
    ident, _size, nhash, nprops, nchild = struct.unpack("<IIQII", f.read(24))
    props = {}
    for _ in range(nprops):
        pid, name_len, arr_len = struct.unpack("<HHI", f.read(8))
        pid = pid.to_bytes(2, "little").rstrip(b"\0")
        pname = f.read(name_len).decode()
        if pid == b"s":
            props[pname] = b"".join(iter(lambda: f.read(1), b"\0")).decode(errors="replace")
        elif pid == b"l" and arr_len == 1 and ident == MATL:
            props[pname] = struct.unpack("<Q", f.read(8))[0]
        else:
            f.seek(SIZES[pid] * arr_len, 1)
    node = {"id": ident, "hash": nhash, "props": props, "children": []}
    for _ in range(nchild):
        node["children"].append(read_node(f, out))
    if ident in (MATL, FILE):
        out.append(node)
    return node


def audit_cast(path):
    nodes = []
    with open(path, "rb") as f:
        _magic, _ver, nroot, _flags = struct.unpack("<IIII", f.read(16))
        for _ in range(nroot):
            read_node(f, nodes)
    mats = []
    for m in (n for n in nodes if n["id"] == MATL):
        files = {c["hash"]: c["props"].get("p", "") for c in m["children"] if c["id"] == FILE}
        slots = {k: files.get(v) for k, v in m["props"].items() if isinstance(v, int)}
        mats.append({"name": m["props"].get("n", "?"), "type": m["props"].get("t", "?"), "slots": slots})
    return mats


root = sys.argv[1]
flt = sys.argv[2] if len(sys.argv) > 2 else ""
summary = {"models": 0, "materials": 0, "materials_no_textures": 0, "slot_refs": 0,
           "slot_missing_file": 0, "slot_unresolved": 0}
res = Counter()
slot_names = Counter()
no_tex = defaultdict(list)
missing = defaultdict(list)
small_albedo = []
for d in sorted(os.listdir(root)):
    if flt not in d:
        continue
    p = os.path.join(root, d, f"{d}_LOD0.cast")
    if not os.path.isfile(p):
        continue
    summary["models"] += 1
    for m in audit_cast(p):
        summary["materials"] += 1
        if not m["slots"]:
            summary["materials_no_textures"] += 1
            no_tex[m["name"]].append(d)
        for slot, rel in m["slots"].items():
            summary["slot_refs"] += 1
            slot_names[slot] += 1
            if not rel:
                summary["slot_unresolved"] += 1
                continue
            fp = rel if os.path.isabs(rel) else os.path.join(root, d, rel)
            if not os.path.isfile(fp):
                summary["slot_missing_file"] += 1
                missing[d].append(rel)
                continue
            with Image.open(fp) as im:
                w, h = im.size
            res[max(w, h)] += 1
            if slot in ("albedo", "diffuse") and max(w, h) <= 512:
                small_albedo.append((d, m["name"], f"{w}x{h}"))

print(json.dumps({
    "summary": summary,
    "slot_names": dict(slot_names.most_common()),
    "resolution_max_side": dict(sorted(res.items())),
    "materials_without_textures": {k: len(v) for k, v in sorted(no_tex.items(), key=lambda x: -len(x[1]))[:25]},
    "missing_files_examples": {k: v[:3] for k, v in list(missing.items())[:5]},
    "small_albedo_examples": small_albedo[:15],
    "small_albedo_count": len(small_albedo),
}, indent=1))
