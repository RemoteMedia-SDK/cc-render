"""Background-Blender step 4: transfer CC5 facial expression shapes onto an SMPL-X-rigged operator.

Needs the CC5 landmarks (config.CC5_LANDMARKS/cc5.png*) and checks/face/<op>.png(.json,.lm.json)
from face_front_render.py + face_landmarks.py (MediaPipe). Steps:
  1. MediaPipe landmarks -> 3D by casting the render rays onto each head (skin + eyeballs)
  2. CC5 head -> operator head: similarity fit, then an RBF warp through the landmark pairs,
     then a smoothed along-normal snap onto the operator skin
  3. CC5 shape deltas are carried into the fitted head (similarity rotation/scale applied)
  4. operator face verts are bound to the fitted CC5 head (Surface Deform), tongue to the CC5 tongue;
     each CC5 shape at weight 1 gives one operator shape key (same name)
  5. the moving part of the face is split into <op>_face (shape keys live only there)
Saves <op>_smplx_cc5.blend/.fbx next to the rig (the plain rig files are left alone).
"""
import json
import math
import os
from collections import defaultdict

import bmesh
import bpy

import sys  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402
import numpy as np
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree
from mathutils.kdtree import KDTree

OP = config.op_name()
OUT_DIR = config.out_dir(OP)
FACE_DIR = config.checks_dir(OP, "face")
CC_FBX = config.CC5_FBX
FACE_OVAL = {10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377, 152, 148,
             176, 149, 150, 136, 172, 58, 132, 93, 234, 127, 162, 21, 54, 103, 67, 109}
EXCLUDE = ("eye_smplhf", "j_eyeball", "tongue", "gum", "teeth", "tag_", "j_helmet", "j_visor")

bpy.ops.wm.open_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_smplx.blend"))
rig = bpy.data.objects[f"SMPLX-{OP}"]
op_ob = bpy.data.objects[OP]
assert rig.matrix_world == Matrix.Identity(4) and op_ob.matrix_world == Matrix.Identity(4)
before = set(bpy.data.objects)
bpy.ops.import_scene.fbx(filepath=CC_FBX)
cc_objs = [o for o in bpy.data.objects if o not in before]
cc = {o.name.split(".")[0]: o for o in cc_objs}
cc_arm = next(o for o in cc_objs if o.type == "ARMATURE")
for o in cc_objs:
    o.hide_render = True
bpy.context.view_layer.update()


def world_co(ob, kb=None):
    src = kb.data if kb is not None else ob.data.vertices
    a = np.empty(len(src) * 3)
    src.foreach_get("co", a)
    M = np.array(ob.matrix_world)
    return a.reshape(-1, 3) @ M[:3, :3].T + M[:3, 3]


def tris(me):
    me.calc_loop_triangles()
    t = np.empty(len(me.loop_triangles) * 3, dtype=np.int64)
    me.loop_triangles.foreach_get("vertices", t)
    return t.reshape(-1, 3)


# --- operator vertex roles -------------------------------------------------------------------------
me = op_ob.data
op_co = world_co(op_ob)
gname = {g.index: g.name for g in op_ob.vertex_groups}
dom = []
for v in me.vertices:
    g = max(v.groups, key=lambda g: g.weight, default=None)
    dom.append(gname[g.group] if g else "")
dom = np.array(dom)
is_eye = np.char.find(dom, "eye_smplhf") >= 0
is_tongue = np.char.find(dom, "tongue") >= 0
excluded = np.array([any(x in d for x in EXCLUDE) for d in dom])
head_tree = {b.name for b in rig.data.bones if b.name == "head" or any(p.name == "head" for p in b.parent_recursive)}
head_tree -= {"left_eye_smplhf", "right_eye_smplhf"}
head_plain = np.isin(dom, ["head", "jaw"])

lip_mat = defaultdict(int)
for p in me.polygons:
    if dom[p.vertices[0]].startswith("j_nose"):  # the nose is skin only; lip bones also carry beards
        lip_mat[p.material_index] += 1
skin_mat = max(lip_mat, key=lip_mat.get)
skin_v = np.zeros(len(op_co), bool)
for p in me.polygons:
    if p.material_index == skin_mat:
        skin_v[list(p.vertices)] = True
