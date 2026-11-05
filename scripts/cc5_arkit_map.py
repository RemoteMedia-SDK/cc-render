"""Write the ARKit -> CC5 HD map for avatars baked from CC5's HD expression set.

    python scripts/cc5_arkit_map.py <expressions.json> <base arkit map> <out map>

The base map (e.g. remotemedia-sdk's beautiful_assistant.arkit_map.resolved.json) is kept as it is
except:

- `mouthSmile*` -> `Mouth_Corner_Pull_*` and `mouthFrown*` -> `Mouth_Corner_Depress_*`: CC5 HD's lip
  corner puller and depressor (FACS AU12 / AU15). `Mouth_Corner_Up/Down` only nudge the corner tips,
  and none of CC5's correctives are built on them.
- `correctives`: CC5's combination shapes (`C_<A>_<B>[_<C>]`), each to be driven at the product of
  its drivers' weights. Kept only when every driver is a morph this map drives. A driver token is
  matched to the base shape whose name ends with it once underscores are dropped (`StretchL` ->
  `Mouth_Stretch_L`, `CheekRaiseL` -> `Eye_Cheek_Raise_L`; of several, the one the map drives); a
  token that matches none, or several the map drives, drops the corrective.

- `jaw_open_deg`: 28, the jaw bone's turn at `jawOpen` 1 on CC5's rig (the renderer's default, 45,
  gapes); `scripts/saluki/smplx_to_glb.py` writes the operator's own.

`meshes` are not set here: the bakers (`bake_morphs`, `scripts/saluki/smplx_to_glb.py`) write the
meshes each morph ended up on into their output map. They also leave out correctives that move no
vertex 1 mm (CC5's eyelid ones, ~0.3 mm, and its `*_JawOpen` ones, which are empty): each morph
target costs GPU memory on every vertex of its mesh.
"""
import json
import sys

REMAP = {
    "mouthSmileLeft": "Mouth_Corner_Pull_L", "mouthSmileRight": "Mouth_Corner_Pull_R",
    "mouthFrownLeft": "Mouth_Corner_Depress_L", "mouthFrownRight": "Mouth_Corner_Depress_R",
}


def token_index(bases):
    """Every suffix of each base name, underscores dropped and a mid-name side letter moved last
    (Eye_L_Look_Down -> ...LookDownL) -> the base names it could mean."""
    index = {}
    for b in bases:
        segs = b.split("_")
        forms = [segs]
        side = [i for i, s in enumerate(segs) if s in ("L", "R") and i < len(segs) - 1]
        if side:
            i = side[0]
            forms.append(segs[:i] + segs[i + 1:] + [segs[i]])
        for f in forms:
            for k in range(len(f)):
                index.setdefault("".join(f[k:]), set()).add(b)
    return index


def correctives(names, driven):
    bases = [n for n in names if not n.startswith("C_")]
    index = token_index(bases)
    out, skipped = [], []
    for n in names:
        if not n.startswith("C_"):
            continue
        drivers = []
        # "RL" is both sides: CornerPullRL -> CornerPullR, CornerPullL
        toks = [t for tok in n[2:].split("_") for t in ((tok[:-2] + "R", tok[:-2] + "L") if tok.endswith("RL") else (tok,))]
        for tok in toks:
            hits = index.get(tok, set())
            if len(hits) > 1:  # StretchL: Mouth_Stretch_L or Neck_Stretch_L -> the one the map drives
                hits &= driven
            if len(hits) != 1:
                drivers = None
                break
            drivers.append(next(iter(hits)))
        if drivers and all(d in driven for d in drivers):
            out.append({"morph": n, "drivers": drivers})
        else:
            skipped.append(n)
    return out, skipped


def main():
    pack, base, dst = sys.argv[1:4]
    names = [e["name"] for e in json.load(open(pack, encoding="utf-8"))["expressions"]]
    src = json.load(open(base, encoding="utf-8"))
    mapping = {}
    for arkit, refs in src["mapping"].items():
        if arkit in REMAP:
            refs = [{"morph": REMAP[arkit], "weight": 1.0}]
        mapping[arkit] = [{"morph": r["morph"], "weight": r.get("weight", 1.0)} for r in refs]
    missing = sorted({r["morph"] for refs in mapping.values() for r in refs} - set(names))
    if missing:
        sys.exit(f"morphs not in the pack: {missing}")
    driven = {r["morph"] for refs in mapping.values() for r in refs}
    fixes, skipped = correctives(names, driven)
    doc = {
        "_comment": "ARKit-52 -> CC5 HD expression shapes, for avatars baked from CC5's HD set; "
                    "written by scripts/cc5_arkit_map.py. Correctives are driven at the product of "
                    "their drivers' weights.",
        "_source": base,
        "mapping": mapping,
        "correctives": fixes,
        # CC5's rig turns CC_Base_JawRoot 28 deg at jawOpen 1; its Jaw_Open morph only reshapes the
        # skin around that (alone it leaves the lips closed). Bakers for other rigs replace it.
        "jaw_open_deg": 28.0,
    }
    with open(dst, "w", encoding="utf-8", newline="\n") as f:
        json.dump(doc, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print(f"{len(mapping)} ARKit keys, {len(driven)} morphs, {len(fixes)} correctives "
          f"({len(skipped)} skipped: a driver isn't driven or doesn't resolve)")


if __name__ == "__main__":
    main()
