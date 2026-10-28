"""Conform a character to a target shape with its CC5 sliders (needs numpy + scipy).

The pack's linear model, shape(w) = base + sum_i t_i * delta_i, turns "make our
avatar look like this mesh" into bounded least squares, iterated with closest-point
matching because the target (a scan, another character, an SMPL body) has its own
topology:

    for each iteration:
        pair our vertices with the target surface (and target samples with our
        nearest vertices, so the shape cannot shrink into part of the target)
        solve  min |A t - r|^2 + reg |t|^2  with every t_i in [0, 1]

Each slider has up to two captured sides (towards max / towards min), so it starts as
two columns; afterwards every slider keeps its dominant side and the fit is solved
again with one column per slider, so the result is a valid slider weight vector that
`Pack.shape_glb` and the editor accept as is.

    import cc5_shape
    from cc5_shape import fit
    pack = cc5_shape.Pack("pack")
    base = fit.base_mesh(pack, open("character.glb", "rb").read())
    target_v, target_f = fit.load_mesh("target.glb")            # or .obj
    result = fit.fit(pack, base, target_v, target_f, meshes=["CC_Base_Body"],
                     categories=["Actor/Body/", "Actor/HD Body/"])
    shaped = pack.shape_glb(open("character.glb", "rb").read(), result.weights)

Positions are in the glTF frame (Y-up, meters). The target should stand in roughly the
same pose as the character's bind pose (CC5 A-pose).
"""
from __future__ import annotations

import json
import struct
from dataclasses import dataclass, field

import numpy as np

_DT = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
_N = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


def _glb(data: bytes):
    if data[:4] != b"glTF":
        raise ValueError("not a GLB file")
    n = struct.unpack_from("<I", data, 12)[0]
    g = json.loads(data[20:20 + n])
    binoff = 20 + n + 8

    def acc(i):
        a = g["accessors"][i]
        bv = g["bufferViews"][a["bufferView"]]
        dt, k = _DT[a["componentType"]], _N[a["type"]]
        start = binoff + bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = bv.get("byteStride", 0)
        item = np.dtype(dt).itemsize * k
        if stride and stride != item:
            raw = np.frombuffer(data, np.uint8, stride * (a["count"] - 1) + item, start)
            return np.stack([raw[j * stride:j * stride + item].view(dt) for j in range(a["count"])])
        out = np.frombuffer(data, dt, a["count"] * k, start)
        return out.reshape(-1, k) if k > 1 else out
    return g, acc


def _node_worlds(g):
    nodes = g.get("nodes", [])
    parent = {c: i for i, n in enumerate(nodes) for c in n.get("children", [])}

    def local(n):
        if "matrix" in n:
            return np.array(n["matrix"], float).reshape(4, 4).T
        x, y, z, w = n.get("rotation", [0, 0, 0, 1])
        R = np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                      [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                      [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])
        M = np.eye(4)
        M[:3, :3] = R * np.array(n.get("scale", [1, 1, 1]))
        M[:3, 3] = n.get("translation", [0, 0, 0])
        return M
    W = {}

    def world(i):
        if i not in W:
            W[i] = (world(parent[i]) if i in parent else np.eye(4)) @ local(nodes[i])
        return W[i]
    return [world(i) for i in range(len(nodes))]


@dataclass
class Base:
    """Our character's bind mesh in the pack's global vertex space."""
    verts: np.ndarray          # (vertex_count, 3)
    faces: np.ndarray          # (F, 3) global vertex indices
    mesh_of: np.ndarray        # (vertex_count,) index into names
    names: list[str]

    def mask(self, meshes=None) -> np.ndarray:
        if not meshes:
            return np.ones(len(self.verts), bool)
        ids = [self.names.index(m) for m in meshes]
        return np.isin(self.mesh_of, ids)