# The game's gum bones also carry the skin of the inner lip corners (j_insidegum_*): excluded, it stayed put
# while the lips around it moved, pinching every corner shape into a crease. Only the gums and teeth stay out.
excluded &= ~(skin_v & np.array([any(x in d for x in ("gum", "teeth")) for d in dom]))
# every bone under head counts, including the game's hash-named face bones (bone_xxxxxxxx)
face_feature = np.isin(dom, list(head_tree - {"head", "jaw"})) & ~excluded
eye_mats = {p.material_index for p in me.polygons if is_eye[p.vertices[0]]}
op_tris = tris(me)
tri_mat = np.empty(len(me.loop_triangles), dtype=np.int64)
me.loop_triangles.foreach_get("material_index", tri_mat)


def bvh(co, t):
    return BVHTree.FromPolygons([Vector(c) for c in co], t.tolist())


op_skin_bvh = bvh(op_co, op_tris[tri_mat == skin_mat])
op_cast_bvh = bvh(op_co, op_tris[np.isin(tri_mat, list(eye_mats | {skin_mat}))])

# --- CC5 data -------------------------------------------------------------------------------------
cc_body, cc_tongue, cc_eye = cc["CC_Base_Body"], cc["CC_Base_Tongue"], cc["CC_Base_Eye"]
cb_co = world_co(cc_body)
cb_tris = tris(cc_body.data)
ce_co = world_co(cc_eye)
cc_cast_bvh = bvh(np.vstack([cb_co, ce_co]), np.vstack([cb_tris, tris(cc_eye.data) + len(cb_co)]))


# --- 1. landmarks to 3D ---------------------------------------------------------------------------
def lift(tag, tree):
    cam = json.load(open(f"{tag}.png.json"))  # tag = path prefix of the render
    uv = json.load(open(f"{tag}.png.lm.json"))["uv"]
    pts, ok = [], []
    for i, (u, v) in enumerate(uv):
        x = cam["cx"] + (u / cam["W"] - 0.5) * cam["scale"]
        z = cam["cz"] + (0.5 - v / cam["H"]) * cam["scale"]
        hit, n, _, _ = tree.ray_cast(Vector((x, cam["eye_mid"][1] - 1.0, z)), Vector((0, 1, 0)))
        # front-facing surface only (camera looks +Y), and not deeper than 5 cm behind the eyes: rays through
        # holes in the face skin (nose/mouth pieces in another material) otherwise hit the back of the head
        good = (hit is not None and i not in FACE_OVAL and n.y < -0.35
                and hit.y < cam["eye_mid"][1] + 0.05)
        pts.append(tuple(hit) if hit is not None else (0, 0, 0))
        ok.append(good)
    return np.array(pts), np.array(ok)


P_cc, ok_cc = lift(os.path.join(config.CC5_LANDMARKS, "cc5"), cc_cast_bvh)
P_op, ok_op = lift(os.path.join(FACE_DIR, OP), op_cast_bvh)
ok = ok_cc & ok_op


def umeyama(A, B):
    ma, mb = A.mean(0), B.mean(0)
    a, b = A - ma, B - mb
    U, S, Vt = np.linalg.svd(b.T @ a / len(A))
    D = np.eye(3)
    D[2, 2] = np.sign(np.linalg.det(U @ Vt))
    R = U @ D @ Vt
    s = np.trace(np.diag(S) @ D) / (a ** 2).sum(1).mean()
    return s, R, mb - s * R @ ma


s, R, t = umeyama(P_cc[ok], P_op[ok])
res = np.linalg.norm((s * P_cc @ R.T + t) - P_op, axis=1)
ok &= res < 4 * np.median(res[ok])  # drop outliers
s, R, t = umeyama(P_cc[ok], P_op[ok])
sim = lambda X: s * X @ R.T + t
src, dst = sim(P_cc[ok]), P_op[ok]
sim_err = np.linalg.norm(src - dst, axis=1)

# --- 2. RBF warp (biharmonic kernel r, affine part, light smoothing) -------------------------------
n = len(src)
K = np.linalg.norm(src[:, None] - src[None], axis=2) + 2e-4 * np.eye(n)
Pm = np.hstack([np.ones((n, 1)), src])
A = np.zeros((n + 4, n + 4))
A[:n, :n], A[:n, n:], A[n:, :n] = K, Pm, Pm.T
coef = np.linalg.solve(A, np.vstack([dst - src, np.zeros((4, 3))]))


