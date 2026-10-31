"""Run the CoD operator -> SMPL-X (+ CC5 expressions) pipeline for one operator.

usage:
  python run_operator.py <operator> [--steps a,b] [--from STEP] [--body NAME] [--head NAME] [--force]
  python run_operator.py --all [--skip-done] [--steps ...]      every operator in the export folder
  python run_operator.py <operator> --head-only [--head NAME]     the head alone, no body -> smplx/<operator>_head/
  python scripts/inventory.py                                    show the body/head pairing first

<operator> is the short name: the body id without a _1_1 skin (valen, jae, blackwell_4_1, kofi_2_2 ...).
Body and head come from scripts/inventory.py (best face-rigged head for that skin) unless given.
Operators whose head has no face rig (gas masks, full helmets) skip face, cc5 and cc5_sheet.
--head-only rigs just the head (spine3/neck/head/jaw/eyes + face bones, scripts/op_head_only.py), skips poses
and mocap, and also works for heads exported without a body (carpenter, hero_kwon).

steps (in order; default all):
  audit      check the Saluki export is complete (bones, every material textured) before spending time on it
  import     Cast import of body + high_fe head                        -> <op>_cod_import.blend
  rig        SMPL-X re-rig (T-pose by inverse skinning, face bones kept) -> <op>_smplx.blend/.fbx
  poses      shoulder round-trip check + standing poses                -> checks/<op>_poses_*.png, _shoulder_*.png
  mocap      AMASS sample clip on the rig, FBX + video                 -> <op>_smplx_amass.blend/.fbx, <op>_amass_sample.mp4
  face       front face render + MediaPipe landmarks                   -> checks/face/<op>.png(.json,.lm.json)
  cc5        CC5 expression shapes + hinged-jaw mouth weights          -> <op>_smplx_cc5.blend/.fbx
  cc5_sheet  expressions on the operator next to the CC5 source        -> checks/<op>_cc5_expressions.png
Each step's numbers go to checks/summary.json, full logs to checks/logs/.
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "scripts"))
import config  # noqa: E402

STEPS = ["audit", "import", "rig", "poses", "mocap", "face", "cc5", "cc5_sheet"]
JSON_LINE = re.compile(r"^([A-Z0-9_]+_JSON) (.*)$")


def run(cmd, log_name, env=None):
    os.makedirs(config.checks_dir(OP, "logs"), exist_ok=True)
    log = os.path.join(config.checks_dir(OP, "logs"), log_name + ".log")
    p = subprocess.run(cmd, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace")
    with open(log, "w", encoding="utf-8") as f:
        f.write(p.stdout + "\n--- stderr ---\n" + p.stderr)
    if p.returncode != 0:
        tail = "\n".join((p.stdout + p.stderr).strip().splitlines()[-15:])
        sys.exit(f"[{log_name}] failed (exit {p.returncode}); log: {log}\n{tail}")
    found = {}
    for line in p.stdout.splitlines():
        m = JSON_LINE.match(line)
        if m:
            found[m.group(1)] = json.loads(m.group(2))
    return found, p.stdout


def blender(script, log_name, *args):
    env = dict(os.environ, OPERATOR=OP, BODY=ARGS.body, HEAD=ARGS.head)
    cmd = [config.BLENDER, "-b", "--factory-startup", "--python-exit-code", "1",
           "--python", os.path.join(config.SCRIPTS, script), "--", *args]
    return run(cmd, log_name, env)


def py(script, log_name, *args, python=sys.executable):
    return run([python, "-I", os.path.join(config.SCRIPTS, script), *args], log_name)


def ensure_mediapipe():
    if os.path.exists(config.MP_PYTHON):
        return
    print("  setting up MediaPipe environment (one time)...")
    venv = os.path.dirname(os.path.dirname(config.MP_PYTHON))
    subprocess.run([sys.executable, "-m", "venv", venv], check=True)
    subprocess.run([config.MP_PYTHON, "-m", "pip", "install", "-q", "mediapipe==1.2.0"], check=True)


def step_audit():
    found, out = py("mat_audit.py", "audit", config.MODELS, ARGS.head if ARGS.head_only else f"_{OP}_")
    rep = json.loads(out[out.index("{"):])
    names = [n for n in (ARGS.body, ARGS.head) if n]
    missing = [n for n in names if not os.path.isfile(os.path.join(config.MODELS, n, f"{n}_LOD0.cast"))]
    s = rep["summary"]
    problems = []
    if missing:
        problems.append(f"missing exports: {missing}")
    if s["materials_no_textures"] or s["slot_missing_file"]:
        problems.append(f"{s['materials_no_textures']} empty materials, {s['slot_missing_file']} missing images "
                        "(Cordycep: loadcommonfiles before the operator packs; Saluki skips existing model "
                        "folders, so rename them to *_old and export again)")
    if problems and not ARGS.force:
        sys.exit("[audit] " + "; ".join(problems) + "\n  (use --force to continue anyway)")
    return {"summary": s, "problems": problems}


def step_import():
    return blender("op_import.py", "import")[0].get("REPORT_JSON", {}).get("armatures")


def step_rig():
    return blender("op_head_only.py" if ARGS.head_only else "op_smplx.py", "rig")[0].get("STATS_JSON")


def step_poses():
    res = blender("op_pose_check.py", "poses")[0].get("POSECHECK_JSON")
    poses = config.checks_dir(OP, "poses")
    py("pose_sheets.py", "pose_sheets", poses, os.path.join(config.checks_dir(OP), f"{OP}_poses"))
    py("pose_sheets.py", "pose_sheets_close", os.path.join(poses, "close"),
       os.path.join(config.checks_dir(OP), f"{OP}_shoulder"), "close")
    return res


def step_mocap():
    frames = config.checks_dir(OP, "frames")
    for f in glob.glob(os.path.join(frames, "*.png")):
        os.remove(f)
    res = blender("op_drive_amass.py", "mocap")[0].get("DRIVE_JSON")
    run([config.FFMPEG, "-hide_banner", "-loglevel", "error", "-y", "-framerate", "30", "-i",
         os.path.join(frames, "f_%04d.png"), "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
         os.path.join(config.out_dir(OP), f"{OP}_amass_sample.mp4")], "mocap_video")
    shutil.rmtree(frames)
    return res


def step_face():
    ensure_mediapipe()
    lm = config.CC5_LANDMARKS
    if not os.path.exists(os.path.join(lm, "cc5.png.lm.json")):  # CC5 side: once for every operator
        os.makedirs(lm, exist_ok=True)
        blender("face_front_render.py", "face_cc5", "cc", os.path.join(lm, "cc5.png"))
        py("face_landmarks.py", "landmarks_cc5", config.MP_MODEL, os.path.join(lm, "cc5.png"), python=config.MP_PYTHON)
    png = os.path.join(config.checks_dir(OP, "face"), f"{OP}.png")
    if os.path.exists(png + ".lm.json"):
        os.remove(png + ".lm.json")
    # skin + eyeballs only first (glasses/beards hide faces); then the same untextured grey (dense tattoos or
    # face paint, a lone eye strip under a balaclava read as no face); then every material shown
    for mode in ("skin", "gray", "all"):
        blender("face_front_render.py", "face", OP, png, mode)
        _, out = py("face_landmarks.py", "landmarks", config.MP_MODEL, png, python=config.MP_PYTHON)
        if os.path.exists(png + ".lm.json"):
            return {"landmarks": len(json.load(open(png + ".lm.json"))["uv"]), "render": mode}
    sys.exit(f"[face] MediaPipe found no face in {png}, even with every material shown")


def step_cc5():
    return blender("cc5_transfer.py", "cc5")[0].get("CC5_JSON")


def step_cc5_sheet():
    _, out = blender("cc5_expr_sheet.py", "cc5_sheet")
    m = re.search(r"JAW_ANGLES cc=([\d.]+) op=([\d.]+)", out)
    py("expr_montage.py", "cc5_montage", config.checks_dir(OP, "cc5"),
       os.path.join(config.checks_dir(OP), f"{OP}_cc5_expressions.png"))
    return {"jaw_deg_matching_cc5_28deg": float(m.group(2))} if m else {}


FACE_STEPS = ("face", "cc5", "cc5_sheet")
BODY_STEPS = ("poses", "mocap")  # skipped by --head-only


def short_name(op_id):
    return op_id[:-4] if op_id.endswith("_1_1") else op_id


def lookup(op):
    import inventory
    inv, _ = inventory.inventory()
    rec = next((r for r in inv if short_name(r["operator"]) == op or r["operator"] == op), None)
    return rec, inv


def lookup_head(op):
    """Best head for a head-only run: the inventory's pick, else (head exported without a body) the
    rex_head_mp_<op>* folder with the biggest face rig. Returns (head, face_rig) or (None, None)."""
    import inventory
    rec, _ = lookup(op)
    if rec and rec.get("head"):
        return rec["head"], rec.get("face_rig")
    cands = []
    for d in os.listdir(config.MODELS):
        if d.startswith(f"rex_head_mp_{op}") and not d.endswith("_old") and (hc := inventory.check(d)):
            face = all(x in hc["bones"] for x in inventory.HEAD_NEED)
            cands.append(((face, hc["n_bones"]), d, "none" if not face else "full" if hc["n_bones"] > 300 else "simplified"))
    if not cands:
        return None, None
    _, head, face_rig = max(cands)
    return head, face_rig


def run_all():
    import time
    _, inv = lookup("")
    report, t_all = [], time.time()
    for rec in inv:
        op = short_name(rec["operator"])
        summary = os.path.join(config.out_dir(op), "checks", "summary.json")
        last = "mocap" if rec.get("face_rig") == "none" else "cc5_sheet"
        if ARGS.skip_done and os.path.exists(summary) and last in json.load(open(summary)):
            report.append({"operator": op, "status": "skipped (done)"})
            continue
        if rec["problems"]:
            report.append({"operator": op, "status": "not run: " + "; ".join(rec["problems"])})
            continue
        cmd = [sys.executable, os.path.abspath(__file__), op, "--body", rec["body"], "--head", rec["head"]]
        if ARGS.steps:
            cmd += ["--steps", ARGS.steps]
        t = time.time()
        print(f"=== {op} ({len(report) + 1}/{len(inv)}) face rig: {rec.get('face_rig')}", flush=True)
        p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
        ok = p.returncode == 0
        print(("  ok " if ok else "  FAILED ") + f"{time.time() - t:.0f}s" +
              ("" if ok else "\n  " + "\n  ".join((p.stdout + p.stderr).strip().splitlines()[-6:])), flush=True)
        report.append({"operator": op, "status": "ok" if ok else "failed", "seconds": round(time.time() - t),
                       "face_rig": rec.get("face_rig"), "warnings": rec["warnings"],
                       "error": None if ok else (p.stdout + p.stderr).strip().splitlines()[-6:]})
        json.dump(report, open(os.path.join(config.SMPLX_ROOT, "_batch_summary.json"), "w"), indent=1)
    n_ok = sum(r["status"] == "ok" for r in report)
    print(f"\n{n_ok} ok, {sum(r['status'] == 'failed' for r in report)} failed, "
          f"{sum(r['status'].startswith(('skipped', 'not run')) for r in report)} skipped/not run "
          f"in {(time.time() - t_all) / 60:.0f} min -> {os.path.join(config.SMPLX_ROOT, '_batch_summary.json')}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("operator", nargs="?")
    ap.add_argument("--all", action="store_true", help="run every operator found by scripts/inventory.py")
    ap.add_argument("--skip-done", action="store_true", help="with --all: skip operators already finished")
    ap.add_argument("--steps", help="comma list of steps to run")
    ap.add_argument("--from", dest="start", choices=STEPS, help="run this step and everything after it")
    ap.add_argument("--body", help="body model folder (default: from the inventory)")
    ap.add_argument("--head", help="head model folder (default: from the inventory)")
    ap.add_argument("--force", action="store_true", help="continue past a failed export audit")
    ap.add_argument("--head-only", action="store_true",
                    help="rig the head alone (no body, no poses/mocap) -> smplx/<operator>_head/")
    ARGS = ap.parse_args()
    if ARGS.all and ARGS.head_only:
        ap.error("--head-only runs one operator at a time")
    if ARGS.all:
        run_all()
        sys.exit(0)
    if not ARGS.operator:
        ap.error("give an operator name or --all")
    OP = ARGS.operator
    face_rig = "full"
    if ARGS.head_only:
        if ARGS.head:
            import inventory
            hc = inventory.check(ARGS.head)
            face_rig = "none" if not hc or not all(x in hc["bones"] for x in inventory.HEAD_NEED) else \
                "full" if hc["n_bones"] > 300 else "simplified"
        else:
            ARGS.head, face_rig = lookup_head(OP)
            if ARGS.head is None:
                sys.exit(f"no head export found for '{OP}' (rex_head_mp_{OP}*)")
        if face_rig == "none":
            sys.exit(f"{ARGS.head} has no face rig; a head-only rig would have nothing to animate")
        ARGS.body = ""
        OP = f"{OP}_head"
    elif not (ARGS.body and ARGS.head):
        rec, _ = lookup(OP)
        if rec is None:
            sys.exit(f"no export found for '{OP}' (see: python scripts/inventory.py)")
        ARGS.body, ARGS.head = ARGS.body or rec["body"], ARGS.head or rec["head"]
        face_rig = rec.get("face_rig", "full")
    else:
        import inventory
        hc = inventory.check(ARGS.head)
        if hc:
            face = all(x in hc["bones"] for x in inventory.HEAD_NEED)
            face_rig = "none" if not face else "full" if hc["n_bones"] > 300 else "simplified"
    steps = ARGS.steps.split(",") if ARGS.steps else STEPS[STEPS.index(ARGS.start):] if ARGS.start else STEPS
    bad = [s for s in steps if s not in STEPS]
    if bad:
        sys.exit(f"unknown steps {bad}; choose from {STEPS}")
    if ARGS.head_only:
        steps = [s for s in steps if s not in BODY_STEPS]
    if face_rig == "none":
        skipped = [s for s in steps if s in FACE_STEPS]
        steps = [s for s in steps if s not in FACE_STEPS]
        if skipped:
            print(f"[{OP}] head has no face rig: skipping {', '.join(skipped)}")
    os.makedirs(config.out_dir(OP), exist_ok=True)
    summary_path = os.path.join(config.checks_dir(OP), "summary.json")
    summary = json.load(open(summary_path)) if os.path.exists(summary_path) else {}
    summary["models"] = {"body": ARGS.body or None, "head": ARGS.head, "face_rig": face_rig,
                         "head_only": ARGS.head_only}
    for s in steps:
        print(f"[{OP}] {s} ...", flush=True)
        summary[s] = globals()[f"step_{s}"]()
        json.dump(summary, open(summary_path, "w"), indent=1)
    for f in glob.glob(os.path.join(config.out_dir(OP), "*.blend1")):
        os.remove(f)
    print(f"[{OP}] done. outputs: {config.out_dir(OP)}  checks: {config.checks_dir(OP)}")