def base_mesh(pack, glb: bytes) -> Base:
    """Bind mesh of the character GLB (with `_CCIDX`), in the pack's vertex space."""
    g, acc = _glb(glb)
    offsets = {name: off for name, off, _ in pack.meshes}
    names = [name for name, _, _ in pack.meshes]
    verts = np.full((pack.vertex_count, 3), np.nan)
    mesh_of = np.zeros(pack.vertex_count, int)
    for name, off, cnt in pack.meshes:
        mesh_of[off:off + cnt] = names.index(name)
    faces = []
    for m in g["meshes"]:
        if m.get("name") not in offsets:
            continue
        off = offsets[m["name"]]
        for p in m["primitives"]:
            at = p["attributes"]
            if "_CCIDX" not in at:
                raise ValueError("mesh %s has no _CCIDX attribute" % m["name"])
            gi = off + acc(at["_CCIDX"]).astype(np.int64)
            verts[gi] = acc(at["POSITION"])
            if "indices" in p:
                faces.append(gi[acc(p["indices"]).astype(np.int64)].reshape(-1, 3))
    if np.isnan(verts).any():
        raise ValueError("%d pack vertices are not in the GLB" % int(np.isnan(verts[:, 0]).sum()))
    return Base(verts, np.concatenate(faces) if faces else np.zeros((0, 3), int), mesh_of, names)


def load_mesh(path: str, meshes=None):
    """(vertices (V,3), faces (F,3)) of a .glb (all mesh primitives, node transforms
    applied, Y-up meters) or .obj file. `meshes`: only glTF meshes with these names."""
    if path.lower().endswith(".obj"):
        v, f = [], []
        for line in open(path, "rb"):
            if line.startswith(b"v "):
                v.append([float(x) for x in line.split()[1:4]])
            elif line.startswith(b"f "):
                idx = [int(t.split(b"/")[0]) for t in line.split()[1:]]
                idx = [i - 1 if i > 0 else len(v) + i for i in idx]
                f.extend([idx[0], idx[k], idx[k + 1]] for k in range(1, len(idx) - 1))
        return np.array(v, float), np.array(f, int).reshape(-1, 3)
    g, acc = _glb(open(path, "rb").read())
    worlds = _node_worlds(g)
    vs, fs, n = [], [], 0
    for i, node in enumerate(g.get("nodes", [])):
        if "mesh" not in node:
            continue
        m = g["meshes"][node["mesh"]]
        if meshes and m.get("name") not in meshes:
            continue
        # skinned meshes are drawn in bind space; others through their node
        W = np.eye(4) if "skin" in node else worlds[i]
        for p in m["primitives"]:
            pos = acc(p["attributes"]["POSITION"]).astype(float)
            vs.append(pos @ W[:3, :3].T + W[:3, 3])
            if "indices" in p:
                fs.append(acc(p["indices"]).astype(int).reshape(-1, 3) + n)
            n += len(pos)
    return np.concatenate(vs), (np.concatenate(fs) if fs else np.zeros((0, 3), int))


def sample_surface(v, f, n, seed=0, normals=False):
    """n points spread over the triangles' area (the vertices if there are no faces);
    with normals=True also their unit normals (from the triangles, or estimated from
    the 16 nearest points for a bare point cloud)."""
    if len(f) == 0:
        pts = np.asarray(v, float)
        if not normals:
            return pts
        from scipy.spatial import cKDTree
        _, nb = cKDTree(pts).query(pts, 16)
        nrm = np.empty_like(pts)
        for i, idx in enumerate(nb):
            q = pts[idx] - pts[idx].mean(0)
            nrm[i] = np.linalg.svd(q, full_matrices=False)[2][2]
        return pts, nrm
    rng = np.random.default_rng(seed)
    a, b, c = v[f[:, 0]], v[f[:, 1]], v[f[:, 2]]
    cr = np.cross(b - a, c - a)
    area = np.linalg.norm(cr, axis=1)
    tri = rng.choice(len(f), n, p=area / area.sum())
    u, w = rng.random((2, n))
    flip = u + w > 1
    u[flip], w[flip] = 1 - u[flip], 1 - w[flip]
    pts = a[tri] + u[:, None] * (b[tri] - a[tri]) + w[:, None] * (c[tri] - a[tri])
    if not normals:
        return pts
    return pts, cr[tri] / np.maximum(area[tri], 1e-20)[:, None]