def warp(X):
    X = sim(X)
    out = X.copy()
    for i in range(0, len(X), 4000):
        c = X[i:i + 4000]
        out[i:i + 4000] += np.linalg.norm(c[:, None] - src[None], axis=2) @ coef[:n] + \
            np.hstack([np.ones((len(c), 1)), c]) @ coef[n:]
    return out


fit_err = np.linalg.norm(warp(P_cc[ok]) - dst, axis=1)

chin_z = P_op[152, 2]  # MediaPipe 152 = chin
cb_fit = warp(cb_co)
head_v = cb_fit[:, 2] > chin_z - 0.08
head_tris = cb_tris[head_v[cb_tris].all(1)]

# --- 2b. snap onto the operator skin along the CC5 normal, smoothed -------------------------------
tmp = bpy.data.meshes.new("tmp")
tmp.from_pydata(cb_fit.tolist(), [], cb_tris.tolist())
tmp.update()
nrm = np.empty(len(tmp.vertices) * 3)
tmp.vertices.foreach_get("normal", nrm)
nrm = nrm.reshape(-1, 3)
bpy.data.meshes.remove(tmp)
head_center = (np.array(rig.data.bones["head"].head_local) + np.array(rig.data.bones["head"].tail_local)) / 2
disp = np.zeros_like(cb_fit)
for i in np.where(head_v)[0]:
    q, qn, _, d = op_skin_bvh.find_nearest(Vector(cb_fit[i]), 0.015)
    if q is None or Vector(nrm[i]).dot(qn) < 0.5 or np.dot(nrm[i], cb_fit[i] - head_center) < 0:
        continue
    disp[i] = np.dot(np.array(q) - cb_fit[i], nrm[i]) * nrm[i]
nbr = defaultdict(set)
for a, b, c in cb_tris:
    nbr[a] |= {b, c}; nbr[b] |= {a, c}; nbr[c] |= {a, b}
nbr_l = [list(nbr[i]) for i in range(len(cb_fit))]
for _ in range(8):
    disp = 0.5 * disp + 0.5 * np.array([disp[l].mean(0) if l else disp[i] for i, l in enumerate(nbr_l)])
cb_fit = cb_fit + disp * head_v[:, None]

# --- 3. CC5 shape deltas are carried over with the similarity rotation/scale ----------------------
Mrot = s * R


ct_fit = warp(world_co(cc_tongue))

# --- 4. bind operator verts and bake one shape per CC5 shape --------------------------------------
fit_bvh = bvh(cb_fit, head_tris)
mask = np.zeros(len(op_co))
# Head-weighted vertices take shapes only on the skin: earrings, hair and helmets near the face would
# bend with it (Valen's hoops did with Jaw_Open). Lashes and brows follow face bones (face_feature).
for i in np.where(head_plain & skin_v | face_feature)[0]:
    if face_feature[i]:
        mask[i] = 1.0
        continue
    q, _, _, d = fit_bvh.find_nearest(Vector(op_co[i]), 0.015)
    if q is not None:
        mask[i] = float(np.clip((0.015 - d) / 0.011, 0, 1))
mask[excluded] = 0
tmask = is_tongue.astype(float)

# --- mouth split for a hinged jaw ------------------------------------------------------------------
# The game face is posed bone-by-bone, so its weights aren't built for a rigid jaw rotation. Around the mouth:
#  - face skin: split exactly at the lip seam, by surface distance to the upper vs lower lip centre (the lips
#    touch at the seam but only connect around the corners), blending widely past the corners;
#  - everything else (mouth lining, inner lips): split by the plane through the jaw hinge and the lip line,
#    sharp at the lips, widening past the corners (the lining parts behind the teeth).
import heapq


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


def graph_dist(edges, seeds, limit):
    adj = defaultdict(list)
    L = np.linalg.norm(op_co[edges[:, 0]] - op_co[edges[:, 1]], axis=1)
    for (a, b), l in zip(edges.tolist(), L.tolist()):
        adj[a].append((b, l))
        adj[b].append((a, l))
    d = np.full(len(op_co), np.inf)
    h = [(0.0, x) for x in seeds]
    for x in seeds:
        d[x] = 0
    while h:
        dd, u = heapq.heappop(h)
        if dd > d[u] or dd > limit:
            continue
        for v, l in adj[u]:
            if dd + l < d[v]:
                d[v] = dd + l
                heapq.heappush(h, (dd + l, v))
    return d


