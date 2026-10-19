// Real-time XPBD cloth on the GPU (f32). The host records one compute pass per frame; every
// substep is a fixed sequence of dispatches of the entry points below (see cloth_gpu.rs):
//
//   materials (1 workgroup)      integrate, coloured spring passes and grips on positions held
//                                in workgroup memory
//   hash_* + find_candidates     spatial hash of particles and vertex-triangle candidates
//   contact_pairs, apply_acc     cloth-cloth contact with friction; applying the averaged
//                                corrections re-projects each particle on the colliders
//   collide_faces, apply_acc     face samples on the colliders, once per substep
//   damping (1 workgroup)        velocities, motion bounds (tunnelling flags), finiteness, and
//                                edge dashpots on velocities held in workgroup memory
//
// Same material model as the CPU reference (cloth_solver.rs): area-lumped particles, XPBD
// distance springs for stretch and opposite-vertex bending with Rayleigh damping (multipliers
// reset per substep, `passes` Gauss-Seidel passes over graph-coloured batches), the compliant
// grip clamp with a force capacity, edge dashpots, Coulomb contact friction at position level.
// Contact differs by design: proximity vertex-triangle contact between all layers from a spatial
// hash, solved by averaged (Jacobi) passes, instead of the CPU's certified swept contact. Motion
// that could outrun the candidate set is counted (tunnelling flags), never clamped.

struct Params {
    gravity: vec4<f32>,
    dt: f32,
    substeps: u32,
    vertices: u32,
    colours: u32,
    triangles: u32,
    colliders: u32,
    grips: u32,
    passes: u32,
    contact_iterations: u32,
    rebuild_every: u32,
    cell: f32,
    margin: f32,
    candidate_capacity: u32,
    edge_damping: f32,
    springs: u32,
    relaxation: f32,
}

struct Step {
    substep: u32,
    s0: f32,
    s1: f32,
    relax: f32,
}

struct Layer {
    radius: f32,
    stick: f32,
    slide: f32,
    drag: f32,
}

struct Spring {
    a: u32,
    b: u32,
    rest: f32,
    compliance: f32,
    damping_ratio: f32,
    stretch: u32,
    _pad0: u32,
    _pad1: u32,
}

// kind: 0 rounded box (half extents, border), 1 sphere (radius in half.w), 2 capsule along local
// y (half height in half.y, radius in half.w). Pose interpolates from (p0, q0) to (p1, q1) across
// the frame; lo/hi bound it over the frame, grown by the thickest cloth radius.
struct Collider {
    lo: vec4<f32>,
    hi: vec4<f32>,
    p0: vec4<f32>,
    q0: vec4<f32>,
    p1: vec4<f32>,
    q1: vec4<f32>,
    half: vec4<f32>,
    kind: u32,
    mask: u32,
    friction: f32,
    border: f32,
}

struct Grip {
    ids: vec4<u32>,      // xyz vertices, w state: 0 held, 1 released, 2 overloaded
    weights: vec4<f32>,  // xyz barycentric
    material: vec4<f32>, // compliance m/N, damping ratio, capacity N, _
    peaks: vec4<f32>,    // required, applied (written back)
}

