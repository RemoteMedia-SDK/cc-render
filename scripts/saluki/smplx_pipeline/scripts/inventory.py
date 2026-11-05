"""Inventory of Saluki operator exports: pair each body with its best head and check both.

usage: python -I inventory.py [--json out.json]
For every rex_body_mp_<id> (the non-_fe body when both exist) the head candidates are rex_head_mp_<stem>*,
where <stem> is <id> without its _<n>_<m> skin suffix. Ranking:
  same skin + full face rig (~356 bones) > skin-less full rig (e.g. blix_high_fe) > same skin (simplified
  ~146-bone rig) > skin-less > another skin's head (flagged).
face_rig: "full", "simplified" (lips/lids/jaw present, fewer bones; often masked faces) or "none" (gas mask /
full helmet heads with only j_head + neck: body rig + mocap only, no CC5 expressions).
Checks: LOD0 present, required body joints, empty materials, missing image files.
"""
import json
import os
import re
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402

SIZES = {b"b": 1, b"h": 2, b"i": 4, b"l": 8, b"f": 4, b"d": 8, b"2v": 8, b"3v": 12, b"4v": 16}
BONE, MATL, FILE = 0x656E6F62, 0x6C74616D, 0x656C6966
BODY_NEED = ["j_mainroot", "j_spinelower", "j_spineupper", "j_spine4", "j_neck", "j_head"] + [
    f"j_{b}_{s}" for s in ("le", "ri")
    for b in ("hip", "knee", "ankle", "ball", "toe", "clavicle", "shoulder", "elbow", "wrist")] + [
    f"j_{f}_{s}_{i}" for s in ("le", "ri") for f in ("index", "mid", "ring", "pinky", "thumb") for i in (1, 2, 3)]
HEAD_NEED = ["j_spine4", "j_head", "j_jaw", "j_eyeball_le", "j_eyeball_ri", "j_lip_top", "j_lip_bot",
             "j_mouthcorner_le", "j_mouthcorner_ri", "j_nose_tip", "j_chin"]
SKIN_RE = re.compile(r"\d+_\d+")


def read_cast(path):
    bones, mats = [], []

    def node(f):
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
        kids = [node(f) for _ in range(nchild)]
        if ident == BONE:
            bones.append(props.get("n", ""))
        elif ident == MATL:
            files = {k["hash"]: k["props"].get("p", "") for k in kids if k["id"] == FILE}
            mats.append([files.get(v) for k, v in props.items() if isinstance(v, int)])
        return {"id": ident, "hash": nhash, "props": props}

    with open(path, "rb") as f:
        _magic, _ver, nroot, _flags = struct.unpack("<IIII", f.read(16))
        for _ in range(nroot):
            node(f)
    return bones, mats


def check(folder):
    p = os.path.join(config.MODELS, folder, f"{folder}_LOD0.cast")
    if not os.path.isfile(p):
        return None
    bones, mats = read_cast(p)
    missing = sum(1 for m in mats for rel in m
                  if rel and not rel.startswith("$") and not os.path.isfile(os.path.join(config.MODELS, folder, rel)))
    return {"bones": set(bones), "n_bones": len(bones), "materials": len(mats),
            "empty_materials": sum(1 for m in mats if not m), "missing_images": missing}


def inventory():
    folders = sorted(d for d in os.listdir(config.MODELS) if os.path.isdir(os.path.join(config.MODELS, d)))
    bodies = [d for d in folders if d.startswith("rex_body_mp_") and not d.endswith("_old")]
    heads = [d for d in folders if d.startswith("rex_head_mp_") and not d.endswith("_old")]
    ids = sorted({re.sub(r"_fe$", "", d[len("rex_body_mp_"):]) for d in bodies})
    out = []
    for op_id in ids:
        body = f"rex_body_mp_{op_id}" if f"rex_body_mp_{op_id}" in bodies else f"rex_body_mp_{op_id}_fe"
        m = re.match(r"^(.*?)_(\d+_\d+)$", op_id)
        stem, skin = (m.group(1), m.group(2)) if m else (op_id, None)
        rec = {"operator": op_id, "body": body, "head": None, "problems": [], "warnings": []}
        b = check(body)
        if b is None:
            rec["problems"].append("body has no LOD0")
        else:
            miss = [x for x in BODY_NEED if x not in b["bones"]]
            if miss:
                rec["problems"].append(f"body missing {len(miss)} joints: {miss[:5]}")
            rec.update(body_bones=b["n_bones"], body_empty_mats=b["empty_materials"],
                       body_missing_imgs=b["missing_images"])
        cands = []
        for h in heads:
            rest = h[len("rex_head_mp_"):]
            if not (rest == stem or rest.startswith(stem + "_")):
                continue
            hc = check(h)
            if hc is None:
                continue
            face = all(x in hc["bones"] for x in HEAD_NEED)
            full = face and hc["n_bones"] > 300
            same_skin = bool(skin) and f"_{skin}_" in rest + "_"
            skinless = not SKIN_RE.search(rest)
            tier = 4 if same_skin and full else 3 if skinless and full else 2 if same_skin else 1 if skinless else 0
            cands.append(((face, tier, hc["n_bones"]), h, hc, full, tier))
        cands.sort(key=lambda c: c[0], reverse=True)
        if cands:
            (face, _, _), h, hc, full, tier = cands[0]
            rec.update(head=h, head_bones=hc["n_bones"], face_rig="full" if full else "simplified" if face else "none",
                       head_empty_mats=hc["empty_materials"], head_missing_imgs=hc["missing_images"],
                       head_candidates=[c[1] for c in cands])
            if tier == 0:
                rec["warnings"].append("head taken from another skin")
            if not face:
                rec["warnings"].append("no face rig (mask/helmet head): body rig + mocap only, no CC5")
        else:
            rec["problems"].append("no head export")
        for k in ("body_empty_mats", "head_empty_mats", "body_missing_imgs", "head_missing_imgs"):
            if rec.get(k):
                rec["problems"].append(f"{k}={rec[k]}")
        out.append(rec)
    used = {r["head"] for r in out}
    stems = {re.sub(r"_\d+_\d+$", "", r["operator"]) for r in out}
    orphans = [h for h in heads if h not in used and not any(h[len("rex_head_mp_"):].startswith(st) for st in stems)]
    return out, orphans


if __name__ == "__main__":
    inv, orphans = inventory()
    for r in inv:
        flag = "!! " if r["problems"] else "ok "
        print(f"{flag}{r['operator']:28} {r['body'][12:]:28} {str(r['head'])[12:]:34} {r.get('face_rig', '-'):10} "
              f"bones {r.get('body_bones', '-')}/{r.get('head_bones', '-')}  {'; '.join(r['problems'] + r['warnings'])}")
    if orphans:
        print("\nheads with no matching body:", [h[12:] for h in orphans])
    n_bad = sum(bool(r["problems"]) for r in inv)
    print(f"\n{len(inv) - n_bad} ready ({sum(r.get('face_rig') == 'full' for r in inv)} full face rig, "
          f"{sum(r.get('face_rig') == 'simplified' for r in inv)} simplified, "
          f"{sum(r.get('face_rig') == 'none' for r in inv)} no face), {n_bad} with problems")
    if "--json" in sys.argv:
        json.dump(inv, open(sys.argv[sys.argv.index("--json") + 1], "w"), indent=1)