op_edges = np.empty(len(me.edges) * 2, dtype=np.int64)
me.edges.foreach_get("vertices", op_edges)
op_edges = op_edges.reshape(-1, 2)
_B = lambda n: np.array(rig.data.bones[n].head_local)
jaw_p = _B("jaw")
mouth_c = (_B("j_lip_top") + _B("j_lip_bot")) / 2
corner_x = abs(_B("j_mouthcorner_le")[0] - mouth_c[0])
lat = np.abs(op_co[:, 0] - mouth_c[0])
m_ = (mouth_c - jaw_p)[1:]
r_ = op_co[:, 1:] - jaw_p[1:]
sin_a = (m_[0] * r_[:, 1] - m_[1] * r_[:, 0]) / (np.linalg.norm(m_) * np.maximum(np.linalg.norm(r_, axis=1), 1e-9))
ang = np.degrees(np.arcsin(np.clip(sin_a, -1, 1)))  # > 0 below the hinge-lip plane
# Sharp between the lips, widening from 1 cm before the corners: split sharply at a corner, the lining
# there is torn into a vertical wall as the jaw turns.
w_ang = 0.6 + np.clip((lat - corner_x + 0.01) / 0.03, 0, 1) * 11.4
f_mouth = smoothstep(-w_ang, w_ang, ang)
E_skin = op_edges[skin_v[op_edges].all(1)]
cand = np.where(skin_v & (np.linalg.norm(op_co - mouth_c, axis=1) < 0.03) & (np.abs(op_co[:, 0] - mouth_c[0]) < 0.003))[0]


def lip_seed(lo, hi):  # front-most midline skin point lo..hi metres above the mouth centre
    c = cand[(op_co[cand, 2] - mouth_c[2] > lo) & (op_co[cand, 2] - mouth_c[2] < hi)]
    return [int(c[np.argmin(op_co[c, 1])])] if len(c) else None


s_up, s_lo = lip_seed(0.003, 0.009), lip_seed(-0.009, -0.003)
lip_gap_mm = None  # masked faces (balaclava, gaiter) have no lip skin: the hinge plane splits the whole mouth
if s_up and s_lo:
    d_up, d_lo = graph_dist(E_skin, s_up, 0.15), graph_dist(E_skin, s_lo, 0.15)
    lip_gap_mm = float(d_up[s_lo[0]]) * 1000
    assert lip_gap_mm > 3 * 1000 * np.linalg.norm(op_co[s_up[0]] - op_co[s_lo[0]]), "lips look joined at the seam"
    seam_ok = skin_v & np.isfinite(d_up) & np.isfinite(d_lo)
    tau = 0.0008 + 0.004 * np.clip((lat - corner_x) / 0.02, 0, 1)
    f_mouth = np.where(seam_ok, 1 / (1 + np.exp(-np.clip((d_up - d_lo) / tau, -50, 50))), f_mouth)
mouth_R = 1 - smoothstep(0.03, 0.05, np.linalg.norm(op_co - mouth_c, axis=1))
# At the lip corners the skin takes CC5's split (blended over the corner, built for opening) instead
# of the seam's: upper lip on the head and lower lip on the jaw meeting in a 1 mm step stretched the
# corners into a vertical wall with the mouth open.
corners = [_B(n) for n in ("j_mouthcorner_le", "j_mouthcorner_ri") if n in rig.data.bones]
corner_d = np.min([np.linalg.norm(op_co - c, axis=1) for c in corners], axis=0)
mouth_R = mouth_R * smoothstep(0.004, 0.012, corner_d)
# shapes near the lips sample CC5 only on their own side of the seam; no smoothing across it
near_lips = (mask > 0) & (np.linalg.norm(op_co - mouth_c, axis=1) < 0.035) & (np.abs(f_mouth - 0.5) > 0.3)
sample_side = np.where(near_lips, (f_mouth > 0.5).astype(int), -1)
lip_side = np.where(near_lips & (lat < corner_x + 0.004), (f_mouth > 0.5).astype(int), -1)
cc_vg = {g.index: g.name for g in cc_body.vertex_groups}
JAWISH = ("CC_Base_JawRoot", "CC_Base_Teeth02", "CC_Base_Tongue01", "CC_Base_Tongue02", "CC_Base_Tongue03")
cc_jaw = np.zeros(len(cb_co))
for v in cc_body.data.vertices:
    tot = sum(g.weight for g in v.groups)
    if tot > 0:
        cc_jaw[v.index] = sum(g.weight for g in v.groups if cc_vg[g.group] in JAWISH) / tot


