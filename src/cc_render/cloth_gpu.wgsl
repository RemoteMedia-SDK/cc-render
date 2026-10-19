// Real-time XPBD cloth on the GPU (f32). The host records one compute pass per frame; every
// substep is a fixed sequence of dispatches of the entry points below (see cloth_gpu.rs):
//
//   materials (1 workgroup)      integrate, coloured spring passes and grips on positions held
//                                in workgroup memory
//   hash_* + find_candidates     spatial hash of particles and vertex-triangle candidates
//   contact_pairs, apply_acc     cloth-cloth contact with friction; applying the averaged
//                                corrections re-projects each particle on the colliders
//   collide_faces, apply_acc     face samples on the colliders, once per substep
//   finish (all particles)       velocities, motion bounds (tunnelling flags), finiteness, and
//                                edge dashpots
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
    edges: u32,
    edge_slot_capacity: u32,
    propagate_supports: u32,
    _pad1: u32,
}

struct Step {
    substep: u32,
    s0: f32,
    s1: f32,
    relax: f32,
    // contact_pairs: also process the edge-edge candidates (once per substep).
    edges_on: u32,
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
    wa: f32, // inverse masses of a and b (static), saving two loads per projection
    wb: f32,
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
    ambiguous: atomic<u32>,
    witness: atomic<u32>,
    edge_candidates: atomic<u32>,
    edge_slots: atomic<u32>,
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
@group(0) @binding(21) var<storage, read_write> edges: array<vec4<u32>>;     // a, b, layer
@group(0) @binding(22) var<storage, read_write> ecount: array<atomic<u32>>;
@group(0) @binding(23) var<storage, read_write> estart: array<u32>;
@group(0) @binding(24) var<storage, read_write> eslots: array<u32>;
@group(0) @binding(25) var<storage, read_write> ecand: array<vec2<u32>>;
// Stretch-edge neighbours of every particle (CSR): neighbours[adjacency[i] .. adjacency[i + 1]].
@group(0) @binding(26) var<storage, read_write> adjacency: array<u32>;
@group(0) @binding(27) var<storage, read_write> neighbours: array<u32>;
// Support normals propagated by this contact pass (4 per particle; see solve_pair).
@group(0) @binding(28) var<storage, read_write> propagate: array<atomic<i32>>;

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

// Barycentric weights of p's projection on the plane of (a, b, c), unclamped.
fn plane_weights(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = p - a;
    let d00 = dot(v0, v0);
    let d01 = dot(v0, v1);
    let d11 = dot(v1, v1);
    let d20 = dot(v2, v0);
    let d21 = dot(v2, v1);
    let den = d00 * d11 - d01 * d01;
    if (abs(den) <= 1e-20) { return vec3<f32>(-1.0); }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    return vec3<f32>(1.0 - v - w, v, w);
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
            let wa = s.wa;
            let wb = s.wb;
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
        support[i] = vec4<f32>(0.0);
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
    let held = atomicLoad(&propagate[i * 4u + 3u]);
    if (held > 0) {
        let n = vec3<f32>(f32(atomicLoad(&propagate[i * 4u])), f32(atomicLoad(&propagate[i * 4u + 1u])),
            f32(atomicLoad(&propagate[i * 4u + 2u])));
        // A collider support stands; otherwise the layers beneath hold it.
        if (length(n) > 0.0 && support[i].w == 0.0) { support[i] = vec4<f32>(normalize(n), 2.0); }
        for (var k = 0u; k < 4u; k++) { atomicStore(&propagate[i * 4u + k], 0); }
    }
    collide_vertex(i);
}

// ---- cloth-cloth contact ----

// One contact between two material points: particles ids with signed weights g (the contact
// point difference is sum g_k x_k), along `normal`, short of the gap by `depth`. Supported
// particles cannot be pushed into their collider (the CPU reference's fixed supports), so the
// free side takes the correction; Coulomb friction (static `stick`, kinetic `slide`) reduces
// the relative slip of the two points since the substep start.
//
// Supports propagate up a resting stack: a particle pressed against particles that are
// themselves supported (by a collider, or through the layers beneath) is held by that
// reaction too, and becomes supported for the next pass (`propagate`, applied in apply_acc).
// The light top sheet between the heavy duvet and the supported fitted sheet then passes the
// correction up instead of being driven through the fitted sheet. A lifted flap touches no
// supported particle and is unaffected.
fn solve_pair(ids: vec4<u32>, g: vec4<f32>, normal: vec3<f32>, depth: f32, stick: f32, slide: f32) {
    var dirs: array<vec3<f32>, 4>;
    var w = 0.0;
    var wf = 0.0;
    var rel = vec3<f32>(0.0);
    for (var k = 0u; k < 4u; k++) {
        let im = pos[ids[k]].w;
        var dk = normal * (im * g[k]);
        let sup = support[ids[k]];
        if (sup.w > 0.0) { dk -= sup.xyz * min(dot(dk, sup.xyz), 0.0); }
        dirs[k] = dk;
        w += dot(dk, normal) * g[k];
        wf += im * g[k] * g[k];
        rel += (pos[ids[k]].xyz - prev[ids[k]].xyz) * g[k];
    }
    if (w <= 1e-12 || wf <= 0.0) { return; }
    let l = depth / w;
    let tangent = rel - normal * dot(rel, normal);
    let slip = length(tangent);
    var cancel = 0.0;
    if (slip > 0.0) {
        if (slip <= stick * depth) { cancel = slip; } else { cancel = min(slide * depth, slip); }
    }
    var back = vec3<f32>(0.0);
    if (slip > 0.0) { back = tangent / slip * (cancel / wf); }
    // Which side rests on a supported side: every particle with real weight on that side is
    // supported with its normal facing the other side.
    var a_held = true;
    var b_held = true;
    for (var k = 0u; k < 4u; k++) {
        let sup = support[ids[k]];
        if (abs(g[k]) < 0.05) { continue; }
        if (g[k] > 0.0) {
            a_held = a_held && sup.w > 0.0 && dot(sup.xyz, -normal) > 0.5;
        } else {
            b_held = b_held && sup.w > 0.0 && dot(sup.xyz, normal) > 0.5;
        }
    }
    for (var k = 0u; k < 4u; k++) {
        add_acc(ids[k], dirs[k] * l - back * (pos[ids[k]].w * g[k]));
        if (params.propagate_supports != 0u && abs(g[k]) >= 0.05 && ((g[k] > 0.0 && b_held) || (g[k] < 0.0 && a_held))) {
            let n = normal * sign(g[k]);
            atomicAdd(&propagate[ids[k] * 4u], i32(round(n.x * 1e6)));
            atomicAdd(&propagate[ids[k] * 4u + 1u], i32(round(n.y * 1e6)));
            atomicAdd(&propagate[ids[k] * 4u + 2u], i32(round(n.z * 1e6)));
            atomicAdd(&propagate[ids[k] * 4u + 3u], 1);
        }
    }
    atomicAdd(&counters.contacts, 1u);
}

fn vertex_face(v: u32, t: u32) {
    let tri = tris[t];
    let pv = pos[v];
    let pa = pos[tri.x];
    let pb = pos[tri.y];
    let pc = pos[tri.z];
    let gap = layers[vlayer[v]].radius + layers[tri.w].radius;
    let wts = closest_weights(pv.xyz, pa.xyz, pb.xyz, pc.xyz);
    let q = pa.xyz * wts.x + pb.xyz * wts.y + pc.xyz * wts.z;
    let d = pv.xyz - q;
    let dist = length(d);
    if (dist >= gap + 2.0 * params.margin) { return; }
    // Which side of the face the vertex is on, from the substep start: the plane heights at
    // the start (h0) and now (h1). A vertex that changed sides while over the face crossed it
    // this substep and is returned to the side it came from, wherever its closest point now is
    // (crossing near an edge ends with the closest point on that edge).
    let a0 = prev[tri.x].xyz;
    let b0 = prev[tri.y].xyz;
    let c0 = prev[tri.z].xyz;
    let n0_raw = cross(b0 - a0, c0 - a0);
    let face = cross(pb.xyz - pa.xyz, pc.xyz - pa.xyz);
    let face_len = length(face);
    let n0_len = length(n0_raw);
    if (face_len <= 1e-12 || n0_len <= 1e-12) { return; }
    let n1 = face / face_len;
    let h0 = dot(prev[v].xyz - a0, n0_raw / n0_len);
    let h1 = dot(pv.xyz - pa.xyz, n1);
    let over = plane_weights(pv.xyz - n1 * h1, pa.xyz, pb.xyz, pc.xyz);
    let over_face = all(over >= vec3<f32>(-0.02));
    let interior = wts.x > 1e-4 && wts.y > 1e-4 && wts.z > 1e-4;
    var side = select(-1.0, 1.0, h0 > 0.0);
    if (over_face && abs(h0) < 1e-7) {
        // In the face's plane at the substep start (a crossing the previous substep left just
        // unresolved): it came from the side it is moving away from. At rest relative to the
        // face it has no trustworthy side (the CPU reference's initially intersecting surface)
        // and the frame fails.
        let approach = dot(vel[v].xyz - (vel[tri.x].xyz * wts.x + vel[tri.y].xyz * wts.y
            + vel[tri.z].xyz * wts.z), n0_raw / n0_len);
        if (abs(approach) < 1e-4) {
            atomicAdd(&counters.ambiguous, 1u);
            return;
        }
        side = select(1.0, -1.0, approach > 0.0);
    }
    let crossed = h0 * h1 < 0.0 && over_face;
    var normal = n1 * side;
    var separation = h1 * side;
    if (!crossed && !interior) {
        // Beside the face (closest point on an edge or corner): a contact only within the gap,
        // along the true separation.
        if (dist >= gap) { return; }
        if (dist > 1e-9) {
            normal = d / dist;
            separation = dist;
        }
    }
    let depth = gap - separation;
    if (depth <= 0.0) { return; }
    let la = layers[vlayer[v]];
    let lb = layers[tri.w];
    solve_pair(vec4<u32>(v, tri.x, tri.y, tri.z), vec4<f32>(1.0, -wts.x, -wts.y, -wts.z), normal,
        depth, sqrt(la.stick * lb.stick), sqrt(la.slide * lb.slide));
}

// Closest points of segments p0p1 and q0q1: parameters (s, t) (Ericson, RTCD 5.1.9).
fn segment_params(p0: vec3<f32>, p1: vec3<f32>, q0: vec3<f32>, q1: vec3<f32>) -> vec2<f32> {
    let d1 = p1 - p0;
    let d2 = q1 - q0;
    let r = p0 - q0;
    let a = dot(d1, d1);
    let e = dot(d2, d2);
    let f = dot(d2, r);
    if (a <= 1e-20 && e <= 1e-20) { return vec2<f32>(0.0); }
    var s = 0.0;
    var t = 0.0;
    if (a <= 1e-20) {
        t = clamp(f / e, 0.0, 1.0);
    } else {
        let c = dot(d1, r);
        if (e <= 1e-20) {
            s = clamp(-c / a, 0.0, 1.0);
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            if (denom > 1e-20) { s = clamp((b * f - c * e) / denom, 0.0, 1.0); }
            t = (b * s + f) / e;
            if (t < 0.0) {
                t = 0.0;
                s = clamp(-c / a, 0.0, 1.0);
            } else if (t > 1.0) {
                t = 1.0;
                s = clamp((b - c) / a, 0.0, 1.0);
            }
        }
    }
    return vec2<f32>(s, t);
}

// Edge-edge contact: edges can pass through each other without any vertex crossing a face
// (two coarse sheets bending over the mattress's rounded edges). The side comes from the same
// segment parameters at the substep start; an edge pair that swapped sides is returned.
fn edge_edge(e: u32, f: u32) {
    let ea = edges[e];
    let fa = edges[f];
    let p0 = pos[ea.x].xyz;
    let p1 = pos[ea.y].xyz;
    let q0 = pos[fa.x].xyz;
    let q1 = pos[fa.y].xyz;
    let gap = layers[ea.z].radius + layers[fa.z].radius;
    let st = segment_params(p0, p1, q0, q1);
    let d = mix(p0, p1, st.x) - mix(q0, q1, st.y);
    let dist = length(d);
    if (dist >= gap + 2.0 * params.margin) { return; }
    let d0 = mix(prev[ea.x].xyz, prev[ea.y].xyz, st.x) - mix(prev[fa.x].xyz, prev[fa.y].xyz, st.y);
    let d0_len = length(d0);
    let inner = st.x > 1e-4 && st.x < 1.0 - 1e-4 && st.y > 1e-4 && st.y < 1.0 - 1e-4;
    var start_dir = d0;
    if (inner && d0_len < 1e-7) {
        // Touching at the substep start: the edges came from the sides they are moving away
        // from; at rest relative to each other there is no trustworthy side and the frame fails.
        let vrel = mix(vel[ea.x].xyz, vel[ea.y].xyz, st.x) - mix(vel[fa.x].xyz, vel[fa.y].xyz, st.y);
        if (length(vrel) < 1e-4) {
            atomicAdd(&counters.ambiguous, 1u);
            return;
        }
        start_dir = -vrel;
    }
    var normal: vec3<f32>;
    var separation: f32;
    if (inner && dot(d, start_dir) < 0.0) {
        normal = normalize(start_dir);
        separation = dot(d, normal);
    } else {
        if (dist >= gap) { return; }
        if (dist > 1e-9) { normal = d / dist; } else { normal = normalize(start_dir); }
        separation = dist;
    }
    let depth = gap - separation;
    if (depth <= 0.0) { return; }
    let la = layers[ea.z];
    let lb = layers[fa.z];
    solve_pair(vec4<u32>(ea.x, ea.y, fa.x, fa.y),
        vec4<f32>(1.0 - st.x, st.x, -(1.0 - st.y), -st.y), normal, depth,
        sqrt(la.stick * lb.stick), sqrt(la.slide * lb.slide));
}

// Vertex-face candidates first, then edge-edge candidates, in one grid-stride pass.
@compute @workgroup_size(256)
fn contact_pairs(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let n_vf = min(atomicLoad(&counters.candidates), params.candidate_capacity);
    var n_ee = min(atomicLoad(&counters.edge_candidates), params.candidate_capacity);
    if (step.edges_on == 0u) { n_ee = 0u; }
    for (var k = gid.x; k < n_vf + n_ee; k += groups.x * WIDE) {
        if (k < n_vf) {
            vertex_face(cand[k].x, cand[k].y);
        } else {
            let pair = ecand[k - n_vf];
            edge_edge(pair.x, pair.y);
        }
    }
}

// ---- edge hash: every edge in each cell its grown bounds overlap ----

fn edge_bounds(e: u32) -> array<vec3<f32>, 2> {
    let ed = edges[e];
    let a = pos[ed.x].xyz;
    let b = pos[ed.y].xyz;
    let grow = vec3<f32>(layers[ed.z].radius + params.margin);
    return array<vec3<f32>, 2>(min(a, b) - grow, max(a, b) + grow);
}

@compute @workgroup_size(256)
fn edge_clear(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x < TABLE) { atomicStore(&ecount[gid.x], 0u); }
    if (gid.x == 0u) { atomicStore(&counters.edge_candidates, 0u); }
}

