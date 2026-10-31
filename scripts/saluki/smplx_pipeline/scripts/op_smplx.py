"""Background-Blender step 2: re-rig an imported operator (CoD skeleton) onto an SMPL-X skeleton.

Opens <operator>_cod_import.blend and:
  1. snaps the head skeleton onto the body's j_spine4
  2. poses the arms into a T-pose and bakes it as the new rest pose
  3. converts cm -> m and turns the character to face -Y (SMPL-X Blender convention, Z-up, left = +X)
  4. builds a 55-joint SMPL-X armature from the CoD joint positions
  5. folds the weights of every CoD bone into its SMPL-X joint
  5b. re-derives the T-pose rest shape by inverse skinning: with the SMPL-X rig posed back into the
      game's bind pose, the mesh reproduces the original game mesh exactly (the CoD bake in step 2
      leaves the game's undriven shoulder helper bones behind, which put ~1-4 cm error on the deltoid)
  6. saves .blend + .fbx and renders verification images
"""
import json
import math
import os
import sys

import bpy

import sys  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402
import numpy as np
from mathutils import Matrix, Vector

OP = config.op_name()
OUT_DIR = config.out_dir(OP)
SRC_BLEND = os.path.join(OUT_DIR, f"{OP}_cod_import.blend")
SCRATCH = config.checks_dir(OP, "rig")

bpy.ops.wm.open_mainfile(filepath=SRC_BLEND)

from cod_rig import (SMPLX_PARENTS, JOINT_SRC, add_face_bones, bone_parents,  # noqa: E402
                     fold_weights, SPLIT_AT_MOUTH)

arms = [o for o in bpy.data.objects if o.type == "ARMATURE"]
body_arm = next(o for o in arms if "body" in o.name)
head_arm = next(o for o in arms if "head" in o.name)

# --- 1. snap head skeleton onto the body ---------------------------------------------------------
Bs = body_arm.matrix_world @ body_arm.data.bones["j_spine4"].matrix_local
Hs = head_arm.data.bones["j_spine4"].matrix_local
head_arm.matrix_world = Bs @ Hs.inverted()
bpy.context.view_layer.update()
head_err = ((head_arm.matrix_world @ head_arm.data.bones["j_head"].head_local)
            - (body_arm.matrix_world @ body_arm.data.bones["j_head"].head_local)).length

# --- 2. T-pose the arms --------------------------------------------------------------------------
# CoD space: faces +X, left = +Y, Z-up (cm)


def rotate_world(arm_ob, bone, rot):
    pb = arm_ob.pose.bones[bone]
    pivot = Matrix.Translation(pb.head)
    pb.matrix = pivot @ rot.to_matrix().to_4x4() @ pivot.inverted() @ pb.matrix
    bpy.context.view_layer.update()


def aim(arm_ob, bone, child, target):
    pb, cb = arm_ob.pose.bones[bone], arm_ob.pose.bones[child]
    d = (cb.head - pb.head).normalized()
    q = d.rotation_difference(target)
    rotate_world(arm_ob, bone, q)
    return q.to_matrix()


tpose_rot = {}  # side -> (shoulder, elbow) world rotations, CoD space
for s, side, lateral in (("le", "left", Vector((0, 1, 0))), ("ri", "right", Vector((0, -1, 0)))):
    tpose_rot[side] = (aim(body_arm, f"j_shoulder_{s}", f"j_elbow_{s}", lateral),
                       aim(body_arm, f"j_elbow_{s}", f"j_wrist_{s}", lateral))

# --- 3. bake pose into the meshes, move everything into SMPL-X space ----------------------------
T = Matrix.Rotation(math.radians(-90), 4, "Z") @ Matrix.Scale(0.01, 4)  # +X fwd -> -Y fwd, cm -> m

meshes = [o for o in bpy.data.objects if o.type == "MESH"]
bind_co = {}  # original game-pose vertex positions in SMPL-X space
for ob in meshes:
    co = np.empty(len(ob.data.vertices) * 3)
    ob.data.vertices.foreach_get("co", co)
    M = np.array(T @ ob.matrix_world)
    bind_co[ob.name] = co.reshape(-1, 3) @ M[:3, :3].T + M[:3, 3]
for ob in meshes:
    for m in [m for m in ob.modifiers if m.type == "ARMATURE"]:
        with bpy.context.temp_override(object=ob, active_object=ob, selected_objects=[ob]):
            bpy.ops.object.modifier_apply(modifier=m.name)
    mw = ob.matrix_world.copy()
    ob.parent = None
    ob.data.transform(T @ mw)
    ob.matrix_world = Matrix.Identity(4)

# joint positions (posed body, rest head) in SMPL-X space
# Heads without a face rig (gas masks, full helmets: only j_head/j_neck*) get jaw and eyes at the average
# offsets from the head joint measured on full-rig heads (Valen, Jae); nothing is weighted to them.
FACE_DEFAULT = {"jaw": Vector((0, -0.016, -0.023)), "left_eye_smplhf": Vector((0.033, -0.089, 0.048)),
                "right_eye_smplhf": Vector((-0.033, -0.089, 0.048))}