class Binding:
    """Smooth spatial blend of nearby fitted-CC5 vertices for each masked operator vertex (Gaussian weights,
    radius adapted to the local CC5 vertex spacing), so the result can't jump across creases the way a
    single closest point does. Lip vertices only sample their own lip (side 0 upper / 1 lower). Operator
    vertices farther than FAR from any CC5 vertex (mouth interior) are filled from their mesh neighbours."""
    FAR = 0.010

    def __init__(self, fit_co, usable, vmask, side=None, cc_lower=None, cut_side=None):
        self.idx = np.where(vmask > 0)[0]
        self.w = vmask[self.idx]
        side = np.full(len(op_co), -1) if side is None else side
        sets = {-1: np.where(usable)[0]}
        if cc_lower is not None:
            sets[0], sets[1] = np.where(usable & ~cc_lower)[0], np.where(usable & cc_lower)[0]
        trees = {}
        for k, ids in sets.items():
            t = KDTree(len(ids))
            for j, c in enumerate(fit_co[ids]):
                t.insert(c, j)
            t.balance()
            trees[k] = t
        rows, cols, ws = [], [], []
        self.fill = np.zeros(len(self.idx), bool)
        for j, i in enumerate(self.idx):
            k = side[i] if side[i] in trees else -1
            p = Vector(op_co[i])
            _, _, d0 = trees[k].find(p)
            if d0 > self.FAR:
                self.fill[j] = True
                continue
            R = max(0.004, 2.0 * d0)
            for _, n, d in trees[k].find_range(p, R):
                rows.append(j)
                cols.append(sets[k][n])
                ws.append(math.exp(-(d / (0.5 * R)) ** 2))
        self.rows, self.cols = np.array(rows, dtype=np.int64), np.array(cols, dtype=np.int64)
        self.ws = np.array(ws, dtype=float)
        self.norm = np.maximum(np.bincount(self.rows, weights=self.ws, minlength=len(self.idx)), 1e-12)
        local = -np.ones(len(op_co), dtype=np.int64)
        local[self.idx] = np.arange(len(self.idx))
        e = local[op_edges]
        cut = side if cut_side is None else cut_side
        ga, gb = cut[op_edges[:, 0]], cut[op_edges[:, 1]]
        across_lips = (ga >= 0) & (gb >= 0) & (ga != gb)
        self.e = e[(e >= 0).all(1) & ~across_lips]
        self.L = np.linalg.norm(op_co[self.idx][self.e[:, 0]] - op_co[self.idx][self.e[:, 1]], axis=1)
        self.deg = np.bincount(self.e.ravel(), minlength=len(self.idx)).astype(float)

    def sample(self, field):  # field: (V,) or (V, 3) on the fitted CC5 vertices
        f = field[self.cols]
        if field.ndim == 1:
            return np.bincount(self.rows, weights=f * self.ws, minlength=len(self.idx)) / self.norm
        return np.stack([np.bincount(self.rows, weights=f[:, c] * self.ws, minlength=len(self.idx))
                         for c in range(field.shape[1])], 1) / self.norm[:, None]

    def smooth(self, d, iters, only=None):
        for _ in range(iters):
            acc = np.zeros_like(d)
            np.add.at(acc, self.e[:, 0], d[self.e[:, 1]])
            np.add.at(acc, self.e[:, 1], d[self.e[:, 0]])
            avg = np.where(self.deg[:, None] > 0, acc / np.maximum(self.deg, 1)[:, None], d)
            new = 0.5 * d + 0.5 * avg
            d = new if only is None else np.where(only[:, None], new, d)
        return d

    def transfer(self, delta):
        d = self.sample(delta)
        if self.fill.any():
            d = self.smooth(d, 40, self.fill)
        d = self.smooth(d, 2)
        for _ in range(3):  # extra smoothing only where neighbours still disagree
            jump = np.linalg.norm(d[self.e[:, 0]] - d[self.e[:, 1]], axis=1)
            bad = jump > np.maximum(1.5e-3, 0.8 * self.L)
            if not bad.any():
                break
            region = np.zeros(len(self.idx), bool)
            region[self.e[bad].ravel()] = True
            for _ in range(2):
                region[self.e[region[self.e].any(1)].ravel()] = True
            d = self.smooth(d, 15, region)
        return d * self.w[:, None]


