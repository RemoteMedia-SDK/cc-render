"""CoD -> SMPL-X rig definitions shared by op_smplx.py (body + head) and op_head_only.py (head alone).

SMPL-X joint names/parents, which CoD bone gives each joint its position, how every other CoD bone's weights
fold into a joint, and which head bones are kept as face bones under SMPL-X head/jaw.
"""
from mathutils import Matrix

# --- SMPL-X skeleton definition (names/parents as in the smplx package) ---------------------------
SMPLX_PARENTS = {
    "pelvis": None, "left_hip": "pelvis", "right_hip": "pelvis", "spine1": "pelvis",
    "left_knee": "left_hip", "right_knee": "right_hip", "spine2": "spine1",
    "left_ankle": "left_knee", "right_ankle": "right_knee", "spine3": "spine2",
    "left_foot": "left_ankle", "right_foot": "right_ankle", "neck": "spine3",
    "left_collar": "spine3", "right_collar": "spine3", "head": "neck",
    "left_shoulder": "left_collar", "right_shoulder": "right_collar",
    "left_elbow": "left_shoulder", "right_elbow": "right_shoulder",
    "left_wrist": "left_elbow", "right_wrist": "right_elbow",
    "jaw": "head", "left_eye_smplhf": "head", "right_eye_smplhf": "head",
}
FINGERS = [("index", "index"), ("middle", "mid"), ("pinky", "pinky"), ("ring", "ring"), ("thumb", "thumb")]
for side, s in (("left", "le"), ("right", "ri")):
    for smplx_f, _ in FINGERS:
        SMPLX_PARENTS[f"{side}_{smplx_f}1"] = f"{side}_wrist"
        SMPLX_PARENTS[f"{side}_{smplx_f}2"] = f"{side}_{smplx_f}1"
        SMPLX_PARENTS[f"{side}_{smplx_f}3"] = f"{side}_{smplx_f}2"

# CoD bone -> SMPL-X joint (the joint *positions* come from these bones)
JOINT_SRC = {
    "pelvis": "j_mainroot", "spine1": "j_spinelower", "spine2": "j_spineupper", "spine3": "j_spine4",
    "neck": "j_neck", "head": "j_head", "jaw": "j_jaw",
    "left_eye_smplhf": "j_eyeball_le", "right_eye_smplhf": "j_eyeball_ri",
}
for side, s in (("left", "le"), ("right", "ri")):
    JOINT_SRC.update({
        f"{side}_hip": f"j_hip_{s}", f"{side}_knee": f"j_knee_{s}", f"{side}_ankle": f"j_ankle_{s}",
        f"{side}_foot": f"j_ball_{s}", f"{side}_collar": f"j_clavicle_{s}",
        f"{side}_shoulder": f"j_shoulder_{s}", f"{side}_elbow": f"j_elbow_{s}", f"{side}_wrist": f"j_wrist_{s}",
    })
    for smplx_f, cod_f in FINGERS:
        for i in (1, 2, 3):
            JOINT_SRC[f"{side}_{smplx_f}{i}"] = f"j_{cod_f}_{s}_{i}"

# Extra CoD bones that map directly (weights only)
DIRECT = {v: k for k, v in JOINT_SRC.items()}
DIRECT.update({"j_spinelower_2": "spine1", "j_spineupper_2": "spine2", "j_neck_2": "neck", "j_neck_3": "neck"})
for side, s in (("left", "le"), ("right", "ri")):
    DIRECT[f"j_toe_{s}"] = f"{side}_foot"
    for smplx_f, cod_f in FINGERS:
        DIRECT[f"j_meta{cod_f}_{s}_1"] = f"{side}_wrist"

# Procedural/corrective bones whose parent chain would put them on the wrong joint
KEYWORDS = [  # (substring, joint stem) checked in order; side suffix is appended
    ("hiptwist", "hip"), ("hip_lift", "hip"), ("glute", "hip"), ("pelvic_crease", "hip"),
    ("knee", "knee"), ("ankletwist", "ankle"),
    ("shouldertwist", "shoulder"), ("bicep", "shoulder"), ("deltoid", "shoulder"),
    ("wristfronttwist", "elbow"), ("wristup", "wrist"),
    ("clavicle", "collar"), ("pectoralis", None),  # None -> spine3, no side
]


# Facial bones are kept as extra bones (CoD names, CoD rest orientation) under SMPL-X head/jaw.
# The CoD face rig is flat (everything parented to j_head); these lower-face stems go under jaw.
# ("j_chin" also catches j_chinstrap_*, which in this rig is chin-line skin, not a helmet strap.)
LOWER_FACE = ("ct_lowerlip_jnt", "j_chin", "j_innerlip_bot", "j_lip_bot", "j_lowergum", "j_tongue",
              "j_underchin", "j_underlip", "j_jaw_", "j_insidegum")
