"""Background-Blender step 2 (head-only): rig an operator's head export on its own, without a body.

Opens <op>_cod_import.blend (head only) and:
  1. places the head where it sits on a standing operator: the head skeleton's root (j_spine4) goes onto
     Valen's body j_spine4 in the game's bind pose, so heights and face orientation match the full rigs
  2. converts cm -> m and turns it to face -Y, Z-up (as op_smplx.py)
  3. builds the SMPL-X joints from the chest up: spine3 (j_spine4) -> neck -> head -> jaw, eyes; the face
     bones are kept under head/jaw exactly as in the full rig (cod_rig.add_face_bones)
  4. folds the other CoD weights into those joints (neck twist/add bones -> neck; clavicle and deltoid
     helpers along the cut edge -> spine3)
  5. saves <op>_smplx.blend/.fbx and face renders (rest, jaw open)
The head export is already in its bind pose, so there is no T-pose and no inverse skinning. The face
steps (face, cc5, cc5_sheet) run on the result unchanged.
"""
import json
import math
import os
import sys

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402
from cod_rig import JOINT_SRC, SPLIT_AT_MOUTH, add_face_bones, bone_parents, fold_weights  # noqa: E402
from mathutils import Matrix, Quaternion, Vector  # noqa: E402

OP = config.op_name()
OUT_DIR = config.out_dir(OP)
SCRATCH = config.checks_dir(OP, "rig")

bpy.ops.wm.open_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_cod_import.blend"))

# SMPL-X joints a head export can drive, parent-first
HEAD_JOINTS = {"spine3": None, "neck": "spine3", "head": "neck",
               "jaw": "head", "left_eye_smplhf": "head", "right_eye_smplhf": "head"}
# Valen's body j_spine4 in the game bind pose (CoD space: cm, faces +X, Z-up). Head exports have their
# j_spine4 at the origin with identity rotation; every operator body puts it within a few cm of this.
BODY_SPINE4 = Matrix(((0.19558, -0.98069, 0.00006, -5.46499),
                      (0.00001, -0.00006, -1.0, -0.00045),
                      (0.98069, 0.19558, 0.0, 140.77318),
                      (0.0, 0.0, 0.0, 1.0)))

head_arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
missing = [JOINT_SRC[j] for j in HEAD_JOINTS if JOINT_SRC[j] not in head_arm.data.bones]
if missing:
    sys.exit(f"head has no face rig (missing {missing}); a head-only rig needs jaw and eyes")

# --- 1-2. place the head on a standing operator, move into SMPL-X space -------------------------
head_arm.matrix_world = BODY_SPINE4 @ head_arm.data.bones["j_spine4"].matrix_local.inverted()
bpy.context.view_layer.update()
T = Matrix.Rotation(math.radians(-90), 4, "Z") @ Matrix.Scale(0.01, 4)  # +X fwd -> -Y fwd, cm -> m

meshes = [o for o in bpy.data.objects if o.type == "MESH"]
for ob in meshes:
    for m in [m for m in ob.modifiers if m.type == "ARMATURE"]:
        ob.modifiers.remove(m)  # rest pose: the armature adds nothing
    mw = ob.matrix_world.copy()
    ob.parent = None
    ob.data.transform(T @ mw)
    ob.matrix_world = Matrix.Identity(4)

joint_pos = {j: T @ (head_arm.matrix_world @ head_arm.data.bones[JOINT_SRC[j]].head_local) for j in HEAD_JOINTS}

# --- 3. SMPL-X joints + face bones ---------------------------------------------------------------
smplx_data = bpy.data.armatures.new(f"SMPLX-{OP}")
smplx = bpy.data.objects.new(f"SMPLX-{OP}", smplx_data)
bpy.context.scene.collection.objects.link(smplx)
bpy.context.view_layer.objects.active = smplx
bpy.ops.object.mode_set(mode="EDIT")
fwd, up = Vector((0, -1, 0)), Vector((0, 0, 1))
TAIL = {"spine3": joint_pos["neck"], "neck": joint_pos["head"], "head": joint_pos["head"] + up * 0.15}
for j, p in HEAD_JOINTS.items():  # bone tails as in op_smplx.py
    eb = smplx_data.edit_bones.new(j)
    eb.head = joint_pos[j]
    eb.tail = TAIL.get(j, joint_pos[j] + fwd * 0.03)
    if p:
        eb.parent = smplx_data.edit_bones[p]