struct Counters {
    candidates: atomic<u32>,
    candidate_overflow: atomic<u32>,
    substep_motion_exceeded: atomic<u32>,
    margin_exceeded: atomic<u32>,
    contacts: atomic<u32>,
    nonfinite: atomic<u32>,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> pos: array<vec4<f32>>;   // xyz, w inverse mass
@group(0) @binding(2) var<storage, read_write> prev: array<vec4<f32>>;  // substep start
@group(0) @binding(3) var<storage, read_write> vel: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> vlayer: array<u32>;
@group(0) @binding(5) var<storage, read_write> layers: array<Layer>;
@group(0) @binding(6) var<storage, read_write> springs: array<Spring>;
@group(0) @binding(7) var<storage, read_write> colour_start: array<u32>;
@group(0) @binding(8) var<storage, read_write> lambda: array<f32>;
@group(0) @binding(9) var<storage, read_write> tris: array<vec4<u32>>;     // abc, layer
@group(0) @binding(10) var<storage, read_write> cell_count: array<atomic<u32>>;
@group(0) @binding(11) var<storage, read_write> cell_start: array<u32>;
@group(0) @binding(12) var<storage, read_write> sorted: array<u32>;
@group(0) @binding(13) var<storage, read_write> cand: array<vec2<u32>>;
@group(0) @binding(14) var<storage, read_write> acc: array<atomic<i32>>;  // 4 per vertex
@group(0) @binding(15) var<storage, read_write> colliders: array<Collider>;
@group(0) @binding(16) var<storage, read_write> grips: array<Grip>;
@group(0) @binding(17) var<storage, read_write> anchors: array<vec4<f32>>; // grips x (substeps+1)
@group(0) @binding(18) var<storage, read_write> counters: Counters;
@group(0) @binding(19) var<storage, read_write> built: array<vec4<f32>>;  // positions at rebuild
// Outward normal of the collider each particle rests on (w = 1), from its latest collider pass.
@group(0) @binding(20) var<storage, read_write> support: array<vec4<f32>>;

var<push_constant> step: Step;

const ONE_GROUP: u32 = 1024u;
const WIDE: u32 = 256u;
const TABLE: u32 = 16384u;
const FIXED: f32 = 1e8; // accumulator units per metre

// Workgroup copies of particle coordinates (or velocities) for the one-workgroup stages.
var<workgroup> wx: array<f32, 4096>;
var<workgroup> wy: array<f32, 4096>;
var<workgroup> wz: array<f32, 4096>;
var<workgroup> partial: array<u32, 1024>;

fn substep_h() -> f32 {
    return params.dt / f32(params.substeps);
}

fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

fn quat_conj(q: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(-q.xyz, q.w);
}

fn pose_q(c: Collider, s: f32) -> vec4<f32> {
    var q1 = c.q1;
    if (dot(c.q0, q1) < 0.0) { q1 = -q1; }
    return normalize(mix(c.q0, q1, s));
}

fn cell_of(p: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(floor(p / params.cell));
}

fn hash_cell(c: vec3<i32>) -> u32 {
    let h = (u32(c.x) * 73856093u) ^ (u32(c.y) * 19349663u) ^ (u32(c.z) * 83492791u);
    return h & (TABLE - 1u);
}

fn add_acc(i: u32, d: vec3<f32>) {
    atomicAdd(&acc[i * 4u], i32(round(d.x * FIXED)));
    atomicAdd(&acc[i * 4u + 1u], i32(round(d.y * FIXED)));
    atomicAdd(&acc[i * 4u + 2u], i32(round(d.z * FIXED)));
    atomicAdd(&acc[i * 4u + 3u], 1);
}

fn wpos(i: u32) -> vec3<f32> {
    return vec3<f32>(wx[i], wy[i], wz[i]);
}

fn set_wpos(i: u32, p: vec3<f32>) {
    wx[i] = p.x;
    wy[i] = p.y;
    wz[i] = p.z;
}

// Signed distance of local point l to the collider's core shape (no cloth radius) and the
// outward local normal.
fn shape_distance(c: Collider, l: vec3<f32>) -> vec4<f32> {
    if (c.kind == 1u) {
        let len = length(l);
        var n = vec3<f32>(0.0, 1.0, 0.0);
        if (len > 1e-12) { n = l / len; }
        return vec4<f32>(n, len - c.half.w);
    }
    if (c.kind == 2u) {
        let y = clamp(l.y, -c.half.y, c.half.y);
        let d = l - vec3<f32>(0.0, y, 0.0);
        let len = length(d);
        var n = vec3<f32>(1.0, 0.0, 0.0);
        if (len > 1e-12) { n = d / len; }
        return vec4<f32>(n, len - c.half.w);
    }
    let q = abs(l) - c.half.xyz;
    let outside = max(q, vec3<f32>(0.0));
    let len = length(outside);
    let s = select(vec3<f32>(-1.0), vec3<f32>(1.0), l >= vec3<f32>(0.0));
    var n: vec3<f32>;
    if (len > 0.0) {
        n = outside / len * s;
    } else if (q.x >= q.y && q.x >= q.z) {
        n = vec3<f32>(s.x, 0.0, 0.0);
    } else if (q.y >= q.z) {
        n = vec3<f32>(0.0, s.y, 0.0);
    } else {
        n = vec3<f32>(0.0, 0.0, s.z);
    }
    return vec4<f32>(n, len + min(max(q.x, max(q.y, q.z)), 0.0) - c.border);
}

// Closest point on triangle (a, b, c) to p as barycentric weights (Ericson, RTCD 5.1.5).
fn closest_weights(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if (d1 <= 0.0 && d2 <= 0.0) { return vec3<f32>(1.0, 0.0, 0.0); }
    let bp = p - b;
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if (d3 >= 0.0 && d4 <= d3) { return vec3<f32>(0.0, 1.0, 0.0); }
    let vc = d1 * d4 - d3 * d2;
    if (vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0) {
        let v = d1 / (d1 - d3);
        return vec3<f32>(1.0 - v, v, 0.0);
    }
    let cp = p - c;
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if (d6 >= 0.0 && d5 <= d6) { return vec3<f32>(0.0, 0.0, 1.0); }
    let vb = d5 * d2 - d1 * d6;
    if (vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0) {
        let w = d2 / (d2 - d6);
        return vec3<f32>(1.0 - w, 0.0, w);
    }
    let va = d3 * d6 - d5 * d4;
    if (va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0) {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return vec3<f32>(0.0, 1.0 - w, w);
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    return vec3<f32>(1.0 - v - w, v, w);
}

// ---- materials: one workgroup, positions in workgroup memory ----

fn project_springs(lid: u32, h: f32) {
    for (var colour = 0u; colour < params.colours; colour++) {
        let begin = colour_start[colour];
        let end = colour_start[colour + 1u];
        for (var k = begin + lid; k < end; k += ONE_GROUP) {
            let s = springs[k];
            let wa = pos[s.a].w;
            let wb = pos[s.b].w;
            let w = wa + wb;
            if (w <= 0.0) { continue; }
            let pa = wpos(s.a);
            let pb = wpos(s.b);
            let delta = pa - pb;
            let len = length(delta);
            if (len <= 1e-12) { continue; }
            let n = delta / len;
            // XPBD Eq. 26 with Rayleigh damping, as the CPU springs.
            let gamma = 2.0 * s.damping_ratio * sqrt(s.compliance / w) / h;
            let alpha = s.compliance / (h * h);
            let rel = dot(n, (pa - prev[s.a].xyz) - (pb - prev[s.b].xyz));
            let dl = -(len - s.rest + alpha * lambda[k] + gamma * rel) / ((1.0 + gamma) * w + alpha);
            lambda[k] += dl;
            set_wpos(s.a, pa + n * dl * wa);
            set_wpos(s.b, pb - n * dl * wb);
        }
        workgroupBarrier();
    }
}

fn project_grips(lid: u32, h: f32, substep: u32) {
    for (var g = lid; g < params.grips; g += ONE_GROUP) {
        var grip = grips[g];
        if (grip.ids.w != 0u) { continue; }
        let ids = grip.ids.xyz;
        let wt = grip.weights.xyz;
        var mass = 0.0;
        var value = vec3<f32>(0.0);
        var before = vec3<f32>(0.0);
        for (var k = 0u; k < 3u; k++) {
            mass += pos[ids[k]].w * wt[k] * wt[k];
            value += wpos(ids[k]) * wt[k];
            before += prev[ids[k]].xyz * wt[k];
        }
        let a0 = anchors[g * (params.substeps + 1u) + substep].xyz;
        let a1 = anchors[g * (params.substeps + 1u) + substep + 1u].xyz;
        let compliance = grip.material.x;
        let rel = value - before - (a1 - a0);
        let gamma = 2.0 * grip.material.y * sqrt(compliance / mass) / h;
        let l = -(value - a1 + gamma * rel) / ((1.0 + gamma) * mass + compliance / (h * h));
        let force = length(l) / (h * h);
        grip.peaks.x = max(grip.peaks.x, force);
        if (force > grip.material.z) {
            grip.ids.w = 2u;
            grips[g] = grip;
            continue;
        }
        grip.peaks.y = max(grip.peaks.y, force);
        for (var k = 0u; k < 3u; k++) {
            set_wpos(ids[k], wpos(ids[k]) + l * (pos[ids[k]].w * wt[k]));
        }
        grips[g] = grip;
    }
    workgroupBarrier();
}

@compute @workgroup_size(1024)
fn materials(@builtin(local_invocation_index) lid: u32) {
    let h = substep_h();
    for (var i = lid; i < params.vertices; i += ONE_GROUP) {
        let p = pos[i];
        prev[i] = p;
        var v = vel[i].xyz;
        if (p.w > 0.0) {
            v = v * exp(-layers[vlayer[i]].drag * h) + params.gravity.xyz * h;
        }
        set_wpos(i, p.xyz + v * h);
    }
    for (var k = lid; k < params.springs; k += ONE_GROUP) { lambda[k] = 0.0; }
    storageBarrier();
    workgroupBarrier();
    for (var pass_index = 0u; pass_index < params.passes; pass_index++) {
        project_springs(lid, h);
        if (pass_index + 1u == params.passes) { project_grips(lid, h, step.substep); }
    }
    for (var i = lid; i < params.vertices; i += ONE_GROUP) {
        pos[i] = vec4<f32>(wpos(i), pos[i].w);
    }
}

// ---- spatial hash and candidates ----

@compute @workgroup_size(256)
fn hash_clear(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x < TABLE) { atomicStore(&cell_count[gid.x], 0u); }
    if (gid.x == 0u) { atomicStore(&counters.candidates, 0u); }
}

@compute @workgroup_size(256)
fn hash_count(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.vertices) { return; }
    atomicAdd(&cell_count[hash_cell(cell_of(pos[i].xyz))], 1u);
    built[i] = pos[i];
}