@compute @workgroup_size(256)
fn edge_count(@builtin(global_invocation_id) gid: vec3<u32>) {
    let e = gid.x;
    if (e >= params.edges) { return; }
    let box = edge_bounds(e);
    let lo = cell_of(box[0]);
    let hi = cell_of(box[1]);
    for (var x = lo.x; x <= hi.x; x++) {
        for (var y = lo.y; y <= hi.y; y++) {
            for (var z = lo.z; z <= hi.z; z++) {
                atomicAdd(&ecount[hash_cell(vec3<i32>(x, y, z))], 1u);
            }
        }
    }
}

@compute @workgroup_size(1024)
fn edge_scan(@builtin(local_invocation_index) lid: u32) {
    let per = TABLE / ONE_GROUP;
    var local_sum = 0u;
    for (var k = 0u; k < per; k++) { local_sum += atomicLoad(&ecount[lid * per + k]); }
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
        estart[slot] = running;
        running += atomicLoad(&ecount[slot]);
        atomicStore(&ecount[slot], 0u);
    }
    // Total cell insertions, for the capacity check.
    if (lid == ONE_GROUP - 1u) { atomicStore(&counters.edge_slots, running); }
}

@compute @workgroup_size(256)
fn edge_scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let e = gid.x;
    if (e >= params.edges) { return; }
    let box = edge_bounds(e);
    let lo = cell_of(box[0]);
    let hi = cell_of(box[1]);
    for (var x = lo.x; x <= hi.x; x++) {
        for (var y = lo.y; y <= hi.y; y++) {
            for (var z = lo.z; z <= hi.z; z++) {
                let h = hash_cell(vec3<i32>(x, y, z));
                let slot = estart[h] + atomicAdd(&ecount[h], 1u);
                if (slot < params.edge_slot_capacity) { eslots[slot] = e; }
            }
        }
    }
}

