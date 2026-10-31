"""Background-Blender step 1: import an operator's body+head (LOD0) with the Cast add-on and report."""
import json
import os
import sys

import bpy

import sys  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402

sys.path.insert(0, config.CAST_ADDON)
import io_scene_cast  # noqa: E402

io_scene_cast.register()

MODELS = config.MODELS
OP = config.op_name()
OUT_DIR = config.out_dir(OP)
os.makedirs(OUT_DIR, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
io_scene_cast.register() if not hasattr(bpy.ops.import_scene, "cast") else None

for name in (os.environ.get("BODY", f"rex_body_mp_{OP}_1_1"), os.environ.get("HEAD", f"rex_head_mp_{OP}_1_1_high_fe")):
    if not name:  # head-only run: BODY is empty
        continue
    path = os.path.join(MODELS, name, f"{name}_LOD0.cast")
    bpy.ops.import_scene.cast(filepath=path, import_reset=False, import_merge=True)

report = {"armatures": [], "meshes": []}
for ob in bpy.data.objects:
    if ob.type == "ARMATURE":
        bones = ob.data.bones
        pick = ["j_mainroot", "j_spine4", "j_head", "j_shoulder_le", "j_wrist_le", "j_hip_le", "j_ankle_le", "j_ball_le"]
        report["armatures"].append({
            "name": ob.name, "bones": len(bones), "matrix_world": [list(r) for r in ob.matrix_world],
            "sample": {b: [round(v, 3) for v in (ob.matrix_world @ bones[b].head_local)] for b in pick if b in bones},
        })
    elif ob.type == "MESH":
        mods = [m.type for m in ob.modifiers]
        report["meshes"].append({"name": ob.name, "verts": len(ob.data.vertices), "parent": ob.parent.name if ob.parent else None,
                                 "groups": len(ob.vertex_groups), "mods": mods, "shape_keys": bool(ob.data.shape_keys),
                                 "mats": [m.name for m in ob.data.materials if m][:2]})
report["unit_scale"] = bpy.context.scene.unit_settings.scale_length
bpy.ops.wm.save_as_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_cod_import.blend"))
print("REPORT_JSON " + json.dumps(report))