// Exclusive prefix sum of the TABLE cell counts (16 per thread, then a workgroup scan); clears
// the counts for the scatter.
@compute @workgroup_size(1024)
fn hash_scan(@builtin(local_invocation_index) lid: u32) {
    let per = TABLE / ONE_GROUP;
    var local_sum = 0u;
    for (var k = 0u; k < per; k++) { local_sum += atomicLoad(&cell_count[lid * per + k]); }
    partial[lid] = local_sum;
    workgroupBarrier();
    for (var offset = 1u; offset < ONE_GROUP; offset *= 2u) {
        var add = 0u;
        if (lid >= offset) { add = partial[lid - offset]; }
        workgroupBarrier();
        partial[lid] += add;
        workgroupBarrier();
    }
    var running = partial[lid] - local_sum;
    for (var k = 0u; k < per; k++) {
        let slot = lid * per + k;
        cell_start[slot] = running;
        running += atomicLoad(&cell_count[slot]);
        atomicStore(&cell_count[slot], 0u);
    }
}

@compute @workgroup_size(256)
fn hash_scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.vertices) { return; }
    let h = hash_cell(cell_of(pos[i].xyz));
    sorted[cell_start[h] + atomicAdd(&cell_count[h], 1u)] = i;
}

@compute @workgroup_size(256)
fn find_candidates(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= params.triangles) { return; }
    let tri = tris[t];
    let a = pos[tri.x].xyz;
    let b = pos[tri.y].xyz;
    let c = pos[tri.z].xyz;
    let grow = layers[tri.w].radius + params.margin;
    let blo = min(a, min(b, c)) - vec3<f32>(grow);
    let bhi = max(a, max(b, c)) + vec3<f32>(grow);
    // Cells are searched with the particle radius bounded by the margin as well.
    let lo = cell_of(blo - vec3<f32>(params.margin));
    let hi = cell_of(bhi + vec3<f32>(params.margin));
    for (var x = lo.x; x <= hi.x; x++) {
        for (var y = lo.y; y <= hi.y; y++) {
            for (var z = lo.z; z <= hi.z; z++) {
                let h = hash_cell(vec3<i32>(x, y, z));
                let count = atomicLoad(&cell_count[h]);
                for (var k = 0u; k < count; k++) {
                    let v = sorted[cell_start[h] + k];
                    if (v == tri.x || v == tri.y || v == tri.z) { continue; }
                    let p = pos[v].xyz;
                    // A bucket shared by colliding cells: count each vertex once, in its own cell.
                    if (any(cell_of(p) != vec3<i32>(x, y, z))) { continue; }
                    let r = vec3<f32>(layers[vlayer[v]].radius);
                    if (any(p < blo - r) || any(p > bhi + r)) { continue; }
                    let slot = atomicAdd(&counters.candidates, 1u);
                    if (slot < params.candidate_capacity) {
                        cand[slot] = vec2<u32>(v, t);
                    } else {
                        atomicAdd(&counters.candidate_overflow, 1u);
                    }
                }
            }
        }
    }
}

