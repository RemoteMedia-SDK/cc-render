"""Background-Blender check: is the SMPL-X rig's shoulder/arm setup right?

1. Round trip: pose the SMPL-X rig back into the game's bind pose (undo the T-pose exactly) and
   compare it to the untouched CoD import (distance from each vertex to the nearest original vertex).
2. Standing poses rendered from several views, each with a skeleton overlay (shoulders in red).
Renders go to scratchpad/renders/<op>/poses/<pose>_<view>_{char,skel}.png; montage is done outside.
"""
import json
import math
import os

import bmesh
import bpy

import sys  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402
import numpy as np
from mathutils import Matrix, Vector
from mathutils.kdtree import KDTree

OP = config.op_name()
OUT_DIR = config.out_dir(OP)
RENDER_DIR = config.checks_dir(OP, "poses")
os.makedirs(RENDER_DIR, exist_ok=True)

bpy.ops.wm.open_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_smplx.blend"))
scene = bpy.context.scene
rig = bpy.data.objects[f"SMPLX-{OP}"]
body = bpy.data.objects[OP]
assert rig.matrix_world == Matrix.Identity(4)

T = Matrix.Rotation(math.radians(-90), 4, "Z") @ Matrix.Scale(0.01, 4)  # same as op_smplx.py
Rz = Matrix.Rotation(math.radians(-90), 3, "Z")


def rotate_world(arm_ob, bone, rot3):
    pb = arm_ob.pose.bones[bone]
    pivot = Matrix.Translation(pb.head)
    pb.matrix = pivot @ rot3.to_4x4() @ pivot.inverted() @ pb.matrix
    bpy.context.view_layer.update()


# --- the original CoD character, untouched, moved into SMPL-X space -------------------------------
with bpy.data.libraries.load(os.path.join(OUT_DIR, f"{OP}_cod_import.blend")) as (src, dst):
    dst.objects = list(src.objects)
cod_coll = bpy.data.collections.new("cod")
scene.collection.children.link(cod_coll)
for ob in dst.objects:
    cod_coll.objects.link(ob)
body_arm = next(o for o in dst.objects if o.type == "ARMATURE" and "body" in o.name)
head_arm = next(o for o in dst.objects if o.type == "ARMATURE" and "head" in o.name)
cod_meshes = [o for o in dst.objects if o.type == "MESH"]
bpy.context.view_layer.update()

# exact T-pose rotations op_smplx.py applied (CoD space, body armature at identity)
undo = {}
for s, side, lateral in (("le", "left", Vector((0, 1, 0))), ("ri", "right", Vector((0, -1, 0)))):
    pb = {n: body_arm.pose.bones[f"j_{n}_{s}"] for n in ("shoulder", "elbow", "wrist")}
    R1 = (pb["elbow"].head - pb["shoulder"].head).normalized().rotation_difference(lateral).to_matrix()
    d2 = R1 @ (pb["wrist"].head - pb["elbow"].head).normalized()
    R2 = d2.rotation_difference(lateral).to_matrix()
    R1s, R2s = Rz @ R1 @ Rz.transposed(), Rz @ R2 @ Rz.transposed()
    undo[side] = (R1s.inverted(), R1s.inverted() @ R2s.inverted() @ R1s)

Bs = body_arm.data.bones["j_spine4"].matrix_local
Hs = head_arm.data.bones["j_spine4"].matrix_local
body_arm.matrix_world = T
head_arm.matrix_world = T @ Bs @ Hs.inverted()
bpy.context.view_layer.update()
cod_coll.hide_render = True

# --- poses (world-axis rotations; character faces -Y, left = +X, arms along +-X at rest) ---------
X, Y, Z = Vector((1, 0, 0)), Vector((0, 1, 0)), Vector((0, 0, 1))


def rot(axis, deg):
    return Matrix.Rotation(math.radians(deg), 3, axis)


POSES = {
    "game_bind": None,  # special: undo the T-pose
    "t_pose": [],
    "a_pose": [("left_shoulder", Y, 45), ("right_shoulder", Y, -45)],
    "arms_down": [("left_shoulder", Y, 82), ("right_shoulder", Y, -82),
                  ("left_elbow", X, -15), ("right_elbow", X, -15)],
    "elbows_90": [("left_shoulder", Y, 80), ("right_shoulder", Y, -80),
                  ("left_elbow", X, -90), ("right_elbow", X, -90)],
    "arms_forward": [("left_shoulder", Z, -90), ("right_shoulder", Z, 90)],
    "arms_overhead": [("left_shoulder", Y, -75), ("right_shoulder", Y, 75)],
    "reach_across": [("left_shoulder", Z, -125), ("right_shoulder", Y, -80), ("right_elbow", X, -20)],
    "shrug": [("left_shoulder", Y, 80), ("right_shoulder", Y, -80),
              ("left_collar", Y, -15), ("right_collar", Y, 15)],
    "arms_back": [("left_shoulder", Y, 75), ("right_shoulder", Y, -75),
                  ("left_shoulder", X, 35), ("right_shoulder", X, 35)],
}