def key_deltas(ob):
    kbs = ob.data.shape_keys.key_blocks
    base = world_co(ob, kbs["Basis"])
    out = {}
    for kb in kbs[1:]:
        d = world_co(ob, kb) - base
        if np.abs(d).max() >= 1e-6:
            out[kb.name] = d @ Mrot.T
    return out


body_bind = Binding(cb_fit, head_v, mask, sample_side, cc_jaw > 0.5, cut_side=lip_side)
tongue_bind = Binding(ct_fit, np.ones(len(ct_fit), bool), tmask)
deltas = defaultdict(lambda: np.zeros_like(op_co))
body_keys, tongue_keys = [], []
for bind, src_ob, names in ((body_bind, cc_body, body_keys), (tongue_bind, cc_tongue, tongue_keys)):
    for k, d in key_deltas(src_ob).items():
        deltas[k][bind.idx] += bind.transfer(d)
        names.append(k)
b_idx, t_idx = body_bind.idx, tongue_bind.idx

# --- 4b. CC5's smooth head/jaw weight split onto the operator's face (cc-render folds face bones
#         into head/jaw anyway; CC5's split is built for a wide jaw opening) ---------------------------
bi = body_bind.idx
f_cc = np.clip(body_bind.smooth(body_bind.sample(cc_jaw)[:, None], 3)[:, 0], 0, 1)
Rm = np.where(body_bind.fill, 1.0, mouth_R[bi])  # mouth interior always uses the hinge split
f_jaw = Rm * f_mouth[bi] + (1 - Rm) * f_cc
reweight = []
for j, i in enumerate(body_bind.idx):
    gs = me.vertices[i].groups
    if is_eye[i] or is_tongue[i] or excluded[i] or not gs:
        continue
    if all(gname[g.group] in head_tree for g in gs if g.weight > 1e-5):
        reweight.append((int(i), float(f_jaw[j])))
by_group = defaultdict(list)
for i, _ in reweight:
    for g in me.vertices[i].groups:
        by_group[g.group].append(i)
for gi, vs in by_group.items():
    op_ob.vertex_groups[gname[gi]].remove(vs)
vg_head, vg_jaw = op_ob.vertex_groups["head"], op_ob.vertex_groups["jaw"]
for i, f in reweight:
    if f < 1 - 1e-4:
        vg_head.add([i], float(1 - f), "REPLACE")
    if f > 1e-4:
        vg_jaw.add([i], float(f), "REPLACE")
KD = ("Nose_Wrinkle_R", "Mouth_UpperLip_Raise_R", "Jaw_Open", "Mouth_Corner_Pull_R")
_kd = key_deltas(cc_body)
np.savez(os.path.join(config.checks_dir(OP, "cc5"), "cc5_debug.npz"), op_co=op_co, cb_fit=cb_fit, head_v=head_v, b_idx=b_idx,
         fill=body_bind.fill, P_op=P_op, P_ccw=warp(P_cc), ok=ok, f_jaw=f_jaw,
         **{"cc_" + k: _kd[k] for k in KD}, **{"op_" + k: deltas[k] for k in KD})
moving = np.zeros(len(op_co), bool)
for d in deltas.values():
    moving |= np.abs(d).max(1) > 1e-6

# --- 5. split the moving face region into its own object, add shape keys ---------------------------
attr = me.attributes.new("orig_idx", "INT", "POINT")
attr.data.foreach_set("value", np.arange(len(me.vertices), dtype=np.int32))
poly_verts = [p.vertices[:] for p in me.polygons]
vm = moving.copy()
for _ in range(3):  # select faces touching moving verts, grown 2 rings: the split border has zero delta
    sel = np.array([vm[list(pv)].any() for pv in poly_verts])
    vm = np.zeros_like(vm)
    for pi in np.where(sel)[0]:
        vm[list(poly_verts[pi])] = True