// ---- colliders ----

// Particle contact with every fixed or kinematic collider of its layer, at the collider's pose
// interpolated to the end of this substep. Friction acts on the slip relative to the collider
// surface's own motion over the substep.
@compute @workgroup_size(256)
fn collide_vertices(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.vertices) { return; }
    collide_vertex(i);
}

fn collide_vertex(i: u32) {
    var p = pos[i];
    support[i] = vec4<f32>(0.0);
    if (p.w <= 0.0) { return; }
    let layer = vlayer[i];
    let radius = layers[layer].radius;
    for (var k = 0u; k < params.colliders; k++) {
        let c = colliders[k];
        if ((c.mask & (1u << layer)) == 0u) { continue; }
        if (any(p.xyz < c.lo.xyz) || any(p.xyz > c.hi.xyz)) { continue; }
        let q1 = pose_q(c, step.s1);
        let o1 = mix(c.p0.xyz, c.p1.xyz, step.s1);
        let l = quat_rotate(quat_conj(q1), p.xyz - o1);
        let sd = shape_distance(c, l);
        let depth = radius - sd.w;
        // Resting within a micrometre of the collider counts as supported by it.
        if (depth > -1e-6) { support[i] = vec4<f32>(quat_rotate(q1, sd.xyz), 1.0); }
        if (depth <= 0.0) { continue; }
        let n = quat_rotate(q1, sd.xyz);
        var x = p.xyz + n * depth;
        let q0 = pose_q(c, step.s0);
        let o0 = mix(c.p0.xyz, c.p1.xyz, step.s0);
        let surface_motion = (o1 + quat_rotate(q1, l)) - (o0 + quat_rotate(q0, l));
        let slip_vec = (x - prev[i].xyz) - surface_motion;
        let tangent = slip_vec - n * dot(slip_vec, n);
        let slip = length(tangent);
        if (slip > 0.0) {
            x -= tangent * min(c.friction * depth / slip, 1.0);
        }
        p = vec4<f32>(x, p.w);
    }
    pos[i] = p;
}

