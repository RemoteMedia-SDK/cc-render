"""Tie a head GLB's beard and mustache cards to the skin they grow from.

The skin around the mouth opens with the face mesh's expression morphs
(Jaw_Open, ...) and its jaw weights; the hair cards carry their own weights
and either no morphs (cards outside the face mesh) or morphs transferred to
each vertex on its own, so with the mouth open the beard lags behind the lips
and slides. Here every card (a connected strip of an alpha-cutout primitive
that the jaw moves) takes the skin weights and every morph delta of the skin
vertex nearest its root, so the whole card travels with that patch of skin.
Cards outside the face mesh are moved into it (same skin) to receive the
morphs. Scalp hair, brows and lashes (no jaw weight) are left alone.

python -I scripts/saluki/bind_cards.py IN.glb OUT.glb   (OUT "-": report only)
"""
import json
import struct
import sys

import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components
from scipy.spatial import cKDTree

src, dst = sys.argv[1], sys.argv[2]
b = open(src, 'rb').read()
jlen = struct.unpack_from('<I', b, 12)[0]
j = json.loads(b[20:20 + jlen])
bin_start = 20 + jlen + 8
blen = struct.unpack_from('<I', b, 20 + jlen)[0]
binary = bytearray(b[bin_start:bin_start + blen])
CT = {5121: np.uint8, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
NC = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}


def view(view_i, byte_off, ctype, nc, count):
    v = j['bufferViews'][view_i]
    dt = np.dtype(CT[ctype])
    off = v.get('byteOffset', 0) + byte_off
    stride = v.get('byteStride', dt.itemsize * nc)
    raw = np.frombuffer(bytes(binary), np.uint8, stride * (count - 1) + dt.itemsize * nc, off)
    return np.lib.stride_tricks.as_strided(raw, (count, dt.itemsize * nc), (stride, 1)).copy().view(dt).reshape(count, nc)


def acc(i, normalize=True):
    a = j['accessors'][i]
    nc = NC[a['type']]
    arr = (view(a['bufferView'], a.get('byteOffset', 0), a['componentType'], nc, a['count'])
           if 'bufferView' in a else np.zeros((a['count'], nc), CT[a['componentType']]))
    if 'sparse' in a:
        s = a['sparse']
        idx = view(s['indices']['bufferView'], s['indices'].get('byteOffset', 0), s['indices']['componentType'], 1, s['count'])[:, 0]
        arr = arr.copy()
        arr[idx] = view(s['values']['bufferView'], s['values'].get('byteOffset', 0), a['componentType'], nc, s['count'])
    if normalize and a.get('normalized'):
        return arr / np.iinfo(arr.dtype).max
    return arr


def add_view(data: bytes) -> int:
    while len(binary) % 4:
        binary.append(0)
    j['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': len(data)})
    binary.extend(data)
    return len(j['bufferViews']) - 1


def add_accessor(arr, ctype, typ, minmax=False):
    arr = np.ascontiguousarray(arr.astype(CT[ctype]))
    a = {'bufferView': add_view(arr.tobytes()), 'componentType': ctype, 'count': len(arr), 'type': typ}
    if minmax:
        a['min'] = arr.min(0).tolist()
        a['max'] = arr.max(0).tolist()
    j['accessors'].append(a)
    return len(j['accessors']) - 1


def add_sparse_vec3(delta):
    """A morph target's POSITION deltas as a sparse accessor (zeros elsewhere)."""
    nz = np.flatnonzero(np.abs(delta).max(1) > 1e-7)
    if len(nz) == 0:
        nz = np.array([0])
    vals = np.ascontiguousarray(delta[nz].astype(np.float32))
    a = {'componentType': 5126, 'count': len(delta), 'type': 'VEC3',
         'min': delta.min(0).astype(np.float32).tolist(), 'max': delta.max(0).astype(np.float32).tolist(),
         'sparse': {'count': int(len(nz)),
                    'indices': {'bufferView': add_view(nz.astype(np.uint32).tobytes()), 'componentType': 5125},
                    'values': {'bufferView': add_view(vals.tobytes())}}}
    j['accessors'].append(a)
    return len(j['accessors']) - 1


mats = j['materials']
skin_i = j['skins'][0]
joint_names = [j['nodes'][k]['name'] for k in skin_i['joints']]
JAW = joint_names.index('CC_Base_JawRoot')

# The face mesh: the one with the expression morphs.
face_mi = next(i for i, m in enumerate(j['meshes']) if m.get('extras', {}).get('targetNames'))
face = j['meshes'][face_mi]
T = len(face['extras']['targetNames'])
target_attrs = sorted({k for p in face['primitives'] for t in p.get('targets', []) for k in t})
print(f"face mesh {face['name']}: {T} morphs ({', '.join(target_attrs)})")


def is_skin(p):
    return mats[p['material']].get('extras', {}).get('cc5', {}).get('shader') == 'RLSkin'


# Every skin vertex: position, joints, weights, and its delta in every morph.
sk_pos, sk_j, sk_w, sk_d = [], [], [], []
for mi, m in enumerate(j['meshes']):
    for p in m['primitives']:
        if not is_skin(p):
            continue
        P = acc(p['attributes']['POSITION'])
        sk_pos.append(P)
        sk_j.append(acc(p['attributes']['JOINTS_0']).astype(np.uint16))
        sk_w.append(acc(p['attributes']['WEIGHTS_0']).astype(np.float32))
        if mi == face_mi and p.get('targets'):
            sk_d.append(np.stack([acc(t['POSITION']) for t in p['targets']], 1))  # (n, T, 3)
        else:
            sk_d.append(np.zeros((len(P), T, 3)))
