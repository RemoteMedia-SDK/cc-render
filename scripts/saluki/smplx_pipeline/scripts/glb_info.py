"""Summarise a GLB: meshes, morph-target names, skins. usage: python -I glb_info.py file.glb"""
import json, struct, sys
from collections import Counter
with open(sys.argv[1], "rb") as f:
    magic, ver, length = struct.unpack("<III", f.read(12))
    clen, ctype = struct.unpack("<II", f.read(8))
    g = json.loads(f.read(clen))
print("asset", g.get("asset"), "extensionsUsed", g.get("extensionsUsed"))
print("meshes", len(g["meshes"]), "nodes", len(g["nodes"]), "skins", len(g.get("skins", [])), "anims", len(g.get("animations", [])))
for s in g.get("skins", []):
    print(" skin joints", len(s["joints"]), [g["nodes"][j].get("name") for j in s["joints"][:12]])
for m in g["meshes"]:
    names = (m.get("extras") or {}).get("targetNames") or []
    nt = max((len(p.get("targets", [])) for p in m["primitives"]), default=0)
    verts = sum(g["accessors"][p["attributes"]["POSITION"]]["count"] for p in m["primitives"])
    print(f" mesh {m.get('name')!r}: prims {len(m['primitives'])} verts {verts} targets {nt}")
    if names:
        print("   ", len(names), names[:200])
