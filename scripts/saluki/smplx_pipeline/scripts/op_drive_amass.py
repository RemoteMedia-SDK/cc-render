"""Background-Blender step 3: drive the SMPL-X-rigged operator with an AMASS (SMPL-H) clip and render it.

AMASS conventions: axis-angle per joint, rest pose = SMPL T-pose with identity joint frames in SMPL
canonical space (Y-up, faces +Z, left = +X); root orientation maps canonical -> mocap world (Z-up).
Our rig rests in Blender space (Z-up, faces -Y, left = +X), so a joint's world delta is G_j @ C^T,
where C maps SMPL canonical -> our rest frame.
"""
import json
import math
import os

import bpy

import sys  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402
import numpy as np
from mathutils import Matrix, Vector

OP = config.op_name()
OUT_DIR = config.out_dir(OP)
CLIP = config.AMASS_CLIP
FRAMES_DIR = config.checks_dir(OP, "frames")
STEP = 4  # 120 fps -> 30 fps

bpy.ops.wm.open_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_smplx.blend"))
arm = bpy.data.objects[f"SMPLX-{OP}"]
assert arm.matrix_world == Matrix.Identity(4)

clip = np.load(CLIP, allow_pickle=False)
poses, markers = clip["poses"][::STEP], clip["marker_data"][::STEP]
n = len(poses)

BODY = ["pelvis", "left_hip", "right_hip", "spine1", "left_knee", "right_knee", "spine2", "left_ankle",
        "right_ankle", "spine3", "left_foot", "right_foot", "neck", "left_collar", "right_collar", "head",
        "left_shoulder", "right_shoulder", "left_elbow", "right_elbow", "left_wrist", "right_wrist"]
HAND = [f"{f}{i}" for f in ("index", "middle", "pinky", "ring", "thumb") for i in (1, 2, 3)]
ORDER = BODY + [f"left_{h}" for h in HAND] + [f"right_{h}" for h in HAND]  # SMPL-H pose order
ALL = [b.name for b in arm.data.bones]  # parent-first
parent = {b.name: (b.parent.name if b.parent else None) for b in arm.data.bones}

C = Matrix(((1, 0, 0), (0, 0, -1), (0, 1, 0)))  # columns: canonical x,y,z -> ours
Ct = C.transposed()
rest = {b.name: b.matrix_local.copy() for b in arm.data.bones}
rest_head = {k: m.to_translation() for k, m in rest.items()}
rest_tail = {b.name: b.tail_local.copy() for b in arm.data.bones}
FOOT_PTS = ["left_ankle", "right_ankle", "left_foot", "right_foot"]
rest_floor = min(min(rest_head[b].z, rest_tail[b].z) for b in FOOT_PTS)


def aa_to_mat(v):
    ang = float(np.linalg.norm(v))
    return Matrix.Identity(3) if ang < 1e-8 else Matrix.Rotation(ang, 3, Vector(v / ang))


scene = bpy.context.scene
scene.render.fps = 30
scene.frame_start, scene.frame_end = 1, n
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"

facing = None
pelvis_track = []
for f in range(n):
    local = {name: aa_to_mat(poses[f, 3 * i:3 * i + 3]) for i, name in enumerate(ORDER)}
    G, D, head = {}, {}, {}
    for b in ALL:
        R = local.get(b, Matrix.Identity(3))
        G[b] = G[parent[b]] @ R if parent[b] else R
        D[b] = G[b] @ Ct
        head[b] = (rest_head[b] if parent[b] is None
                   else head[parent[b]] + D[parent[b]] @ (rest_head[b] - rest_head[parent[b]]))
    # keep the lowest foot point on the floor (clip has no root translation)
    low = min(min(head[b].z, (head[b] + D[b] @ (rest_tail[b] - rest_head[b])).z) for b in FOOT_PTS)
    dz = rest_floor - low
    M = {b: Matrix.Translation(head[b] + Vector((0, 0, dz))) @ (D[b] @ rest[b].to_3x3()).to_4x4() for b in ALL}
    pelvis_track.append(tuple(M["pelvis"].to_translation()))
    if facing is None:
        facing = D["pelvis"] @ Vector((0, -1, 0))
    for b in ALL:
        if parent[b] is None:
            basis = rest[b].inverted() @ M[b]
        else:
            p = parent[b]
            basis = (rest[p].inverted() @ rest[b]).inverted() @ M[p].inverted() @ M[b]
        pb = arm.pose.bones[b]
        loc, quat, _ = basis.decompose()
        pb.location, pb.rotation_quaternion = loc, quat
        pb.keyframe_insert("location", frame=f + 1)
        pb.keyframe_insert("rotation_quaternion", frame=f + 1)

# --- check: limb directions of the rig vs the raw mocap markers ----------------------------------
labels = [s.decode() for s in clip["marker_labels"]]


def mk(f, *names):
    pts = [markers[f, labels.index(x)] for x in names]
    return Vector(np.nanmean(pts, axis=0))