def set_pose(name):
    for pb in rig.pose.bones:
        pb.matrix_basis = Matrix.Identity(4)
    bpy.context.view_layer.update()
    if POSES[name] is None:
        for side, (u_sh, u_el) in undo.items():
            rotate_world(rig, f"{side}_shoulder", u_sh)
            rotate_world(rig, f"{side}_elbow", u_el)
        return
    for bone, axis, deg in POSES[name]:
        rotate_world(rig, bone, rot(axis, deg))


# --- skeleton overlay mesh --------------------------------------------------------------------------
BODY_JOINTS = ["pelvis", "spine1", "spine2", "spine3", "neck", "head"] + [
    f"{side}_{j}" for side in ("left", "right")
    for j in ("hip", "knee", "ankle", "foot", "collar", "shoulder", "elbow", "wrist")]
COD_JOINTS = {"pelvis": "j_mainroot", "spine1": "j_spinelower", "spine2": "j_spineupper", "spine3": "j_spine4",
              "neck": "j_neck", "head": "j_head"}
for side, s in (("left", "le"), ("right", "ri")):
    COD_JOINTS.update({f"{side}_{a}": f"j_{b}_{s}" for a, b in (
        ("hip", "hip"), ("knee", "knee"), ("ankle", "ankle"), ("foot", "ball"), ("collar", "clavicle"),
        ("shoulder", "shoulder"), ("elbow", "elbow"), ("wrist", "wrist"))})

mat_j = bpy.data.materials.new("skel_joint")
mat_j.diffuse_color = (0.1, 0.85, 1.0, 1)
mat_s = bpy.data.materials.new("skel_shoulder")
mat_s.diffuse_color = (1.0, 0.1, 0.1, 1)
skel_me = bpy.data.meshes.new("skel")
skel_me.materials.append(mat_j)
skel_me.materials.append(mat_s)
skel = bpy.data.objects.new("skel", skel_me)
scene.collection.objects.link(skel)


def build_skeleton(pos, parent_of):
    bm = bmesh.new()
    for j, p in pos.items():
        red = j.endswith("_shoulder")
        r = bmesh.ops.create_uvsphere(bm, u_segments=12, v_segments=8, radius=0.028 if red else 0.018,
                                      matrix=Matrix.Translation(p))
        for v in r["verts"]:
            for f in v.link_faces:
                f.material_index = 1 if red else 0
        par = parent_of.get(j)
        if par in pos:
            d = p - pos[par]
            m = Matrix.Translation((p + pos[par]) / 2) @ d.to_track_quat("Z", "Y").to_matrix().to_4x4()
            bmesh.ops.create_cone(bm, cap_ends=True, segments=8, radius1=0.006, radius2=0.006,
                                  depth=d.length, matrix=m)
    bm.to_mesh(skel_me)
    bm.free()


rest_shoulder = (rig.data.bones["left_shoulder"].head_local
                 + Vector((0.08, 0, 0)))
rig_parent = {b.name: (b.parent.name if b.parent else None) for b in rig.data.bones}

# --- render setup -----------------------------------------------------------------------------------
if scene.world is None:
    scene.world = bpy.data.worlds.new("World")
scene.world.color = (0.82, 0.84, 0.88)
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
scene.render.resolution_x, scene.render.resolution_y = 600, 700
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam_data.ortho_scale = 2.45
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
VIEWS = {"front": 0, "34": -35, "side": 90, "back": 180}


def shoot(tag, show):
    """show: 'rig' or 'cod' (which character mesh is visible)."""
    body.hide_render = show != "rig"
    cod_coll.hide_render = show != "cod"
    for vname, yaw_deg in VIEWS.items():
        yaw = math.radians(yaw_deg)
        cam.location = (6 * math.sin(yaw), -6 * math.cos(yaw), 1.02)
        cam.rotation_euler = (math.radians(90), 0, yaw)
        # character pass
        skel.hide_render = True
        scene.render.film_transparent = False
        sh.light, sh.color_type = "STUDIO", ("VERTEX" if tag.endswith("heat") else "TEXTURE")
        sh.show_cavity = False
        scene.render.filepath = os.path.join(RENDER_DIR, f"{tag}_{vname}_char.png")
        bpy.ops.render.render(write_still=True)
        if tag.endswith("heat"):
            continue
        # skeleton pass (transparent, composited on top later)
        body.hide_render = cod_coll.hide_render = True
        skel.hide_render = False
        scene.render.film_transparent = True
        sh.light, sh.color_type = "FLAT", "MATERIAL"
        scene.render.filepath = os.path.join(RENDER_DIR, f"{tag}_{vname}_skel.png")
        bpy.ops.render.render(write_still=True)
        body.hide_render = show != "rig"
        cod_coll.hide_render = show != "cod"
    if not tag.endswith("heat"):
        closeups(tag, show)


CLOSE_VIEWS = {"cfront": 0, "cside": 90, "cback": 180, "ctop": None}