// Face contact: a face draped over a convex edge can cut it between its vertices. Sample the
// centroid and edge midpoints; the deepest penetrating sample moves the face by its barycentric
// weights (the CPU reference's triangle contact, with its witness approximated by samples).
@compute @workgroup_size(256)
fn collide_faces(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= params.triangles) { return; }
    let tri = tris[t];
    let layer = tri.w;
    let radius = layers[layer].radius;
    let ids = tri.xyz;
    let pa = pos[ids.x];
    let pb = pos[ids.y];
    let pc = pos[ids.z];
    let lo = min(pa.xyz, min(pb.xyz, pc.xyz));
    let hi = max(pa.xyz, max(pb.xyz, pc.xyz));
    for (var k = 0u; k < params.colliders; k++) {
        let c = colliders[k];
        if ((c.mask & (1u << layer)) == 0u) { continue; }
        if (any(hi < c.lo.xyz) || any(lo > c.hi.xyz)) { continue; }
        let q1 = pose_q(c, step.s1);
        let o1 = mix(c.p0.xyz, c.p1.xyz, step.s1);
        var best = 0.0;
        var best_w = vec3<f32>(0.0);
        var best_n = vec3<f32>(0.0);
        var best_l = vec3<f32>(0.0);
        for (var sample = 0u; sample < 4u; sample++) {
            var w = vec3<f32>(1.0 / 3.0);
            if (sample == 1u) { w = vec3<f32>(0.5, 0.5, 0.0); }
            if (sample == 2u) { w = vec3<f32>(0.0, 0.5, 0.5); }
            if (sample == 3u) { w = vec3<f32>(0.5, 0.0, 0.5); }
            let x = pa.xyz * w.x + pb.xyz * w.y + pc.xyz * w.z;
            let l = quat_rotate(quat_conj(q1), x - o1);
            let sd = shape_distance(c, l);
            let depth = radius - sd.w;
            if (depth > best) {
                best = depth;
                best_w = w;
                best_n = quat_rotate(q1, sd.xyz);
                best_l = l;
            }
        }
        if (best <= 0.0) { continue; }
        let w = best_w;
        let mass = pa.w * w.x * w.x + pb.w * w.y * w.y + pc.w * w.z * w.z;
        if (mass <= 0.0) { continue; }
        let q0 = pose_q(c, step.s0);
        let o0 = mix(c.p0.xyz, c.p1.xyz, step.s0);
        let surface_motion = (o1 + quat_rotate(q1, best_l)) - (o0 + quat_rotate(q0, best_l));
        let moved = (pa.xyz - prev[ids.x].xyz) * w.x + (pb.xyz - prev[ids.y].xyz) * w.y
            + (pc.xyz - prev[ids.z].xyz) * w.z - surface_motion;
        let tangent = moved - best_n * dot(moved, best_n);
        let slip = length(tangent);
        var friction = vec3<f32>(0.0);
        if (slip > 0.0) { friction = tangent * min(c.friction * best / slip, 1.0); }
        let correction = (best_n * best - friction) / mass;
        add_acc(ids.x, correction * pa.w * w.x);
        add_acc(ids.y, correction * pb.w * w.y);
        add_acc(ids.z, correction * pc.w * w.z);
    }
}

