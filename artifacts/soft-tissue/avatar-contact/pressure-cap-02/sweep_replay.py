import re, sys, numpy as np
line = open(sys.argv[1]).read()
vecs = [np.array(list(map(float, m))) for m in re.findall(r'DVec3\(([-0-9.e]+), ([-0-9.e]+), ([-0-9.e]+)\)', line)]
clearance = float(re.search(r'clearance ([0-9.]+)', line).group(1))
old, end = vecs[:4], vecs[4:8]
def closest(p, a, b, c):
    # Ericson, Real-Time Collision Detection 5.1.5
    ab, ac, ap = b - a, c - a, p - a
    d1, d2 = ab @ ap, ac @ ap
    if d1 <= 0 and d2 <= 0: return a
    bp = p - b; d3, d4 = ab @ bp, ac @ bp
    if d3 >= 0 and d4 <= d3: return b
    vc = d1 * d4 - d3 * d2
    if vc <= 0 and d1 >= 0 and d3 <= 0: return a + d1 / (d1 - d3) * ab
    cp = p - c; d5, d6 = ab @ cp, ac @ cp
    if d6 >= 0 and d5 <= d6: return c
    vb = d5 * d2 - d1 * d6
    if vb <= 0 and d2 >= 0 and d6 <= 0: return a + d2 / (d2 - d6) * ac
    va = d3 * d6 - d5 * d4
    if va <= 0 and (d4 - d3) >= 0 and (d5 - d6) >= 0: return b + (d4 - d3) / ((d4 - d3) + (d5 - d6)) * (c - b)
    denom = 1 / (va + vb + vc); return a + ab * vb * denom + ac * vc * denom
motion = [e - o for o, e in zip(old, end)]
speed = max(np.linalg.norm(motion[0] - motion[i]) for i in range(1, 4))
n = np.cross(old[2] - old[1], old[3] - old[1]); n /= np.linalg.norm(n)
print('clearance', clearance, 'relative speed per step %.3e m' % speed)
for t in [0, 0.25, 0.5, 0.75, 1]:
    at = [o + t * m for o, m in zip(old, motion)]
    q = closest(at[0], *at[1:]); d = np.linalg.norm(at[0] - q)
    print('t=%.2f distance %.6e  signed-plane %.6e' % (t, d, (at[0] - at[1]) @ n))
t = 0.0
for k in range(256):
    at = [o + t * m for o, m in zip(old, motion)]
    q = closest(at[0], *at[1:]); d = np.linalg.norm(at[0] - q)
    if d <= clearance + 1e-8: print('hit at iteration', k, 't', t); break
    nrm = (at[0] - q) / d
    if all(all((p[0] - p[i]) @ nrm > clearance + 1e-8 for i in range(1, 4)) for p in [at, end]): print('separated at', k); break
    if speed < 1e-12 or d - clearance > speed * (1 - t): print('cannot reach at', k, 't', t, 'gap', d - clearance); break
    t += 0.9 * (d - clearance) / speed
    if t > 1: print('passed end at', k); break
else:
    print('256 iterations exhausted; t', t, 'gap', d - clearance)