@compute @workgroup_size(256)
fn edge_candidates(@builtin(global_invocation_id) gid: vec3<u32>) {
    let e = gid.x;
    if (e >= params.edges) { return; }
    if (atomicLoad(&counters.edge_slots) > params.edge_slot_capacity) {
        if (e == 0u) { atomicAdd(&counters.candidate_overflow, 1u); }
        return;
    }
    let ea = edges[e];
    let box = edge_bounds(e);
    let lo = cell_of(box[0]);
    let hi = cell_of(box[1]);
    for (var x = lo.x; x <= hi.x; x++) {
        for (var y = lo.y; y <= hi.y; y++) {
            for (var z = lo.z; z <= hi.z; z++) {
                let h = hash_cell(vec3<i32>(x, y, z));
                let count = atomicLoad(&ecount[h]);
                for (var k = 0u; k < count; k++) {
                    let f = eslots[estart[h] + k];
                    if (f <= e) { continue; }
                    let fa = edges[f];
                    if (ea.x == fa.x || ea.x == fa.y || ea.y == fa.x || ea.y == fa.y) { continue; }
                    let other = edge_bounds(f);
                    if (any(box[1] < other[0]) || any(other[1] < box[0])) { continue; }
                    // Record the pair once: in the cell holding the low corner of the overlap.
                    if (any(cell_of(max(box[0], other[0])) != vec3<i32>(x, y, z))) { continue; }
                    let slot = atomicAdd(&counters.edge_candidates, 1u);
                    if (slot < params.candidate_capacity) {
                        ecand[slot] = vec2<u32>(e, f);
                    } else {
                        atomicAdd(&counters.candidate_overflow, 1u);
                    }
                }
            }
        }
    }
}

