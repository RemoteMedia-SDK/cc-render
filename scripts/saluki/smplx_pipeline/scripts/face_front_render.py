"""Render a straight-on orthographic face shot for landmark detection.

usage: blender -b --python face_front_render.py -- <cc|operator name> <out_png>
Writes <out_png> and <out_png>.json with the pixel->world mapping:
  X = cx + (u / W - 0.5) * scale,   Z = cz + (0.5 - v / H) * scale   (camera looks along +Y)
"""
import json
import os
import sys

import bpy

import sys  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
who, out_png = argv[:2]
MODE = argv[2] if len(argv) > 2 else "skin"  # skin | gray (skin, untextured: tattoos/paint) | all
face_only = MODE in ("skin", "gray")
CC_FBX = config.CC5_FBX
SMPLX_DIR = config.SMPLX_ROOT

if who == "cc":
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=CC_FBX)
    arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
    eyes = [arm.matrix_world @ arm.data.bones[b].head_local for b in ("CC_Base_L_Eye", "CC_Base_R_Eye")]
    for ob in bpy.data.objects:  # hair over the brows would hide them from the detector
        if ob.type == "MESH" and ob.name in ("Bang", "Hair_Base", "Real_Hair", "Brows", "Eyelash_Upper", "Eyelash_Lower", "CC_Base_EyeOcclusion", "CC_Base_TearLine", "Earring_Flower_0"):
            ob.hide_render = True
else:
    bpy.ops.wm.open_mainfile(filepath=os.path.join(SMPLX_DIR, who, f"{who}_smplx.blend"))
    rig = bpy.data.objects[f"SMPLX-{who}"]
    eyes = [rig.matrix_world @ rig.data.bones[b].head_local for b in ("left_eye_smplhf", "right_eye_smplhf")]
    # Show only face skin + eyeballs, as on the CC5 side: glasses, beards, hair, lashes, helmets and straps
    # hide the face from the detector (alpha cards also render opaque here). The file is not saved.
    import bmesh
    from collections import Counter
    ob = bpy.data.objects[who] if face_only else None
if who != "cc" and face_only:
    me = ob.data
    gname = {g.index: g.name for g in ob.vertex_groups}
    dom = [gname[max(v.groups, key=lambda g: g.weight).group] if v.groups else "" for v in me.vertices]
    lip = Counter(p.material_index for p in me.polygons if dom[p.vertices[0]].startswith("j_nose"))  # skin; not beards
    keep = {lip.most_common(1)[0][0]} if lip else set()
    keep |= {p.material_index for p in me.polygons if dom[p.vertices[0]].endswith("eye_smplhf")}
    bm = bmesh.new()
    bm.from_mesh(me)
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if f.material_index not in keep], context="FACES")
    bm.to_mesh(me)
    bm.free()
    me.update()
    print("FACE_ONLY_MATERIALS", [me.materials[i].name for i in keep])

eye_mid = (eyes[0] + eyes[1]) / 2
scale = 0.26
cx, cz = eye_mid.x, eye_mid.z - 0.035
scene = bpy.context.scene
if scene.world is None:
    scene.world = bpy.data.worlds.new("World")
scene.world.color = (0.75, 0.75, 0.75)
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light, sh.color_type = ("STUDIO", "SINGLE") if MODE == "gray" else ("FLAT", "TEXTURE")
sh.show_cavity = False
sh.show_shadows = False
scene.render.resolution_x = scene.render.resolution_y = 1024
scene.render.film_transparent = False
scene.view_settings.view_transform = "Standard"
cam_data = bpy.data.cameras.new("facecam")
cam_data.type = "ORTHO"
cam_data.ortho_scale = scale
cam_data.clip_end = 100
cam = bpy.data.objects.new("facecam", cam_data)
scene.collection.objects.link(cam)
cam.location = (cx, eye_mid.y - 2.0, cz)
cam.rotation_euler = (Vector((0, 1, 0))).to_track_quat("-Z", "Z").to_euler()
scene.camera = cam
scene.render.filepath = out_png
bpy.ops.render.render(write_still=True)
json.dump({"cx": cx, "cz": cz, "scale": scale, "W": 1024, "H": 1024, "eye_mid": list(eye_mid),
           "eyes": [list(e) for e in eyes]}, open(out_png + ".json", "w"))
print("RENDERED", out_png)