sk_pos, sk_j, sk_w, sk_d = map(np.concatenate, (sk_pos, sk_j, sk_w, sk_d))
tree = cKDTree(sk_pos)
print(f"skin: {len(sk_pos)} vertices")

moved = []  # (mesh index, primitive) moved into the face mesh
for mi, m in enumerate(j['meshes']):
    for pi, p in enumerate(list(m['primitives'])):
        mat = mats[p['material']]
        if mat.get('alphaMode') != 'MASK' or is_skin(p) or mat.get('extras', {}).get('face_mask'):
            continue  # not cards; removable masks keep their own deformation
        J = acc(p['attributes']['JOINTS_0']).astype(int)
        W = acc(p['attributes']['WEIGHTS_0'])
        if (W * (J == JAW)).sum(1).max() < 0.01:
            continue  # not moved by the jaw: scalp hair, brows, lashes
        P = acc(p['attributes']['POSITION'])
        idx = acc(p['indices']).astype(np.int64).ravel()
        tri = idx.reshape(-1, 3)
        e = np.concatenate([tri[:, [0, 1]], tri[:, [1, 2]], tri[:, [2, 0]]])
        g = coo_matrix((np.ones(len(e)), (e[:, 0], e[:, 1])), shape=(len(P), len(P)))
        ncomp, comp = connected_components(g, directed=False)
        dist, near = tree.query(P)
        # Each card's root: its vertex nearest the skin.
        order = np.lexsort((dist, comp))
        first = np.ones(len(order), bool)
        first[1:] = comp[order][1:] != comp[order][:-1]
        root_of_comp = np.empty(ncomp, np.int64)
        root_of_comp[comp[order][first]] = order[first]
        root = near[root_of_comp[comp]]  # skin vertex each card vertex follows
        # Only small strips are cards; anything larger keeps its own deformation.
        size = np.bincount(comp)
        lo = np.full((ncomp, 3), np.inf)
        hi = np.full((ncomp, 3), -np.inf)
        np.minimum.at(lo, comp, P)
        np.maximum.at(hi, comp, P)
        card = (size <= 400) & (np.linalg.norm(hi - lo, axis=1) <= 0.06)
        bind = card[comp]
        if mi == face_mi and p.get('targets'):
            own_d = np.stack([acc(t['POSITION']) for t in p['targets']], 1)
        else:
            own_d = np.zeros((len(P), T, 3))
        new_jv = np.where(bind[:, None], sk_j[root], J.astype(np.uint16))
        new_wv = np.where(bind[:, None], sk_w[root], W.astype(np.float32))
        new_dv = np.where(bind[:, None, None], sk_d[root], own_d)
        new_j = add_accessor(new_jv, 5123, 'VEC4')
        new_w = add_accessor(new_wv, 5126, 'VEC4')
        q = dict(p)
        q['attributes'] = dict(p['attributes'], JOINTS_0=new_j, WEIGHTS_0=new_w)
        q['targets'] = [{'POSITION': add_sparse_vec3(new_dv[:, t])} for t in range(T)]
        rd = dist[root_of_comp[card]] if card.any() else np.zeros(1)
        if not card.all():
            print(f"  {(~card).sum()} pieces too large for cards ({size[~card].sum()} vertices) keep their deformation")
        print(f"{m['name'][:24]}[{pi}] {mat['name'][:28]}: {len(P)} vertices, {card.sum()} cards, "
              f"root to skin median {np.median(rd) * 1000:.1f} mm (max {rd.max() * 1000:.0f})"
              + ("" if mi == face_mi else f"; moved into {face['name']}"))
        if mi == face_mi:
            face['primitives'][pi] = q
        else:
            moved.append((mi, p, q))

for mi, p, q in moved:
    j['meshes'][mi]['primitives'].remove(p)
    face['primitives'].append(q)
# Primitives that had no targets in the face mesh get zero ones (all share the count).
for p in face['primitives']:
    if len(p.get('targets', [])) != T:
        n = j['accessors'][p['attributes']['POSITION']]['count']
        p['targets'] = [{'POSITION': add_sparse_vec3(np.zeros((n, 3)))} for _ in range(T)]
for t_attr in target_attrs:
    if t_attr != 'POSITION':
        for p in face['primitives']:
            for t in p['targets']:
                t.setdefault(t_attr, None)
            # an attribute missing on some targets is dropped rather than left dangling
            for t in p['targets']:
                if t.get(t_attr) is None:
                    t.pop(t_attr, None)

if dst == '-':
    sys.exit(0)  # report only
while len(binary) % 4:
    binary.append(0)
j['buffers'][0]['byteLength'] = len(binary)
js = json.dumps(j, separators=(',', ':')).encode()
js += b' ' * (-len(js) % 4)
out = bytearray(b'glTF') + struct.pack('<II', 2, 12 + 8 + len(js) + 8 + len(binary))
out += struct.pack('<I', len(js)) + b'JSON' + js
out += struct.pack('<I', len(binary)) + b'BIN\x00' + binary
open(dst, 'wb').write(out)
print(f"wrote {dst} ({len(out) / 1e6:.0f} MB)")