// ---- end of substep ----

// Velocities from the substep's motion, motion bounds (tunnelling flags) and finiteness, then
// the edge dashpots of the CPU's damp_edge_velocities: each stretch edge decays its endpoints'
// relative velocity by `keep`, mass-weighted. With keep = 1 - exp(-20/s * h) ~ 0.014 per
// substep, applying every edge at once from the substep's velocities (instead of in sequence)
// differs only at second order and conserves momentum exactly, so it runs in one parallel pass.
@compute @workgroup_size(256)
fn finish(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.vertices) { return; }
    let h = substep_h();
    let keep = 1.0 - exp(-params.edge_damping * h);
    let p = pos[i];
    let motion = p.xyz - prev[i].xyz;
    let radius = layers[vlayer[i]].radius;
    if (length(motion) > 2.0 * radius) { atomicAdd(&counters.substep_motion_exceeded, 1u); }
    if (length(p.xyz - built[i].xyz) > 0.5 * params.margin) { atomicAdd(&counters.margin_exceeded, 1u); }
    if (any(p.xyz != p.xyz) || any(abs(p.xyz) > vec3<f32>(1e4))) { atomicAdd(&counters.nonfinite, 1u); }
    let v = motion / h;
    var damped = v;
    let wi = p.w;
    for (var k = adjacency[i]; k < adjacency[i + 1u]; k++) {
        let j = neighbours[k];
        let wj = pos[j].w;
        if (wi + wj <= 0.0) { continue; }
        let vj = (pos[j].xyz - prev[j].xyz) / h;
        damped -= (v - vj) * (keep * wi / (wi + wj));
    }
    vel[i] = vec4<f32>(damped, 0.0);
}