bpy.ops.object.select_all(action="DESELECT")
op_ob.select_set(True)
bpy.context.view_layer.objects.active = op_ob
bpy.ops.object.mode_set(mode="EDIT")
bm = bmesh.from_edit_mesh(me)
bm.faces.ensure_lookup_table()
for f in bm.faces:
    f.select_set(False)
for f in bm.faces:
    if sel[f.index]:
        f.select_set(True)
bm.select_flush_mode()
bmesh.update_edit_mesh(me)
bpy.ops.mesh.separate(type="SELECTED")
bpy.ops.object.mode_set(mode="OBJECT")
face_ob = next(o for o in bpy.context.selected_objects if o != op_ob)
face_ob.name = face_ob.data.name = f"{OP}_face"
fo = np.empty(len(face_ob.data.vertices), dtype=np.int32)
face_ob.data.attributes["orig_idx"].data.foreach_get("value", fo)
border_delta = max((np.abs(d[fo]).max(1) * ~moving[fo]).max() for d in deltas.values())
fbase = op_co[fo]
face_ob.shape_key_add(name="Basis", from_mix=False)
for k in body_keys + [k for k in tongue_keys if k not in body_keys]:
    sk = face_ob.shape_key_add(name=k, from_mix=False)
    sk.data.foreach_set("co", (fbase + deltas[k][fo]).ravel())
for kb in face_ob.data.shape_keys.key_blocks[1:]:
    kb.value = 0.0  # new keys come in at 1.0 in Blender 5.2
for o in (op_ob, face_ob):
    o.data.attributes.remove(o.data.attributes["orig_idx"])

stats = {
    "landmarks_used": int(ok.sum()), "similarity_scale": round(float(s), 4),
    "landmark_err_after_similarity_mm": [round(float(np.median(sim_err)) * 1000, 2), round(float(sim_err.max()) * 1000, 2)],
    "landmark_err_after_warp_mm": [round(float(np.median(fit_err)) * 1000, 2), round(float(fit_err.max()) * 1000, 2)],
    "snap_mm_median_max": [round(float(np.median(np.linalg.norm(disp[head_v], axis=1))) * 1000, 2),
                           round(float(np.linalg.norm(disp[head_v], axis=1).max()) * 1000, 2)],
    "skin_material": me.materials[skin_mat].name, "bound_face_verts": int(len(b_idx)), "bound_tongue_verts": int(len(t_idx)), "reweighted_head_jaw": len(reweight), "filled_far_verts": int(body_bind.fill.sum()), "lip_seam_surface_gap_mm": round(lip_gap_mm, 1) if lip_gap_mm else "no lip skin (masked face)", "near_lip_verts_upper_lower": [int((sample_side == 0).sum()), int((sample_side == 1).sum())],
    "shape_keys": len(face_ob.data.shape_keys.key_blocks) - 1, "face_object_verts": len(face_ob.data.vertices),
    "body_object_verts": len(op_ob.data.vertices), "max_delta_at_split_border_mm": round(float(border_delta) * 1000, 4),
    "max_delta_mm": {k: round(float(np.abs(deltas[k]).max()) * 1000, 2) for k in
                     ("Eye_Blink_L", "Jaw_Open", "V_Open", "Mouth_Corner_Pull_L", "Tongue_Out", "Brow_Raise_In_L")
                     if k in deltas},
}

# --- save + export (CC5 helper objects removed) ---------------------------------------------------
cc_keep = list(cc_objs)
cc_names = [o.name for o in cc_keep]
for o in list(cc_keep):
    bpy.data.objects.remove(o, do_unlink=True)
bpy.ops.wm.save_as_mainfile(filepath=os.path.join(OUT_DIR, f"{OP}_smplx_cc5.blend"))
bpy.ops.object.select_all(action="DESELECT")
for o in (rig, op_ob, face_ob):
    o.select_set(True)
bpy.context.view_layer.objects.active = rig
bpy.ops.export_scene.fbx(filepath=os.path.join(OUT_DIR, f"{OP}_smplx_cc5.fbx"), use_selection=True,
                         object_types={"ARMATURE", "MESH"}, add_leaf_bones=False, bake_anim=False,
                         use_mesh_modifiers=False, path_mode="ABSOLUTE")
print("CC5_JSON " + json.dumps(stats))