def vertex_normals(v, f):
    """Area-weighted unit vertex normals."""
    nrm = np.zeros_like(v, dtype=float)
    cr = np.cross(v[f[:, 1]] - v[f[:, 0]], v[f[:, 2]] - v[f[:, 0]])
    for k in range(3):
        np.add.at(nrm, f[:, k], cr)
    return nrm / np.maximum(np.linalg.norm(nrm, axis=1), 1e-20)[:, None]


@dataclass
class FitResult:
    weights: list[float]                 # full slider weight vector (pack order)
    changed: dict[str, float]            # slider id -> weight, for sliders that moved
    transform: np.ndarray                # 4x4 applied to the target (target -> our frame)
    history: list[dict] = field(default_factory=list)  # per iteration: mean/p95 distance (m)

    def save_recipe(self, pack, path):
        """Write the fit as a recipe (open it in the editor, or pack.weights_from_recipe)."""
        with open(path, "w", encoding="utf-8") as f:
            f.write(pack.recipe(self.weights))


def _columns(pack, sliders, mask):
    """Sparse columns over masked vertices: list of (slider index, side, rows, deltas)."""
    gidx = np.flatnonzero(mask)
    pos = np.full(len(mask), -1)
    pos[gidx] = np.arange(len(gidx))
    cols = []
    for i in sliders:
        s = pack.sliders[i]
        for which in ("max", "min"):
            st = pack.state(i, which)
            if st is None or abs(st[0] - s["base"]) < 1e-9:
                continue
            vi = np.frombuffer(st[1], "<u4").astype(np.int64)
            vd = np.frombuffer(st[2], "<f4").reshape(-1, 3).astype(float)
            keep = pos[vi] >= 0
            if keep.any():
                cols.append((i, which, st[0], pos[vi[keep]], vd[keep]))
    return gidx, cols


def _dense(cols, rows, n):
    A = np.zeros((n, 3, len(cols)))
    for k, (_, _, _, r, d) in enumerate(cols):
        A[r, :, k] = d
    return A[rows]


def _solve(A, r, reg, lo, hi):
    """min |A t - r|^2 + reg |t|^2, lo <= t <= hi, via the normal equations (fast for
    many rows: the bounded problem is solved on the n x n Cholesky factor)."""
    from scipy.optimize import lsq_linear
    G = A.T @ A + reg * np.eye(A.shape[1])
    h = A.T @ r
    L = np.linalg.cholesky(G)
    y = np.linalg.solve(L, h)
    return lsq_linear(L.T, y, bounds=(lo, hi), method="bvls").x


def target_from_cc_glb(pack, glb: bytes, meshes=("CC_Base_Body",)) -> np.ndarray:
    """Per-vertex target positions (pack vertex space, NaN where absent) from another
    CC character's GLB with `_CCIDX` (same CC base topology), for `fit(..., matched=...)`."""
    g, acc = _glb(glb)
    offsets = {name: off for name, off, _ in pack.meshes}
    out = np.full((pack.vertex_count, 3), np.nan)
    for m in g["meshes"]:
        if m.get("name") in offsets and m.get("name") in meshes:
            for p in m["primitives"]:
                at = p["attributes"]
                if "_CCIDX" in at:
                    out[offsets[m["name"]] + acc(at["_CCIDX"]).astype(np.int64)] = acc(at["POSITION"])
    return out