face_bones = add_face_bones(smplx_data, head_arm, T, HEAD_JOINTS)
bpy.ops.object.mode_set(mode="OBJECT")

# --- 4. fold CoD weights -------------------------------------------------------------------------
fold_report = {}
parents = bone_parents(head_arm)
for ob in meshes:
    fold_weights(ob, parents, set(face_bones), HEAD_JOINTS, face_bones, smplx, fold_report)
bpy.data.objects.remove(head_arm, do_unlink=True)

bpy.ops.object.select_all(action="DESELECT")
for ob in meshes:
    ob.select_set(True)
bpy.context.view_layer.objects.active = meshes[0]
bpy.ops.object.join()
mesh = bpy.context.view_layer.objects.active
mesh.name = OP

# --- 5. checks, save, export ---------------------------------------------------------------------
zs = [v.co.z for v in mesh.data.vertices]
stats = {
    "head_only": True,
    "face_rig": True,
    "total_bones": len(smplx_data.bones),
    "face_bones": len(face_bones),
    "face_under_jaw": len([b for b in smplx_data.bones if b.parent and b.parent.name == "jaw"]),
    "face_under_jaw_split": sorted(b.name for b in smplx_data.bones
                                   if b.parent and b.parent.name == "jaw" and b.name.startswith(SPLIT_AT_MOUTH)),
    "verts": len(mesh.data.vertices),
    "verts_bad_weight_sum": sum(1 for v in mesh.data.vertices if abs(sum(g.weight for g in v.groups) - 1) > 1e-3),
    "verts_unweighted": sum(1 for v in mesh.data.vertices if not v.groups),
    "z_range_m": [round(min(zs), 3), round(max(zs), 3)],
    "joints": {j: [round(c, 3) for c in joint_pos[j]] for j in HEAD_JOINTS},
    "fold_sizes": {j: len(v) for j, v in fold_report.items()},
}

bpy.ops.wm.save_as_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_smplx.blend"))
bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
smplx.select_set(True)
bpy.ops.export_scene.fbx(filepath=os.path.join(OUT_DIR, f"{OP}_smplx.fbx"), use_selection=True,
                         add_leaf_bones=False, bake_anim=False, path_mode="ABSOLUTE")

# --- verification renders: front, 3/4, jaw opened (+X rotation tips the chin down) --------------
scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.display.shading.color_type = "TEXTURE"
scene.display.shading.light = "STUDIO"
scene.render.resolution_x = scene.render.resolution_y = 700
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam_data.ortho_scale = 0.42
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
center = Vector((joint_pos["head"].x, 0, (joint_pos["spine3"].z + max(zs)) / 2))


def shoot(name, yaw_deg):
    yaw = math.radians(yaw_deg)
    cam.location = center + Vector((5 * math.sin(yaw), -5 * math.cos(yaw), 0))
    cam.rotation_euler = (math.radians(90), 0, yaw)
    scene.render.filepath = os.path.join(SCRATCH, name)
    bpy.ops.render.render(write_still=True)


shoot("render_head_front.png", 0)
shoot("render_head_34.png", -35)
pb = smplx.pose.bones["jaw"]
pivot = Matrix.Translation(pb.head)
pb.matrix = pivot @ Quaternion(Vector((1, 0, 0)), math.radians(18)).to_matrix().to_4x4() @ pivot.inverted() @ pb.matrix
bpy.context.view_layer.update()
shoot("render_head_jaw_open.png", 0)

print("STATS_JSON " + json.dumps(stats))