has_face = all(JOINT_SRC[j] in head_arm.data.bones for j in FACE_DEFAULT)
joint_pos = {}
for j, src in JOINT_SRC.items():
    if src in body_arm.pose.bones and j not in FACE_DEFAULT:
        joint_pos[j] = T @ (body_arm.matrix_world @ body_arm.pose.bones[src].head)
    elif has_face:
        joint_pos[j] = T @ (head_arm.matrix_world @ head_arm.data.bones[src].head_local)
for j, off in FACE_DEFAULT.items():
    joint_pos.setdefault(j, joint_pos["head"] + off)
toe = {s: T @ (body_arm.matrix_world @ body_arm.pose.bones[f"j_toe_{s}"].head) for s in ("le", "ri")}

# --- 4. build the SMPL-X armature ----------------------------------------------------------------
parents_body = bone_parents(body_arm)
parents_head = bone_parents(head_arm)

smplx_data = bpy.data.armatures.new(f"SMPLX-{OP}")
smplx = bpy.data.objects.new(f"SMPLX-{OP}", smplx_data)
bpy.context.scene.collection.objects.link(smplx)
bpy.context.view_layer.objects.active = smplx
bpy.ops.object.mode_set(mode="EDIT")
children = {}
for j, p in SMPLX_PARENTS.items():
    children.setdefault(p, []).append(j)
PRIMARY_CHILD = {"pelvis": "spine1", "spine3": "neck", "head": None}
for side in ("left", "right"):
    PRIMARY_CHILD[f"{side}_wrist"] = f"{side}_middle1"
fwd, up = Vector((0, -1, 0)), Vector((0, 0, 1))
for j in SMPLX_PARENTS:  # dict order is parent-first
    eb = smplx_data.edit_bones.new(j)
    eb.head = joint_pos[j]
    child = PRIMARY_CHILD.get(j, (children.get(j) or [None])[0])
    if j.endswith("_foot"):
        tail = toe["le" if j.startswith("left") else "ri"]
    elif child:
        tail = joint_pos[child]
    elif j == "head":
        tail = joint_pos[j] + up * 0.15
    elif j in ("jaw", "left_eye_smplhf", "right_eye_smplhf"):
        tail = joint_pos[j] + fwd * 0.03
    else:  # fingertips: continue the last segment
        par = SMPLX_PARENTS[j]
        tail = joint_pos[j] + (joint_pos[j] - joint_pos[par])
    if (tail - eb.head).length < 1e-4:
        tail = eb.head + up * 0.02
    eb.tail = tail
    if SMPLX_PARENTS[j]:
        eb.parent = smplx_data.edit_bones[SMPLX_PARENTS[j]]

face_bones = add_face_bones(smplx_data, head_arm, T, SMPLX_PARENTS)
bpy.ops.object.mode_set(mode="OBJECT")
face_set = set(face_bones)

# --- 5. fold CoD weights into SMPL-X joints ------------------------------------------------------
fold_report = {}
for ob in meshes:
    is_head = ob.name in {c.name for c in head_arm.children} or any(
        vg.name in parents_head and vg.name not in parents_body for vg in ob.vertex_groups)
    parents = {**parents_body, **parents_head} if is_head else parents_body
    fold_weights(ob, parents, face_set if is_head else set(), SMPLX_PARENTS, face_bones, smplx, fold_report)

# --- 5b. inverse skinning: rest shape such that the game bind pose reproduces the game mesh ------
Rz = Matrix.Rotation(math.radians(-90), 3, "Z")
for side, (R1, R2) in tpose_rot.items():
    R1s, R2s = Rz @ R1 @ Rz.transposed(), Rz @ R2 @ Rz.transposed()
    rotate_world(smplx, f"{side}_shoulder", R1s.inverted().to_quaternion())
    rotate_world(smplx, f"{side}_elbow", (R1s.inverted() @ R2s.inverted() @ R1s).to_quaternion())
bone_names = [b.name for b in smplx.data.bones]
skin = np.array([np.array(smplx.pose.bones[n].matrix @ smplx.data.bones[n].matrix_local.inverted())
                 for n in bone_names])  # (J, 4, 4), rig at identity world
bone_idx = {n: i for i, n in enumerate(bone_names)}
rest_shift = []
for ob in meshes:
    n = len(ob.data.vertices)
    W = np.zeros((n, len(bone_names)))
    gi = {vg.index: bone_idx[vg.name] for vg in ob.vertex_groups}
    for v in ob.data.vertices:
        for g in v.groups:
            W[v.index, gi[g.group]] += g.weight
    B = np.einsum("nj,jab->nab", W, skin)
    B[W.sum(axis=1) < 1e-6] = np.eye(4)  # unweighted verts stay put
    rest = np.linalg.solve(B[:, :3, :3], (bind_co[ob.name] - B[:, :3, 3])[:, :, None])[:, :, 0]
    old = np.empty(n * 3)
    ob.data.vertices.foreach_get("co", old)
    rest_shift.append(np.linalg.norm(rest - old.reshape(-1, 3), axis=1))
    ob.data.vertices.foreach_set("co", rest.ravel())
    ob.data.update()