def fit(pack, base: Base, target_v=None, target_f=None, *, meshes=("CC_Base_Body",), categories=None,
        sliders=None, iterations=8, reg=1e-4, align="translate", samples=80000, stride=1,
        reverse_weight=1.0, max_dist=0.05, min_normal_dot=0.5, matched=None, landmarks=None,
        landmark_weight=100.0, verbose=True) -> FitResult:
    """Fit slider weights so `base` matches the target surface.

    meshes          our meshes that must match (default: the body skin only)
    categories      slider category prefixes to use (e.g. ["Actor/Body/"]); or `sliders`
                    as explicit slider indices; default: every slider
    align           "translate" (feet on the floor, centred), "similarity" (also scale),
                    or "none" (target already in our frame)
    reg             pull towards the character's own design (larger = smaller changes)
    stride          use every n-th of our vertices in the solve (speed)
    max_dist        ignore pairs further apart than this (m)
    min_normal_dot  ignore pairs whose surfaces face different ways (e.g. toe vs toe)
    matched         (vertex_count, 3) known target position per vertex (NaN = none),
                    e.g. target_from_cc_glb() for another CC character: solved directly,
                    point to point; target_v may then be None
    landmarks       {our global vertex index: target point (target frame)} pinning
                    features a surface fit lets slide (eye corners, nose tip, chin...),
                    weighted by landmark_weight

    Pairs are point-to-plane: only the distance along the surface normal counts, so
    vertices may slide along the target surface (sampling density does not bias it).
    """
    from scipy.spatial import cKDTree
    target_v = None if target_v is None else np.asarray(target_v, float)
    target_f = np.zeros((0, 3), int) if target_f is None else np.asarray(target_f, int)
    mask = base.mask(meshes)
    if sliders is None:
        sliders = [i for i, s in enumerate(pack.sliders)
                   if categories is None or s["category"].startswith(tuple(categories))]
    ours0 = base.verts[mask]

    T = np.eye(4)
    if target_v is None and matched is None:
        raise ValueError("give target_v (a surface) and/or matched (per-vertex targets)")
    if target_v is None:
        target_v = matched[mask][~np.isnan(matched[mask][:, 0])]
        surface = False
    else:
        surface = True
    if align in ("translate", "similarity"):
        s = 1.0
        if align == "similarity":
            s = np.ptp(ours0[:, 1]) / np.ptp(target_v[:, 1])
        c_t = np.array([np.mean(target_v[:, 0]), target_v[:, 1].min(), np.mean(target_v[:, 2])])
        c_o = np.array([np.mean(ours0[:, 0]), ours0[:, 1].min(), np.mean(ours0[:, 2])])
        T[:3, :3] *= s
        T[:3, 3] = c_o - s * c_t
    elif align != "none":
        raise ValueError("align must be translate, similarity or none")
    def to_ours(p):
        return np.asarray(p, float) @ T[:3, :3].T + T[:3, 3]
    if target_v is not None:
        tv = to_ours(target_v)
        tpts, tnrm = sample_surface(tv, target_f, samples, normals=True)
        ttree = cKDTree(tpts)

    gidx, cols = _columns(pack, sliders, mask)
    n = len(gidx)
    rows = np.arange(0, n, stride)
    A3 = _dense(cols, rows, n)                      # (rows, 3, C)
    local = np.full(len(mask), -1)
    local[gidx] = np.arange(n)
    our_faces = local[base.faces[np.all(mask[base.faces], axis=1)]]
    rev_pts, rev_nrm = tpts[::2], tnrm[::2]
    history = []

    def shape(tt, cc):
        out = ours0.copy()
        for k, (_, _, _, r, d) in enumerate(cc):
            if tt[k]:
                out[r] += tt[k] * d
        return out

    pin_rows, pin_pts = np.zeros(0, int), np.zeros((0, 3))
    if matched is not None:
        mm = to_ours(matched[gidx])
        have = ~np.isnan(mm[:, 0])
        pin_rows, pin_pts = np.flatnonzero(have), mm[have]
        pin_w = np.ones(len(pin_rows))
    if landmarks:
        lr = np.array([local[g] for g in landmarks], int)
        if (lr < 0).any():
            raise ValueError("landmark vertices must be in the fitted meshes")
        pin_rows = np.concatenate([pin_rows, lr])
        pin_pts = np.vstack([pin_pts, to_ours(np.array(list(landmarks.values()), float))])
        pin_w = np.concatenate([pin_w if matched is not None else np.zeros(0), np.full(len(lr), landmark_weight)])
    A_pin = _dense(cols, pin_rows, n) if len(pin_rows) else None

    def system(cc, tt, A, keep=None):
        pins = None
        if A_pin is not None:
            Ap = A_pin if keep is None else A_pin[:, :, keep]
            sw = np.sqrt(pin_w)[:, None, None]
            pins = ((Ap * sw).reshape(-1, Ap.shape[2]), ((pin_pts - ours0[pin_rows]) * sw[:, :, 0]).ravel())
        if not surface:
            cur = shape(tt, cc)
            d = np.linalg.norm(cur[pin_rows] - pin_pts, axis=1)
            return pins[0], pins[1], {"mean_m": float(d.mean()), "p95_m": float(np.percentile(d, 95)), "pairs": len(d)}
        cur = shape(tt, cc)
        onrm = vertex_normals(cur, our_faces)
        cr, on = cur[rows], onrm[rows]
        # forward: our vertex -> nearest target point, along the target normal
        d, j = ttree.query(cr)
        ok = (d < max_dist) & (np.abs(np.sum(tnrm[j] * on, 1)) > min_normal_dot)
        nf = tnrm[j[ok]]
        Af = np.einsum("rk,rkc->rc", nf, A[ok])
        bf = np.sum(nf * (tpts[j[ok]] - ours0[rows][ok]), 1)
        # reverse: target point -> our nearest vertex, along our normal
        dr, k = cKDTree(cr).query(rev_pts)
        okr = (dr < max_dist) & (np.abs(np.sum(rev_nrm * on[k], 1)) > min_normal_dot)
        nr = on[k[okr]]
        Ar = np.einsum("rk,rkc->rc", nr, A[k[okr]]) * np.sqrt(reverse_weight)
        br = np.sum(nr * (rev_pts[okr] - ours0[rows][k[okr]]), 1) * np.sqrt(reverse_weight)
        plane = np.abs(np.sum(nf * (tpts[j[ok]] - cr[ok]), 1))
        stats = {"mean_m": float(plane.mean()), "p95_m": float(np.percentile(plane, 95)),
                 "pairs": int(ok.sum() + okr.sum())}
        if pins is not None:
            return np.vstack([Af, Ar, pins[0]]), np.concatenate([bf, br, pins[1]]), stats
        return np.vstack([Af, Ar]), np.concatenate([bf, br]), stats

    t = np.zeros(len(cols))
    if not surface:
        iterations = 1  # point to point with known pairs: linear, one solve
    for it in range(iterations):
        A, b, st = system(cols, t, A3)
        history.append(dict(st, iteration=it))
        if verbose:
            print("fit %d: distance mean %.2f mm p95 %.2f mm (%d pairs)"
                  % (it, st["mean_m"] * 1e3, st["p95_m"] * 1e3, st["pairs"]))
        t = _solve(A, b, reg, 0.0, 1.0)

    # one side per slider: keep the dominant side, re-solve with single columns
    best = {}
    for k, (i, which, w, r, d) in enumerate(cols):
        mag = t[k] * np.abs(d).sum()
        if i not in best or mag > best[i][0]:
            best[i] = (mag, k)
    keep = sorted(k for _, k in best.values())
    cols1 = [cols[k] for k in keep]
    A31 = A3[:, :, keep]
    t1 = t[keep]
    for _ in range(3 if surface else 1):
        A, b, st = system(cols1, t1, A31, keep)
        t1 = _solve(A, b, reg, 0.0, 1.0)
    _, _, st = system(cols1, t1, A31, keep)
    history.append(dict(st, iteration="final"))
    if verbose:
        print("fit final: distance mean %.2f mm p95 %.2f mm" % (st["mean_m"] * 1e3, st["p95_m"] * 1e3))

    weights = pack.base_weights()
    changed = {}
    for tk, (i, which, w, _, _) in zip(t1, cols1):
        if tk > 1e-4:
            s = pack.sliders[i]
            weights[i] = float(s["base"] + tk * (w - s["base"]))
            changed[s["id"]] = weights[i]
    return FitResult(weights, changed, T, history)