// Apply the averaged accumulated corrections (relaxed by step.relax), clear them, then project
// the particle against the colliders again.
@compute @workgroup_size(256)
fn apply_acc(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.vertices) { return; }
    let count = atomicLoad(&acc[i * 4u + 3u]);
    if (count > 0) {
        let d = vec3<f32>(f32(atomicLoad(&acc[i * 4u])), f32(atomicLoad(&acc[i * 4u + 1u])),
            f32(atomicLoad(&acc[i * 4u + 2u]))) * step.relax / (FIXED * f32(count));
        pos[i] = vec4<f32>(pos[i].xyz + d, pos[i].w);
        for (var k = 0u; k < 4u; k++) { atomicStore(&acc[i * 4u + k], 0); }
    }
    collide_vertex(i);
}

// ---- cloth-cloth contact ----

@compute @workgroup_size(256)
fn contact_pairs(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let n = min(atomicLoad(&counters.candidates), params.candidate_capacity);
    for (var k = gid.x; k < n; k += groups.x * WIDE) {
        let pair = cand[k];
        let v = pair.x;
        let tri = tris[pair.y];
        let pv = pos[v];
        let pa = pos[tri.x];
        let pb = pos[tri.y];
        let pc = pos[tri.z];
        let gap = layers[vlayer[v]].radius + layers[tri.w].radius;
        let wts = closest_weights(pv.xyz, pa.xyz, pb.xyz, pc.xyz);
        let q = pa.xyz * wts.x + pb.xyz * wts.y + pc.xyz * wts.z;
        let d = pv.xyz - q;
        let dist = length(d);
        if (dist >= gap + 2.0 * params.margin) { continue; }
        // Side from the substep-start configuration: a vertex that crossed the face this substep
        // is returned to the side it came from.
        let a0 = prev[tri.x].xyz;
        let b0 = prev[tri.y].xyz;
        let c0 = prev[tri.z].xyz;
        let n0 = cross(b0 - a0, c0 - a0);
        let q0 = a0 * wts.x + b0 * wts.y + c0 * wts.z;
        let face = cross(pb.xyz - pa.xyz, pc.xyz - pa.xyz);
        let face_len = length(face);
        if (face_len <= 1e-12) { continue; }
        var side = 1.0;
        if (dot(prev[v].xyz - q0, n0) < 0.0) { side = -1.0; }
        let fn_ = face / face_len * side;
        var normal = fn_;
        var separation = dot(d, fn_);
        let interior = wts.x > 1e-4 && wts.y > 1e-4 && wts.z > 1e-4;
        if (!interior) {
            // Beside the face (closest point on an edge or corner): a contact only within the
            // gap, along the true separation. The signed plane distance applies only over the
            // face itself; cloth curving past an edge is not behind the face.
            if (dist >= gap || dist <= 1e-9) { continue; }
            normal = d / dist;
            separation = dist;
        }
        let depth = gap - separation;
        if (depth <= 0.0) { continue; }
        // Response directions of the four particles. A particle resting on a collider cannot be
        // pushed into it: that part of its response is removed (the CPU reference's fixed
        // supports), so the free layer takes the correction instead of the supported one
        // being pushed into the mattress and straight back every pass.
        let ids = vec4<u32>(v, tri.x, tri.y, tri.z);
        let coef = vec4<f32>(pv.w, -pa.w * wts.x, -pb.w * wts.y, -pc.w * wts.z);
        let wt4 = vec4<f32>(1.0, wts.x, wts.y, wts.z);
        var dirs: array<vec3<f32>, 4>;
        var w = 0.0;
        for (var k = 0u; k < 4u; k++) {
            var dk = normal * coef[k];
            let sup = support[ids[k]];
            if (sup.w > 0.0) {
                dk -= sup.xyz * min(dot(dk, sup.xyz), 0.0);
            }
            dirs[k] = dk;
            w += dot(dk, normal) * select(-wt4[k], 1.0, k == 0u);
        }
        if (w <= 1e-12) { continue; }
        let l = depth / w;
        // Coulomb friction between the two material points (geometric-mean coefficients).
        let la = layers[vlayer[v]];
        let lb = layers[tri.w];
        let stick = sqrt(la.stick * lb.stick);
        let slide = sqrt(la.slide * lb.slide);
        let rel = (pv.xyz - prev[v].xyz)
            - ((pa.xyz - a0) * wts.x + (pb.xyz - b0) * wts.y + (pc.xyz - c0) * wts.z);
        let tangent = rel - normal * dot(rel, normal);
        let slip = length(tangent);
        var cancel = 0.0;
        if (slip > 0.0) {
            if (slip <= stick * depth) { cancel = slip; } else { cancel = min(slide * depth, slip); }
        }
        var t_dir = vec3<f32>(0.0);
        if (slip > 0.0) { t_dir = tangent / slip; }
        let wf = pv.w + pa.w * wts.x * wts.x + pb.w * wts.y * wts.y + pc.w * wts.z * wts.z;
        let lt = cancel / wf;
        let slide_back = t_dir * lt;
        add_acc(v, dirs[0] * l - slide_back * pv.w);
        add_acc(tri.x, dirs[1] * l + slide_back * pa.w * wts.x);
        add_acc(tri.y, dirs[2] * l + slide_back * pb.w * wts.y);
        add_acc(tri.z, dirs[3] * l + slide_back * pc.w * wts.z);
        atomicAdd(&counters.contacts, 1u);
    }
}

