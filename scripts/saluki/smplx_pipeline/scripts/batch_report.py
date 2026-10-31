"""Collect every operator's checks/summary.json into one table + one review image.

usage: python -I batch_report.py
Writes SMPLX_ROOT/_report.md (table, outliers flagged) and SMPLX_ROOT/_review.png (per operator: neutral,
smile, jaw open, blink crops from the CC5 expression renders).
"""
import json
import os
import sys

from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402

rows, flagged = [], []
_inv_path = os.path.join(config.SMPLX_ROOT, "_inventory.json")
_inv = {(r["operator"][:-4] if r["operator"].endswith("_1_1") else r["operator"]): r.get("face_rig")
        for r in (json.load(open(_inv_path)) if os.path.exists(_inv_path) else [])}
ops = sorted(d for d in os.listdir(config.SMPLX_ROOT)
             if os.path.isfile(os.path.join(config.SMPLX_ROOT, d, "checks", "summary.json")))
for op in ops:
    s = json.load(open(os.path.join(config.SMPLX_ROOT, op, "checks", "summary.json")))
    rig, poses, cc5 = s.get("rig") or {}, s.get("poses") or {}, s.get("cc5") or {}
    face_rig = (s.get("models") or {}).get("face_rig") or _inv.get(op, "?")
    r = {
        "operator": op, "face_rig": face_rig,
        "bones": rig.get("total_bones"),
        "bad_weights": (rig.get("verts_bad_weight_sum") or 0) + (rig.get("verts_unweighted") or 0),
        "head_snap_cm": rig.get("head_snap_error_cm"),
        "roundtrip_max_cm": max((v["max_cm"] for v in (poses.get("round_trip_error") or {}).values()), default=None),
        "mocap": (s.get("mocap") or {}).get("frames"),
        "face_render": (s.get("face") or {}).get("render"),
        "landmarks": cc5.get("landmarks_used"),
        "sim_err_mm": (cc5.get("landmark_err_after_similarity_mm") or [None])[0],
        "shapes": cc5.get("shape_keys"),
        "lip_seam": cc5.get("lip_seam_surface_gap_mm"),
        "jaw_deg": (s.get("cc5_sheet") or {}).get("jaw_deg_matching_cc5_28deg"),
    }
    issues = []
    if r["bad_weights"]:
        issues.append("unweighted/bad weight verts")
    if r["head_snap_cm"] is not None and r["head_snap_cm"] > 0.2:
        issues.append("head snap > 2 mm")
    if r["roundtrip_max_cm"] is not None and r["roundtrip_max_cm"] > 0.1:
        issues.append("round trip > 1 mm")
    if not r["mocap"] and not (s.get("models") or {}).get("head_only"):
        issues.append("no mocap")
    if face_rig != "none":
        if not r["shapes"]:
            issues.append("no CC5 shapes")
        if r["landmarks"] is not None and r["landmarks"] < 200:
            issues.append("few landmarks (masked face?)")
        if r["sim_err_mm"] is not None and r["sim_err_mm"] > 8:
            issues.append("rough face alignment (check landmarks)")
    r["issues"] = "; ".join(issues)
    rows.append(r)
    if issues:
        flagged.append(op)

cols = list(rows[0].keys()) if rows else []
with open(os.path.join(config.SMPLX_ROOT, "_report.md"), "w") as f:
    f.write(f"# Operator batch report ({len(rows)} operators, {len(flagged)} flagged)\n\n")
    f.write("| " + " | ".join(cols) + " |\n|" + "---|" * len(cols) + "\n")
    for r in rows:
        f.write("| " + " | ".join("" if r[c] is None else str(r[c]) for c in cols) + " |\n")
print(f"{len(rows)} operators, flagged: {flagged}")

# review image: crops from the CC5 expression renders (operator side)
EXPR = [("neutral", (60, 120, 460, 520)), ("smile", (60, 200, 460, 600)), ("jaw_open", (60, 200, 460, 600)),
        ("blink", (60, 40, 460, 440))]
T = 200
with_cc5 = [op for op in ops if os.path.exists(os.path.join(config.SMPLX_ROOT, op, "checks", "cc5", "expr_neutral_op.png"))]
cols_n = 2  # operators per row
rows_n = (len(with_cc5) + cols_n - 1) // cols_n
W = cols_n * (len(EXPR) * T + 20)
img = Image.new("RGB", (W, rows_n * (T + 22)), "white")
d = ImageDraw.Draw(img)
try:
    font = ImageFont.truetype("arial.ttf", 15)
except OSError:
    font = ImageFont.load_default()
for i, op in enumerate(with_cc5):
    r, c = divmod(i, cols_n)
    x0, y0 = c * (len(EXPR) * T + 20), r * (T + 22)
    d.text((x0 + 4, y0 + 3), op + "   (" + ", ".join(e for e, _ in EXPR) + ")", fill="black", font=font)
    for j, (e, box) in enumerate(EXPR):
        p = os.path.join(config.SMPLX_ROOT, op, "checks", "cc5", f"expr_{e}_op.png")
        if os.path.exists(p):
            img.paste(Image.open(p).crop(box).resize((T, T)), (x0 + j * T, y0 + 22))
img.save(os.path.join(config.SMPLX_ROOT, "_review.png"))
print("wrote", os.path.join(config.SMPLX_ROOT, "_report.md"), "and _review.png")