def closeups(tag, show):
    """Tight shots of the left shoulder (character's left = +X) at the T-pose shoulder height."""
    c = rest_shoulder
    cam_data.ortho_scale = 0.75
    for vname, yaw_deg in CLOSE_VIEWS.items():
        if yaw_deg is None:  # from above, looking down, character front at the bottom of the frame
            cam.location, cam.rotation_euler = (c.x, c.y, c.z + 4), (0, 0, 0)
        else:
            yaw = math.radians(yaw_deg)
            cam.location = (c.x + 6 * math.sin(yaw), c.y - 6 * math.cos(yaw), c.z - 0.05)
            cam.rotation_euler = (math.radians(90), 0, yaw)
        for pass_ in ("char", "skel"):
            skel.hide_render = pass_ == "char"
            body.hide_render = pass_ == "skel" or show != "rig"
            cod_coll.hide_render = pass_ == "skel" or show != "cod"
            scene.render.film_transparent = pass_ == "skel"
            sh.light, sh.color_type = ("STUDIO", "TEXTURE") if pass_ == "char" else ("FLAT", "MATERIAL")
            scene.render.filepath = os.path.join(RENDER_DIR, "close", f"{tag}_{vname}_{pass_}.png")
            bpy.ops.render.render(write_still=True)
    cam_data.ortho_scale = 2.45


def world_verts(obs):
    dg = bpy.context.evaluated_depsgraph_get()
    out = []
    for ob in obs:
        ev = ob.evaluated_get(dg)
        me = ev.to_mesh()
        co = np.empty(len(me.vertices) * 3, dtype=np.float64)
        me.vertices.foreach_get("co", co)
        M = np.array(ev.matrix_world)
        out.append(co.reshape(-1, 3) @ M[:3, :3].T + M[:3, 3])
        ev.to_mesh_clear()
    return np.concatenate(out)


report = {}

# 1. original game model, with its own skeleton overlay
cod_pos = {j: T @ (body_arm.data.bones[b].head_local) for j, b in COD_JOINTS.items()}
build_skeleton(cod_pos, rig_parent)
shoot("00_game_original", "cod")

# 2. SMPL-X rig posed back into the game's bind pose + round-trip error
set_pose("game_bind")
rig_pos = {j: rig.pose.bones[j].head.copy() for j in BODY_JOINTS}
report["joint_offset_vs_game_cm"] = {j: round((rig_pos[j] - cod_pos[j]).length * 100, 3) for j in cod_pos}
orig = world_verts(cod_meshes)
tree = KDTree(len(orig))
for i, p in enumerate(orig):
    tree.insert(p, i)
tree.balance()
posed = world_verts([body])
err = np.array([tree.find(p)[2] for p in posed])

groups = {g.index: g.name for g in body.vertex_groups}
dominant = []
for v in body.data.vertices:
    g = max(v.groups, key=lambda g: g.weight, default=None)
    dominant.append(groups[g.group] if g else "")
REGIONS = {"shoulder+collar": ("shoulder", "collar"), "elbow+forearm": ("elbow",), "wrist+hand": (
    "wrist", "index", "middle", "ring", "pinky", "thumb"), "torso/legs/head": None}
reg = {}
for k, keys in REGIONS.items():
    if keys:
        m = np.array([any(x in d for x in keys) for d in dominant])
    else:
        m = ~np.array([any(x in d for x in sum((v for v in REGIONS.values() if v), ())) for d in dominant])
    e = err[m] * 100
    reg[k] = {"verts": int(m.sum()), "median_cm": round(float(np.median(e)), 3),
              "p95_cm": round(float(np.percentile(e, 95)), 3), "max_cm": round(float(e.max()), 3),
              "over_1cm_pct": round(float((e > 1).mean() * 100), 2)}
report["round_trip_error"] = reg

# heat map: 0 cm grey-blue -> 1 cm yellow -> >=2 cm red
attr = body.data.color_attributes.new("roundtrip_err", "FLOAT_COLOR", "POINT")
t = np.clip(err * 100 / 2.0, 0, 1)
col = np.where(t[:, None] < 0.5,
               np.array([0.55, 0.6, 0.75]) * (1 - 2 * t[:, None]) + np.array([1, 0.85, 0.1]) * 2 * t[:, None],
               np.array([1, 0.85, 0.1]) * (2 - 2 * t[:, None]) + np.array([0.9, 0.05, 0.05]) * (2 * t[:, None] - 1))
attr.data.foreach_set("color", np.concatenate([col, np.ones((len(col), 1))], axis=1).ravel().astype(np.float32))
body.data.color_attributes.active_color = attr

build_skeleton(rig_pos, rig_parent)
shoot("01_smplx_game_bind", "rig")
shoot("02_roundtrip_heat", "rig")

# 3. standing poses
for i, name in enumerate([p for p in POSES if p != "game_bind"], start=3):
    set_pose(name)
    build_skeleton({j: rig.pose.bones[j].head.copy() for j in BODY_JOINTS}, rig_parent)
    shoot(f"{i:02d}_{name}", "rig")

print("POSECHECK_JSON " + json.dumps(report))
