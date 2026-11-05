"""Render CC5 expressions on the operator next to the same expressions on the CC5 source character.

usage: blender -b --python cc5_expr_sheet.py   (env OPERATOR); writes renders/<op>/cc5/expr_<name>_{op,cc}.png
"""
import math
import os

import bpy

import sys  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402
from mathutils import Matrix, Vector

OP = config.op_name()
OUT_DIR = config.out_dir(OP)
RENDER_DIR = config.checks_dir(OP, "cc5")
os.makedirs(RENDER_DIR, exist_ok=True)
JAW_DEG = 28.0  # cc-render rotates the jaw bone ~28 deg for jawOpen
EXPR = {
    "neutral": {},
    "blink": {"Eye_Blink_L": 1, "Eye_Blink_R": 1},
    "smile": {"Mouth_Corner_Pull_L": 1, "Mouth_Corner_Pull_R": 1, "Eye_Cheek_Raise_L": 0.6, "Eye_Cheek_Raise_R": 0.6},
    "frown": {"Mouth_Corner_Depress_L": 1, "Mouth_Corner_Depress_R": 1, "Brow_Down_L": 1, "Brow_Down_R": 1},
    "brows_up": {"Brow_Raise_In_L": 1, "Brow_Raise_In_R": 1, "Brow_Raise_Outer_L": 1, "Brow_Raise_Outer_R": 1,
                 "Eye_Widen_L": 1, "Eye_Widen_R": 1},
    "funnel": {"Mouth_Funnel_UL": 1, "Mouth_Funnel_UR": 1, "Mouth_Funnel_DL": 1, "Mouth_Funnel_DR": 1},
    "V_Open": {"V_Open": 1},
    "V_Explosive": {"V_Explosive": 1},
    "sneer": {"Nose_Wrinkle_L": 1, "Nose_Wrinkle_R": 1, "Mouth_UpperLip_Raise_L": 1, "Mouth_UpperLip_Raise_R": 1},
    "jaw_rotation_only": {"_jaw": 1},
    "jaw_open": {"Jaw_Open": 1, "_jaw": 1},
    "jaw_open_smile": {"Jaw_Open": 0.6, "_jaw": 0.6, "Mouth_Corner_Pull_L": 1, "Mouth_Corner_Pull_R": 1},
    "tongue_out": {"Jaw_Open": 0.5, "_jaw": 0.5, "Tongue_Out": 1},
}

bpy.ops.wm.open_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_smplx_cc5.blend"))
rig = bpy.data.objects[f"SMPLX-{OP}"]
face = bpy.data.objects[f"{OP}_face"]
before = set(bpy.data.objects)
bpy.ops.import_scene.fbx(filepath=config.CC5_FBX)
cc_objs = [o for o in bpy.data.objects if o not in before]
cc_arm = next(o for o in cc_objs if o.type == "ARMATURE")
for o in cc_objs:
    if o.type == "MESH" and o.name.split(".")[0] in ("Bang", "Hair_Base", "Real_Hair", "Earring_Flower_0", "Brows",
                                                     "Eyelash_Upper", "Eyelash_Lower", "CC_Base_EyeOcclusion",
                                                     "CC_Base_TearLine"):
        o.hide_render = True
cc_keyed = [o for o in cc_objs if o.type == "MESH" and o.data.shape_keys]

scene = bpy.context.scene
if scene.world is None:
    scene.world = bpy.data.worlds.new("World")
scene.world.color = (0.75, 0.76, 0.8)
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light, sh.color_type = "STUDIO", "TEXTURE"
sh.show_cavity = True
scene.render.resolution_x, scene.render.resolution_y = 520, 600
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam


def jaw_rotate(arm_ob, bone, deg):
    pb = arm_ob.pose.bones[bone]
    pb.matrix_basis = Matrix.Identity(4)
    bpy.context.view_layer.update()
    if deg:
        Mw = arm_ob.matrix_world
        head_w = Mw @ pb.head
        R = Matrix.Rotation(math.radians(deg), 4, "X")
        pw = Matrix.Translation(head_w) @ R @ Matrix.Translation(-head_w) @ (Mw @ pb.matrix)
        pb.matrix = Mw.inverted() @ pw
    bpy.context.view_layer.update()


def shoot(path, center, scale):
    cam_data.ortho_scale = scale
    cam.location = center + Vector((-0.55, -1.5, 0.0)) * 1.0
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)


op_center = (rig.data.bones["left_eye_smplhf"].head_local + rig.data.bones["right_eye_smplhf"].head_local) / 2 \
    - Vector((0, 0, 0.04))
cc_eye = [cc_arm.matrix_world @ cc_arm.data.bones[b].head_local for b in ("CC_Base_L_Eye", "CC_Base_R_Eye")]
cc_center = (cc_eye[0] + cc_eye[1]) / 2 - Vector((0, 0, 0.036))
cc_scale = 0.22 / 1.12
# the operator's jaw pivots at the hinge, far behind the lips; CC5's JawRoot sits near the mouth. Use the
# angle that gives the operator's lower lip the same drop as CC5's at JAW_DEG, scaled to the face size.
op_lip = rig.data.bones["j_lip_bot"].head_local
op_jaw = rig.data.bones["jaw"].head_local
cc_jr = cc_arm.matrix_world @ cc_arm.data.bones["CC_Base_JawRoot"].head_local
cc_lip = cc_center + Vector((0, -0.035, -0.06))  # approx lower lip; refined below from the CC5 mesh
_b = next(o for o in cc_objs if o.name.startswith("CC_Base_Body"))
_cand = [(_b.matrix_world @ v.co) for v in _b.data.vertices]
_cand = [c for c in _cand if abs(c.x) < 0.003 and cc_center.z - 0.09 < c.z < cc_center.z - 0.05]
cc_lip = min(_cand, key=lambda c: c.y)  # front-most midline point around the lower lip
cc_drop = 2 * (cc_lip - cc_jr).length * math.sin(math.radians(JAW_DEG) / 2)
op_deg = math.degrees(2 * math.asin(min(1.0, cc_drop * 1.12 / 2 / (op_lip - op_jaw).length)))
print(f"JAW_ANGLES cc={JAW_DEG} op={op_deg:.1f}")
visible_op = [o for o in bpy.data.objects if o.type == "MESH" and o not in cc_objs and not o.hide_render]
for name, ex in EXPR.items():
    for kb in face.data.shape_keys.key_blocks[1:]:
        kb.value = ex.get(kb.name, 0.0)
    for o in cc_keyed:
        for kb in o.data.shape_keys.key_blocks[1:]:
            kb.value = ex.get(kb.name, 0.0)
    jaw_rotate(rig, "jaw", op_deg * ex.get("_jaw", 0))
    jaw_rotate(cc_arm, "CC_Base_JawRoot", JAW_DEG * ex.get("_jaw", 0))
    for side, objs in (("op", visible_op), ("cc", cc_objs)):
        for o in visible_op + cc_objs:
            o.hide_render = True
        for o in objs:
            if o.type == "MESH" and o.name.split(".")[0] not in (
                    "Bang", "Hair_Base", "Real_Hair", "Earring_Flower_0", "Brows", "Eyelash_Upper", "Eyelash_Lower",
                    "CC_Base_EyeOcclusion", "CC_Base_TearLine"):
                o.hide_render = False
        if side == "op":
            shoot(os.path.join(RENDER_DIR, f"expr_{name}_op.png"), op_center, 0.22)
        else:
            shoot(os.path.join(RENDER_DIR, f"expr_{name}_cc.png"), cc_center, 0.22 / 1.12)
print("SHEET_DONE")