LIMBS = {  # rig (from, to) : marker (from, to)
    ("left_hip", "left_knee"): (("LHIP",), ("LKNE", "LKNI")),
    ("left_knee", "left_ankle"): (("LKNE", "LKNI"), ("LANK",)),
    ("right_knee", "right_ankle"): (("RKNE", "RKNI"), ("RANK",)),
    ("left_shoulder", "left_elbow"): (("LFSH", "LBSH"), ("LELB", "LIEL")),
    ("left_elbow", "left_wrist"): (("LELB", "LIEL"), ("LOWR", "LIWR")),
    ("right_shoulder", "right_elbow"): (("RFSH", "RBSH"), ("RELB", "RIEL")),
    ("right_elbow", "right_wrist"): (("RELB", "RIEL"), ("ROWR", "RIWR")),
    ("spine3", "head"): (("CLAV", "TOPBACK"), ("LFHD", "RFHD", "LBHD", "RBHD")),
}
limb_err = {}
for f in (0, n // 2, n - 1):
    scene.frame_set(f + 1)
    for (a, b), (ma, mb) in LIMBS.items():
        v_rig = arm.pose.bones[b].head - arm.pose.bones[a].head
        v_mk = mk(f, *mb) - mk(f, *ma)
        limb_err.setdefault(f"{a}->{b}", []).append(round(math.degrees(v_rig.angle(v_mk)), 1))

# --- reference: the original mocap markers, shown beside the operator -----------------------------------
facing = Vector((facing.x, facing.y, 0)).normalized()
side = Vector((facing.y, -facing.x, 0)).normalized()  # character's right-hand side, horizontal
mk_center0 = Vector(np.nanmean(markers[0], axis=0))
offset = Vector((pelvis_track[0][0], pelvis_track[0][1], 0)) + side * 1.1 - Vector((mk_center0.x, mk_center0.y, 0))
mat = bpy.data.materials.new("marker")
mat.diffuse_color = (1.0, 0.45, 0.1, 1)
mk_mesh = bpy.data.meshes.new("marker")
import bmesh  # noqa: E402
bm = bmesh.new()
bmesh.ops.create_uvsphere(bm, u_segments=8, v_segments=6, radius=0.018)
bm.to_mesh(mk_mesh)
bm.free()
mk_mesh.materials.append(mat)
for i in range(markers.shape[1]):
    ob = bpy.data.objects.new(f"marker_{i:02d}", mk_mesh)
    scene.collection.objects.link(ob)
    last = None
    for f in range(n):
        p = markers[f, i]
        visible = not np.isnan(p).any()
        if visible:
            last = Vector(p) + offset
        ob.location = last if last is not None else Vector((0, 0, -10))
        ob.scale = (1, 1, 1) if visible else (0, 0, 0)
        ob.keyframe_insert("location", frame=f + 1)
        ob.keyframe_insert("scale", frame=f + 1)

# --- a seat under the operator (the clip is a seated subject) ---------------------------------------------
seat_top = min(p[2] for p in pelvis_track) - 0.11
bpy.ops.mesh.primitive_cube_add(size=1, location=(pelvis_track[0][0], pelvis_track[0][1], seat_top / 2))
seat = bpy.context.active_object
seat.scale = (0.42, 0.42, seat_top)
seat.location -= facing * 0.12  # sit on the back of the seat
seat_mat = bpy.data.materials.new("seat")
seat_mat.diffuse_color = (0.35, 0.28, 0.22, 1)
seat.data.materials.append(seat_mat)

# --- scene: floor, light background, camera --------------------------------------------------------
bpy.ops.mesh.primitive_plane_add(size=8, location=(0, 0, 0))
floor = bpy.context.active_object
floor_mat = bpy.data.materials.new("floor")
floor_mat.diffuse_color = (0.55, 0.55, 0.58, 1)
floor.data.materials.append(floor_mat)

if scene.world is None:
    scene.world = bpy.data.worlds.new("World")
scene.world.color = (0.82, 0.84, 0.88)
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light, sh.color_type = "STUDIO", "TEXTURE"
sh.show_shadows, sh.shadow_intensity = True, 0.35
sh.show_cavity = True
scene.display.light_direction = (0.4, -0.6, 0.7)
scene.render.resolution_x, scene.render.resolution_y = 960, 720
scene.render.film_transparent = False

center = Vector(pelvis_track[0]) + side * 0.55
yaw_dir = (facing.normalized() * math.cos(math.radians(25)) + side * math.sin(math.radians(25))).normalized()
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam_data.ortho_scale = 3.0
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
cam.location = center + yaw_dir * 6 + Vector((0, 0, 0.9))
target = Vector((center.x, center.y, 0.85))
cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
scene.camera = cam

bpy.ops.wm.save_as_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_smplx_amass.blend"))

bpy.ops.object.select_all(action="DESELECT")
arm.select_set(True)
bpy.data.objects[OP].select_set(True)
bpy.context.view_layer.objects.active = arm
bpy.ops.export_scene.fbx(filepath=os.path.join(OUT_DIR, f"{OP}_smplx_amass.fbx"), use_selection=True,
                         object_types={"ARMATURE", "MESH"}, add_leaf_bones=False, bake_anim=True,
                         bake_anim_use_all_bones=True, bake_anim_use_nla_strips=False,
                         bake_anim_use_all_actions=False, bake_anim_force_startend_keying=True,
                         bake_anim_step=1.0, bake_anim_simplify_factor=0.0, path_mode="ABSOLUTE")

os.makedirs(FRAMES_DIR, exist_ok=True)
scene.render.image_settings.file_format = "PNG"
scene.render.filepath = os.path.join(FRAMES_DIR, "f_")
bpy.ops.render.render(animation=True)
print("DRIVE_JSON " + json.dumps({"frames": n, "facing": list(facing), "rest_floor": rest_floor,
                                  "seat_top": seat_top, "limb_angle_err_deg": limb_err}))