# Families that span both lips: members below the mouth line go under jaw.
SPLIT_AT_MOUTH = ("j_mouth_add", "j_mouth_bag")
FACE_BONE_LEN = 0.01


def is_face_bone(name, parents):
    if name in DIRECT or name.startswith(("j_neck", "j_proc_", "j_clavicle")):
        return False
    p = parents.get(name)
    while p is not None:
        if p == "j_head":
            return True
        p = parents.get(p)
    return False


def smplx_for(bone_name, parents):
    """SMPL-X joint that a CoD bone's weights fold into."""
    if bone_name in DIRECT:
        return DIRECT[bone_name]
    side = "left" if bone_name.endswith("_le") else "right" if bone_name.endswith("_ri") else None
    for key, stem in KEYWORDS:
        if key in bone_name:
            if stem is None:
                return "spine3"
            if side:
                return f"{side}_{stem}"
    p = parents.get(bone_name)
    while p is not None:
        if p in DIRECT:
            return DIRECT[p]
        p = parents.get(p)
    return "pelvis"


def bone_parents(arm_ob):
    return {b.name: (b.parent.name if b.parent else None) for b in arm_ob.data.bones}


def present(j, joints):
    """j, or its nearest SMPL-X ancestor that the rig has (a head-only rig has no arms/spine below spine3)."""
    while j is not None and j not in joints:
        j = SMPLX_PARENTS[j]
    return j if j is not None else next(iter(joints))


def add_face_bones(arm_data, head_arm, T, joints):
    """Add the CoD head's face bones (CoD names, CoD rest orientation, T applied) to arm_data, which must be
    in edit mode and already hold the SMPL-X joints `joints`. Returns their names, parent-first."""
    parents_head = bone_parents(head_arm)
    face_bones = [b.name for b in head_arm.data.bones if is_face_bone(b.name, parents_head)]  # parent-first
    face_mat = {name: T @ head_arm.matrix_world @ head_arm.data.bones[name].matrix_local for name in face_bones}
    mouth_z = ((face_mat["j_lip_top"].translation.z + face_mat["j_lip_bot"].translation.z) / 2
               if "j_lip_top" in face_mat else 0.0)
    for name in face_bones:
        w = face_mat[name]
        eb = arm_data.edit_bones.new(name)
        eb.head, eb.tail = (0, 0, 0), (0, FACE_BONE_LEN, 0)
        eb.matrix = Matrix.LocRotScale(w.to_translation(), w.to_quaternion(), None)
        cod_parent = parents_head[name]
        if cod_parent == "j_head":
            lower = name.startswith(LOWER_FACE) or (
                name.startswith(SPLIT_AT_MOUTH) and w.translation.z < mouth_z - 0.003)
            new_parent = "jaw" if lower else "head"
        else:
            new_parent = (cod_parent if cod_parent in face_bones
                          else present(smplx_for(cod_parent, parents_head), joints))
        eb.parent = arm_data.edit_bones[new_parent]
    return face_bones


def fold_weights(ob, parents, face_set, joints, face_bones, rig, fold_report):
    """Replace ob's CoD vertex groups by one group per SMPL-X joint / kept face bone (weights summed and
    normalised) and bind ob to rig. fold_report collects joint -> CoD bones folded into it."""
    idx_to_joint = {}
    for vg in ob.vertex_groups:
        j = vg.name if vg.name in face_set else present(smplx_for(vg.name, parents), joints)
        idx_to_joint[vg.index] = j
        fold_report.setdefault(j, set()).add(vg.name)
    acc = {}
    for v in ob.data.vertices:
        w = {}
        for g in v.groups:
            if g.weight > 0:
                j = idx_to_joint[g.group]
                w[j] = w.get(j, 0.0) + g.weight
        tot = sum(w.values()) or 1.0
        for j, x in w.items():
            acc.setdefault(j, ([], []))
            acc[j][0].append(v.index)
            acc[j][1].append(x / tot)
    ob.vertex_groups.clear()
    for j in [*joints, *face_bones]:
        vg = ob.vertex_groups.new(name=j)
        if j in acc:
            for vi, x in zip(*acc[j]):
                vg.add([vi], x, "REPLACE")
    mod = ob.modifiers.new("SMPLX", "ARMATURE")
    mod.object = rig
    ob.parent = rig