rest_shift = np.concatenate(rest_shift) * 100
for pb in smplx.pose.bones:
    pb.matrix_basis = Matrix.Identity(4)
bpy.context.view_layer.update()

for a in (body_arm, head_arm):
    bpy.data.objects.remove(a, do_unlink=True)

# join meshes into one object
bpy.ops.object.select_all(action="DESELECT")
for ob in meshes:
    ob.select_set(True)
bpy.context.view_layer.objects.active = meshes[0]
bpy.ops.object.join()
mesh = bpy.context.view_layer.objects.active
mesh.name = OP

# --- 6. checks, save, export ---------------------------------------------------------------------
bad = sum(1 for v in mesh.data.vertices if abs(sum(g.weight for g in v.groups) - 1.0) > 1e-3)
unweighted = sum(1 for v in mesh.data.vertices if not v.groups)
stats = {
    "head_snap_error_cm": round(head_err, 4),
    "face_rig": has_face,
    "rest_shape_change_cm": {"verts_moved_over_1mm": int((rest_shift > 0.1).sum()),
                             "p95_of_moved": round(float(np.percentile(rest_shift[rest_shift > 0.1], 95)), 3)
                             if (rest_shift > 0.1).any() else 0, "max": round(float(rest_shift.max()), 3)},
    "total_bones": len(smplx_data.bones),
    "face_bones": len(face_bones),
    "face_under_jaw": len([b for b in smplx_data.bones if b.parent and b.parent.name == "jaw"]),
    "face_under_jaw_split": sorted(b.name for b in smplx_data.bones
                                   if b.parent and b.parent.name == "jaw" and b.name.startswith(SPLIT_AT_MOUTH)),
    "verts": len(mesh.data.vertices),
    "verts_bad_weight_sum": bad,
    "verts_unweighted": unweighted,
    "height_m": round(max((mesh.matrix_world @ v.co).z for v in mesh.data.vertices), 3),
    "joints": {j: [round(c, 3) for c in joint_pos[j]] for j in
               ("pelvis", "head", "left_shoulder", "left_wrist", "right_wrist", "left_ankle", "jaw")},
    "fold_sizes": {j: len(v) for j, v in fold_report.items()},
}

bpy.ops.wm.save_as_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_smplx.blend"))
bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
smplx.select_set(True)
bpy.ops.export_scene.fbx(filepath=os.path.join(OUT_DIR, f"{OP}_smplx.fbx"), use_selection=True,
                         add_leaf_bones=False, bake_anim=False, path_mode="ABSOLUTE")

# --- verification renders ------------------------------------------------------------------------
scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.display.shading.color_type = "TEXTURE"
scene.display.shading.light = "STUDIO"
scene.render.resolution_x, scene.render.resolution_y = 900, 1000
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam_data.ortho_scale = 2.1
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam


def shoot(name, yaw_deg):
    yaw = math.radians(yaw_deg)
    cam.location = (5 * math.sin(yaw), -5 * math.cos(yaw), 0.88)
    cam.rotation_euler = (math.radians(90), 0, yaw)
    scene.render.filepath = os.path.join(SCRATCH, name)
    bpy.ops.render.render(write_still=True)


shoot("render_tpose_front.png", 0)

from mathutils import Quaternion  # noqa: E402

# face close-ups: rest vs SMPL-X jaw opened (+X rotation tips the chin down for a -Y-facing character)
head_z = joint_pos["head"].z + 0.04
cam_data.ortho_scale = 0.32
cam.location, cam.rotation_euler = (0, -5, head_z), (math.radians(90), 0, 0)
scene.render.resolution_x = scene.render.resolution_y = 700
scene.render.filepath = os.path.join(SCRATCH, "render_face_rest.png")
bpy.ops.render.render(write_still=True)
bpy.context.view_layer.objects.active = smplx
bpy.ops.object.mode_set(mode="POSE")
rotate_world(smplx, "jaw", Quaternion(Vector((1, 0, 0)), math.radians(18)))
scene.render.filepath = os.path.join(SCRATCH, "render_face_jaw_open.png")
bpy.ops.render.render(write_still=True)
cam_data.ortho_scale = 2.1
scene.render.resolution_x, scene.render.resolution_y = 900, 1000

test_pose = [  # world-axis rotations, character faces -Y, left = +X
    ("left_shoulder", Vector((0, 1, 0)), 60), ("right_shoulder", Vector((0, 1, 0)), -20),
    ("right_elbow", Vector((0, 0, 1)), 90), ("left_hip", Vector((1, 0, 0)), -45),
    ("left_knee", Vector((1, 0, 0)), 70), ("head", Vector((0, 0, 1)), 30),
    ("spine2", Vector((0, 1, 0)), 10),
]
for bone, axis, deg in test_pose:
    rotate_world(smplx, bone, Quaternion(axis, math.radians(deg)))
bpy.ops.object.mode_set(mode="OBJECT")
shoot("render_testpose_front.png", 0)
shoot("render_testpose_34.png", -35)

print("STATS_JSON " + json.dumps(stats))