// ---- end of substep ----

// Velocities from the substep's motion, motion bounds (tunnelling flags) and finiteness, then
// edge dashpots as the CPU's damp_edge_velocities, over the coloured stretch springs, on
// velocities held in workgroup memory.
@compute @workgroup_size(1024)
fn damping(@builtin(local_invocation_index) lid: u32) {
    let h = substep_h();
    let keep = 1.0 - exp(-params.edge_damping * h);
    for (var i = lid; i < params.vertices; i += ONE_GROUP) {
        let p = pos[i];
        let motion = p.xyz - prev[i].xyz;
        set_wpos(i, motion / h);
        let radius = layers[vlayer[i]].radius;
        if (length(motion) > 2.0 * radius) { atomicAdd(&counters.substep_motion_exceeded, 1u); }
        if (length(p.xyz - built[i].xyz) > 0.5 * params.margin) { atomicAdd(&counters.margin_exceeded, 1u); }
        if (any(p.xyz != p.xyz) || any(abs(p.xyz) > vec3<f32>(1e4))) { atomicAdd(&counters.nonfinite, 1u); }
    }
    workgroupBarrier();
    for (var colour = 0u; colour < params.colours; colour++) {
        let begin = colour_start[colour];
        let end = colour_start[colour + 1u];
        for (var k = begin + lid; k < end; k += ONE_GROUP) {
            let s = springs[k];
            if (s.stretch == 0u) { continue; }
            let wa = pos[s.a].w;
            let wb = pos[s.b].w;
            if (wa + wb <= 0.0) { continue; }
            let c = (wpos(s.a) - wpos(s.b)) * keep / (wa + wb);
            set_wpos(s.a, wpos(s.a) - c * wa);
            set_wpos(s.b, wpos(s.b) + c * wb);
        }
        workgroupBarrier();
    }
    for (var i = lid; i < params.vertices; i += ONE_GROUP) { vel[i] = vec4<f32>(wpos(i), 0.0); }
}
