"""Shared paths for the CoD operator -> SMPL-X (+ CC5 expressions) pipeline.

Imported by every script (Blender-side and plain Python). Per-operator outputs go to
SMPLX_ROOT/<operator>/, check renders to SMPLX_ROOT/<operator>/checks/.
"""
import os

PIPE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPTS = os.path.join(PIPE, "scripts")
# deps\ and cc5_source\ (Cast add-on, AMASS clip, MediaPipe model and venv, the CC5 source FBX): ~400 MB of
# data kept out of the repo
DATA = os.environ.get("SMPLX_PIPELINE_DATA", r"C:\Users\mail\Downloads\saluki\smplx_pipeline")

# Saluki exports and pipeline outputs
SALUKI_EXPORT = r"C:\Users\mail\Downloads\saluki\exported_files\mw7"
MODELS = os.path.join(SALUKI_EXPORT, "models")
SMPLX_ROOT = os.path.join(SALUKI_EXPORT, "smplx")

# tools
BLENDER = r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
FFMPEG = "ffmpeg"
MP_PYTHON = os.path.join(DATA, "deps", "mpenv", "Scripts", "python.exe")

# shared data (same for every operator)
CAST_ADDON = os.path.join(DATA, "deps", "cast_addon", "pkg")           # dtzxporter/cast v2.01 io_scene_cast
AMASS_CLIP = os.path.join(DATA, "deps", "amass", "amass_sample.npz")    # SMPL-H, 120 fps, seated subject
MP_MODEL = os.path.join(DATA, "deps", "mediapipe", "face_landmarker.task")
CC5_FBX = os.path.join(DATA, "cc5_source", "character.fbx")             # CC5 "Beautiful" with expression shapes
CC5_LANDMARKS = os.path.join(DATA, "cc5_source", "landmarks")           # cc5.png(.json,.lm.json): computed once


def op_name():
    return os.environ.get("OPERATOR", "valen")


def out_dir(op=None):
    return os.path.join(SMPLX_ROOT, op or op_name())


def checks_dir(op=None, *sub):
    d = os.path.join(out_dir(op), "checks", *sub)
    os.makedirs(d, exist_ok=True)
    return d
