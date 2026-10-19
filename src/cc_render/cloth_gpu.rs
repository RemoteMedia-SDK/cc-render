//! Real-time cloth batch on the GPU (wgpu compute, f32): the live solver when an adapter exists.
//!
//! The CPU batch (`ClothBatch`) stays the reference. This batch keeps its material model and its
//! step contract: layers advance together by explicit simulation time, grips commit with the
//! cloth, and a failed frame publishes nothing. What changes is how contact is found and solved:
//! one dispatch per frame runs every substep on a single 1024-thread workgroup
//! (`cloth_gpu.wgsl`), with proximity vertex-triangle contact from a spatial hash instead of the
//! CPU's certified swept contact. Motion that could outrun that contact set is counted per frame
//! (`GpuReport`), not clamped.
//!
//! Colliders are fixed rounded boxes (the layers' `FixedClothCollider`s) and kinematic spheres
//! and capsules whose start and end poses are supplied per frame; contact uses the pose
//! interpolated to each substep, with friction relative to the collider surface's motion.
use super::batch::Rest;
use super::*;
use std::ops::Range;
use std::sync::OnceLock;
use wgpu_pin as wgpu;

/// A per-frame kinematic body collider (for example an avatar limb). Never moved by the cloth.
#[derive(Clone, Copy, Debug)]
pub(crate) enum KinematicShape {
    Sphere { radius: f64 },
    /// Segment along the local y axis from -half_height to +half_height, swept by `radius`.
    Capsule { half_height: f64, radius: f64 },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct KinematicCollider {
    pub shape: KinematicShape,
    /// Pose at the start and at the end of the frame being stepped.
    pub start: (DVec3, DQuat),
    pub end: (DVec3, DQuat),
    pub friction: f64,
}

/// What one GPU frame did, read back with the positions.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub(crate) struct GpuReport {
    /// Wall time of the frame on the host: upload, dispatch, readback, commit.
    pub frame_ms: f64,
    pub candidates: u32,
    pub contacts: u32,
    /// Tunnelling flags: particle-substeps that moved more than twice their contact radius, and
    /// that moved more than half the candidate margin since the last candidate rebuild.
    pub substep_motion_exceeded: u32,
    pub margin_exceeded: u32,
    /// Contacts with no trustworthy side (vertex in a face's plane or on it at a substep
    /// start); any fails the frame.
    pub ambiguous: u32,
    pub edge_candidates: u32,
    pub colliders: u32,
    pub resting: bool,
}

/// Solver resolution; defaults follow the CPU reference where it has an equivalent.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GpuOptions {
    pub passes: u32,
    pub contact_iterations: u32,
    pub rebuild_every: u32,
    /// Candidate margin beyond the contact gap, metres.
    pub margin: f32,
    /// Hash cell size, metres.
    pub cell: f32,
    /// Over-relaxation of the averaged contact corrections.
    pub relaxation: f32,
    /// Diagnostics only: no face-collider contact.
    pub skip_faces: bool,
    /// Diagnostics only: no edge-edge contact.
    pub skip_edges: bool,
    /// Contact passes per substep that include the edge-edge candidates.
    pub edge_passes: u32,
    /// Substeps between edge-edge candidate rebuilds (a multiple of `rebuild_every`).
    pub edge_rebuild_every: u32,
}

impl Default for GpuOptions {
    fn default() -> Self {
        Self {
            passes: CONSTRAINT_PASSES as u32,
            contact_iterations: 3,
            rebuild_every: 4,
            margin: 0.01,
            cell: 0.06,
            relaxation: 1.0,
            skip_faces: false,
            skip_edges: false,
            edge_passes: 1,
            edge_rebuild_every: 12,
        }
        .with_test_overrides()
    }
}

impl GpuOptions {
    fn with_test_overrides(self) -> Self {
        #[cfg(test)]
        {
            let get = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
            let mut o = self;
            if let Some(v) = get("CC_GPU_PASSES") { o.passes = v as u32; }
            if let Some(v) = get("CC_GPU_CONTACT_ITERATIONS") { o.contact_iterations = v as u32; }
            if let Some(v) = get("CC_GPU_RELAXATION") { o.relaxation = v; }
            if let Some(v) = get("CC_GPU_REBUILD_EVERY") { o.rebuild_every = v as u32; }
            if let Some(v) = get("CC_GPU_SKIP_FACES") { o.skip_faces = v > 0.; }
            if let Some(v) = get("CC_GPU_SKIP_EDGES") { o.skip_edges = v > 0.; }
            if let Some(v) = get("CC_GPU_EDGE_PASSES") { o.edge_passes = v as u32; }
            if let Some(v) = get("CC_GPU_EDGE_REBUILD_EVERY") { o.edge_rebuild_every = v as u32; }
            return o;
        }
        #[allow(unreachable_code)]
        self
    }
}

const CANDIDATE_CAPACITY: u32 = 1 << 20;
const TABLE: u64 = 16384;
const MAX_COLLIDERS: usize = 256;
/// Cell insertions per edge the edge hash can hold on average.
const EDGE_SLOTS_PER_EDGE: u64 = 48;
const PARAMS_BYTES: u64 = 96;
const COUNTER_WORDS: usize = 16;
const COLLIDER_BYTES: usize = 128;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: Pipelines,
    info: String,
}

/// One compute pipeline per shader stage (cloth_gpu.wgsl), all on one layout.
struct Pipelines {
    materials: wgpu::ComputePipeline,
    hash_clear: wgpu::ComputePipeline,
    hash_count: wgpu::ComputePipeline,
    hash_scan: wgpu::ComputePipeline,
    hash_scatter: wgpu::ComputePipeline,
    find_candidates: wgpu::ComputePipeline,
    collide_vertices: wgpu::ComputePipeline,
    collide_faces: wgpu::ComputePipeline,
    apply_acc: wgpu::ComputePipeline,
    contact_pairs: wgpu::ComputePipeline,
    finish: wgpu::ComputePipeline,
    edge_clear: wgpu::ComputePipeline,
    edge_count: wgpu::ComputePipeline,
    edge_scan: wgpu::ComputePipeline,
    edge_scatter: wgpu::ComputePipeline,
    edge_candidates: wgpu::ComputePipeline,
}

const BINDINGS: u32 = 28;
/// Particles the one-workgroup stages hold in workgroup memory (cloth_gpu.wgsl `wx`).
const MAX_VERTICES: usize = 4096;

/// The shared compute device: `CC_CLOTH_GPU=0` disables it; `CC_CLOTH_GPU_ADAPTER=<n>` picks
/// the nth discrete Vulkan adapter (default the last one, which is the otherwise idle GPU 1 on
/// the development machine).
fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| {
        if std::env::var("CC_CLOTH_GPU").as_deref() == Ok("0") {
            return None;
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapters: Vec<_> = instance
            .enumerate_adapters(wgpu::Backends::VULKAN)
            .into_iter()
            .filter(|a| a.get_info().device_type == wgpu::DeviceType::DiscreteGpu)
            .collect();
        let index = std::env::var("CC_CLOTH_GPU_ADAPTER")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(adapters.len().checked_sub(1)?);
        let adapter = adapters.into_iter().nth(index)?;
        let limits = adapter.limits();
        if limits.max_compute_invocations_per_workgroup < 1024
            || limits.max_storage_buffers_per_shader_stage < BINDINGS
            || limits.max_compute_workgroup_storage_size < 49152
            || limits.max_push_constant_size < 16
            || !adapter.features().contains(wgpu::Features::PUSH_CONSTANTS)
        {
            return None;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("cloth"),
                required_features: wgpu::Features::PUSH_CONSTANTS,
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cloth_gpu"),
            source: wgpu::ShaderSource::Wgsl(include_str!("cloth_gpu.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cloth"),
            entries: &(0..BINDINGS)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 0 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage { read_only: false }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cloth"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[wgpu::PushConstantRange {
                stages: wgpu::ShaderStages::COMPUTE,
                range: 0..20,
            }],
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipelines = Pipelines {
            materials: pipeline("materials"),
            hash_clear: pipeline("hash_clear"),
            hash_count: pipeline("hash_count"),
            hash_scan: pipeline("hash_scan"),
            hash_scatter: pipeline("hash_scatter"),
            find_candidates: pipeline("find_candidates"),
            collide_vertices: pipeline("collide_vertices"),
            collide_faces: pipeline("collide_faces"),
            apply_acc: pipeline("apply_acc"),
            contact_pairs: pipeline("contact_pairs"),
            finish: pipeline("finish"),
            edge_clear: pipeline("edge_clear"),
            edge_count: pipeline("edge_count"),
            edge_scan: pipeline("edge_scan"),
            edge_scatter: pipeline("edge_scatter"),
            edge_candidates: pipeline("edge_candidates"),
        };
        Some(Gpu {
            device,
            queue,
            layout,
            pipelines,
            info: format!("{:?}", adapter.get_info()),
        })
    })
    .as_ref()
}

/// Adapter description of the live GPU solver, or None when the CPU reference is live.
pub(crate) fn gpu_adapter() -> Option<&'static str> {
    gpu().map(|g| g.info.as_str())
}

fn bytes_f32(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn bytes_u32(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Greedy graph colouring: no two springs of one colour share a particle, so a colour's springs
/// project in parallel exactly as sequential Gauss-Seidel would.
fn colour_springs(springs: &[(usize, &DistanceSpring)], vertices: usize) -> (Vec<usize>, Vec<u32>) {
    let mut used: Vec<u64> = vec![0; vertices];
    let mut colour_of = Vec::with_capacity(springs.len());
    let mut colours = 0;
    for (offset, s) in springs {
        let [a, b] = s.vertices.map(|v| v + offset);
        let taken = used[a] | used[b];
        let colour = (!taken).trailing_zeros() as usize;
        assert!(colour < 64, "spring graph needs more than 64 colours");
        used[a] |= 1 << colour;
        used[b] |= 1 << colour;
        colours = colours.max(colour + 1);
        colour_of.push(colour);
    }
    let mut order: Vec<usize> = (0..springs.len()).collect();
    order.sort_by_key(|&k| (colour_of[k], k));
    let mut starts = vec![0u32; colours + 1];
    for &k in &order {
        starts[colour_of[k] + 1] += 1;
    }
    for c in 0..colours {
        starts[c + 1] += starts[c];
    }
    (order, starts)
}

struct Buffers {
    params: wgpu::Buffer,
    pos: wgpu::Buffer,
    vel: wgpu::Buffer,
    colliders: wgpu::Buffer,
    grips: wgpu::Buffer,
    anchors: wgpu::Buffer,
    counters: wgpu::Buffer,
    readback: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub(crate) struct GpuClothBatch {
    layout: Vec<Range<usize>>,
    triangles: Vec<Vec<[usize; 3]>>,
    /// Particle coordinates on the GPU are f32 offsets from this point (the batch's initial
    /// centre), keeping sub-micrometre resolution anywhere in the room.
    origin: DVec3,
    vertices: usize,
    colours: u32,
    springs: u32,
    edges: u32,
    triangle_count: u32,
    buffers: Buffers,
    /// GPU particle state no longer matches the committed CPU state (first frame, failed frame,
    /// or a layer surface published outside this batch).
    dirty: bool,
    published: Vec<u64>,
    rest: Rest,
    pub(crate) options: GpuOptions,
    frame_dt: f64,
    pub(crate) last: GpuReport,
}

impl GpuClothBatch {
    /// None when no suitable GPU is available: the caller keeps the CPU reference.
    pub(crate) fn new(solvers: &[&ClothSolver]) -> Result<Option<Self>, ClothError> {
        let Some(gpu) = gpu() else {
            return Ok(None);
        };
        if solvers.is_empty() || solvers.len() > 32 {
            return Err(ClothError::InvalidInput);
        }
        if solvers.iter().map(|s| s.positions.len()).sum::<usize>() > MAX_VERTICES {
            // Larger batches need the global-memory material stages: not built yet, the CPU
            // reference stays live for them.
            return Ok(None);
        }
        let mut layout = Vec::new();
        let mut offset = 0;
        for s in solvers {
            layout.push(offset..offset + s.positions.len());
            offset += s.positions.len();
        }
        let vertices = offset;
        let (lo, hi) = solvers
            .iter()
            .flat_map(|s| s.positions.iter())
            .fold((DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)), |(lo, hi), p| {
                (lo.min(*p), hi.max(*p))
            });
        let origin = ((lo + hi) * 0.5).round();
        let device = &gpu.device;
        let storage = |label: &str, contents: &[u8], extra: wgpu::BufferUsages| {
            let size = contents.len().max(16) as u64;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size.next_multiple_of(16),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | extra,
                mapped_at_creation: false,
            });
            if !contents.is_empty() {
                gpu.queue.write_buffer(&buffer, 0, contents);
            }
            buffer
        };
        let zeroed = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size.max(16).next_multiple_of(16),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let vlayer: Vec<u32> = layout
            .iter()
            .enumerate()
            .flat_map(|(l, r)| std::iter::repeat(l as u32).take(r.len()))
            .collect();
        let layers: Vec<f32> = solvers
            .iter()
            .flat_map(|s| {
                let p = s.parameters;
                [
                    p.thickness_m * 0.5,
                    p.cloth_static_friction as f32,
                    p.cloth_kinetic_friction as f32,
                    p.air_drag_per_second as f32,
                ]
            })
            .collect();
        let all: Vec<(usize, &DistanceSpring)> = solvers
            .iter()
            .zip(&layout)
            .flat_map(|(s, r)| s.springs.iter().map(move |sp| (r.start, sp)))
            .collect();
        let (order, starts) = colour_springs(&all, vertices);
        let mut spring_bytes = Vec::with_capacity(order.len() * 32);
        for &k in &order {
            let (offset, s) = all[k];
            spring_bytes.extend(bytes_u32(&[(s.vertices[0] + offset) as u32, (s.vertices[1] + offset) as u32]));
            spring_bytes.extend(bytes_f32(&[s.rest as f32, s.compliance as f32, s.damping_ratio as f32]));
            let solver = solvers[layout.iter().position(|r| r.start == offset).unwrap()];
            spring_bytes.extend(bytes_u32(&[s.stretch as u32]));
            spring_bytes.extend(bytes_f32(&[
                solver.inverse_mass[s.vertices[0]] as f32,
                solver.inverse_mass[s.vertices[1]] as f32,
            ]));
        }
        let tris: Vec<u32> = solvers
            .iter()
            .zip(&layout)
            .enumerate()
            .flat_map(|(l, (s, r))| {
                s.triangles.iter().flat_map(move |t| {
                    [(t[0] + r.start) as u32, (t[1] + r.start) as u32, (t[2] + r.start) as u32, l as u32]
                })
            })
            .collect();
        let triangle_count = (tris.len() / 4) as u32;
        // Unique material edges of every layer, with the layer.
        let mut edge_list = Vec::new();
        for (l, (s, r)) in solvers.iter().zip(&layout).enumerate() {
            let mut seen = std::collections::BTreeSet::new();
            for t in &s.triangles {
                for k in 0..3 {
                    let (a, b) = (t[k] + r.start, t[(k + 1) % 3] + r.start);
                    seen.insert((a.min(b), a.max(b)));
                }
            }
            edge_list.extend(seen.into_iter().flat_map(|(a, b)| [a as u32, b as u32, l as u32, 0]));
        }
        let edge_count = (edge_list.len() / 4) as u32;
        // Stretch-edge neighbours per particle (CSR) for the edge dashpots.
        let mut neighbour_lists = vec![Vec::new(); vertices];
        for (offset, sp) in &all {
            if sp.stretch {
                let [a, b] = sp.vertices.map(|v| v + offset);
                neighbour_lists[a].push(b as u32);
                neighbour_lists[b].push(a as u32);
            }
        }
        let mut adjacency = vec![0u32];
        for list in &neighbour_lists {
            adjacency.push(adjacency.last().unwrap() + list.len() as u32);
        }
        let neighbour_ids: Vec<u32> = neighbour_lists.concat();
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: PARAMS_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pos = zeroed("pos", vertices as u64 * 16);
        let prev = zeroed("prev", vertices as u64 * 16);
        let vel = zeroed("vel", vertices as u64 * 16);
        let vlayer = storage("vlayer", &bytes_u32(&vlayer), wgpu::BufferUsages::empty());
        let layers = storage("layers", &bytes_f32(&layers), wgpu::BufferUsages::empty());
        let springs = storage("springs", &spring_bytes, wgpu::BufferUsages::empty());
        let colour_start = storage("colours", &bytes_u32(&starts), wgpu::BufferUsages::empty());
        let lambda = zeroed("lambda", order.len() as u64 * 4);
        let tri_buffer = storage("tris", &bytes_u32(&tris), wgpu::BufferUsages::empty());
        let cell_count = zeroed("cell_count", TABLE * 4);
        let cell_start = zeroed("cell_start", TABLE * 4);
        let sorted = zeroed("sorted", vertices as u64 * 4);
        let cand = zeroed("cand", CANDIDATE_CAPACITY as u64 * 8);
        let acc = zeroed("acc", vertices as u64 * 16);
        let colliders = zeroed("colliders", (MAX_COLLIDERS * COLLIDER_BYTES) as u64);
        let grips = zeroed("grips", 64 * 64);
        let anchors = zeroed("anchors", 64 * 257 * 16);
        let counters = zeroed("counters", COUNTER_WORDS as u64 * 4);
        let built = zeroed("built", vertices as u64 * 16);
        let support = zeroed("support", vertices as u64 * 16);
        let edges = storage("edges", &bytes_u32(&edge_list), wgpu::BufferUsages::empty());
        let ecount = zeroed("ecount", TABLE * 4);
        let estart = zeroed("estart", TABLE * 4);
        let eslots = zeroed("eslots", edge_count as u64 * EDGE_SLOTS_PER_EDGE * 4);
        let ecand = zeroed("ecand", CANDIDATE_CAPACITY as u64 * 8);
        let adjacency = storage("adjacency", &bytes_u32(&adjacency), wgpu::BufferUsages::empty());
        let neighbours = storage("neighbours", &bytes_u32(&neighbour_ids), wgpu::BufferUsages::empty());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: vertices as u64 * 32 + 64 * 64 + COUNTER_WORDS as u64 * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entries: Vec<&wgpu::Buffer> = vec![
            &params, &pos, &prev, &vel, &vlayer, &layers, &springs, &colour_start, &lambda,
            &tri_buffer, &cell_count, &cell_start, &sorted, &cand, &acc, &colliders, &grips,
            &anchors, &counters, &built, &support, &edges, &ecount, &estart, &eslots, &ecand,
            &adjacency, &neighbours,
        ];
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cloth"),
            layout: &gpu.layout,
            entries: &entries
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        });
        Ok(Some(Self {
            layout,
            triangles: solvers.iter().map(|s| s.triangles.clone()).collect(),
            origin,
            vertices,
            colours: (starts.len() - 1) as u32,
            springs: order.len() as u32,
            edges: edge_count,
            triangle_count,
            buffers: Buffers {
                params,
                pos,
                vel,
                colliders,
                grips,
                anchors,
                counters,
                readback,
                bind_group,
            },
            dirty: true,
            published: Vec::new(),
            rest: Rest::new(),
            options: GpuOptions::default(),
            frame_dt: 1. / 60.,
            last: GpuReport::default(),
        }))
    }

    /// Particles, springs, spring colours, triangles and edges of the batch.
    pub(crate) fn layout_summary(&self) -> String {
        format!(
            "particles {} springs {} colours {} triangles {} edges {}",
            self.vertices, self.springs, self.colours, self.triangle_count, self.edges
        )
    }

    pub(crate) fn start_at_rest(&mut self) {
        self.rest.adopt_on_first_tick = true;
    }

    pub(crate) fn resting(&self) -> bool {
        self.rest.asleep
    }

    pub(crate) fn step(
        &mut self,
        layers: &mut [ClothLayerStep<'_>],
        dt: f64,
        gravity: DVec3,
    ) -> Result<(), ClothError> {
        self.step_with_kinematic(layers, dt, gravity, &[])
    }

    fn validate(&self, layers: &[ClothLayerStep<'_>], dt: f64, gravity: DVec3) -> Result<(), ClothError> {
        if !dt.is_finite()
            || !(0.0..=1. / 30.).contains(&dt)
            || dt == 0.
            || !gravity.is_finite()
            || layers.len() != self.layout.len()
            || layers.iter().zip(&self.layout).zip(&self.triangles).any(|((l, r), t)| {
                l.solver.positions.len() != r.len() || l.solver.triangles != *t
            })
            || layers.iter().map(|l| l.grips.len()).sum::<usize>() > 64
        {
            return Err(ClothError::InvalidInput);
        }
        Ok(())
    }

    /// One frame. Kinematic colliders (body parts) touch every layer and keep the batch awake.
    pub(crate) fn step_with_kinematic(
        &mut self,
        layers: &mut [ClothLayerStep<'_>],
        dt: f64,
        gravity: DVec3,
        kinematic: &[KinematicCollider],
    ) -> Result<(), ClothError> {
        self.validate(layers, dt, gravity)?;
        if kinematic.iter().any(|k| {
            ![k.start.0, k.end.0].iter().all(|p| p.is_finite())
                || ![k.start.1, k.end.1].iter().all(|q| q.is_finite())
                || !k.friction.is_finite()
                || k.friction < 0.
                || match k.shape {
                    KinematicShape::Sphere { radius } => !(radius > 0.),
                    KinematicShape::Capsule { half_height, radius } => !(radius > 0. && half_height >= 0.),
                }
        }) {
            return Err(ClothError::InvalidInput);
        }
        let started = std::time::Instant::now();
        let inputs = Rest::inputs(layers, gravity);
        if std::mem::take(&mut self.rest.adopt_on_first_tick) {
            for layer in layers.iter_mut() {
                layer.solver.velocities.fill(DVec3::ZERO);
            }
            self.rest.asleep = true;
            self.rest.inputs = inputs;
            self.rest.revisions = layers.iter().map(|l| l.solver.surface.revision()).collect();
        }
        if kinematic.is_empty() && self.rest.holds(layers, inputs) {
            self.last = GpuReport {
                frame_ms: started.elapsed().as_secs_f64() * 1e3,
                resting: true,
                ..Default::default()
            };
            return Ok(());
        }
        self.rest.wake();
        let gpu = gpu().expect("GPU batch exists only with a device");
        let revisions: Vec<u64> = layers.iter().map(|l| l.solver.surface.revision()).collect();
        if self.dirty || revisions != self.published {
            self.upload_state(gpu, layers);
        }
        let substeps = layers.iter().map(|l| l.solver.parameters.substeps).max().unwrap() as u32;
        let tick_start: Vec<DVec3> = layers.iter().flat_map(|l| l.solver.positions.iter().copied()).collect();
        // Grips: material state at the frame start and their anchors at every substep boundary.
        let mut grip_bytes = Vec::new();
        let mut anchor_bytes = Vec::new();
        for (layer, range) in layers.iter_mut().zip(&self.layout) {
            for grip in layer.grips.iter_mut() {
                grip.prepare(&layer.solver.surface)?;
                let (ids, weights, material) = grip.gpu_view();
                let state = match grip.state() {
                    super::super::cloth_grip::ClampState::Held => 0,
                    super::super::cloth_grip::ClampState::Released => 1,
                    super::super::cloth_grip::ClampState::Overloaded => 2,
                };
                grip_bytes.extend(bytes_u32(&ids.map(|i| (i + range.start) as u32)));
                grip_bytes.extend(bytes_u32(&[state]));
                grip_bytes.extend(bytes_f32(&weights.map(|w| w as f32)));
                grip_bytes.extend(bytes_f32(&[0.]));
                grip_bytes.extend(bytes_f32(&[
                    material.compliance_m_n as f32,
                    material.damping_ratio as f32,
                    material.capacity_n as f32,
                    0.,
                ]));
                grip_bytes.extend(bytes_f32(&[0.; 4]));
                for s in 0..=substeps {
                    let a = grip.anchor(s as f64 / substeps as f64) - self.origin;
                    anchor_bytes.extend(bytes_f32(&[a.x as f32, a.y as f32, a.z as f32, 0.]));
                }
            }
        }
        let grip_count = (grip_bytes.len() / 64) as u32;
        if grip_count > 0 && (substeps as usize + 1) * grip_count as usize > 64 * 257 {
            return Err(ClothError::InvalidInput);
        }
        self.frame_dt = dt;
        let collider_bytes = self.colliders(layers, kinematic)?;
        let o = self.options;
        let mut params = bytes_f32(&[gravity.x as f32, gravity.y as f32, gravity.z as f32, 0., dt as f32]);
        params.extend(bytes_u32(&[
            substeps,
            self.vertices as u32,
            self.colours,
            self.triangle_count,
            (collider_bytes.len() / COLLIDER_BYTES) as u32,
            grip_count,
            o.passes,
            o.contact_iterations,
            o.rebuild_every.max(1),
        ]));
        params.extend(bytes_f32(&[o.cell, o.margin]));
        params.extend(bytes_u32(&[CANDIDATE_CAPACITY]));
        params.extend(bytes_f32(&[EDGE_DAMPING_PER_SECOND as f32]));
        params.extend(bytes_u32(&[self.springs, o.relaxation.to_bits()]));
        params.extend(bytes_u32(&[self.edges, (self.edges as u64 * EDGE_SLOTS_PER_EDGE) as u32, 0, 0]));
        let b = &self.buffers;
        let queue = &gpu.queue;
        queue.write_buffer(&b.params, 0, &params);
        if !collider_bytes.is_empty() {
            queue.write_buffer(&b.colliders, 0, &collider_bytes);
        }
        if !grip_bytes.is_empty() {
            queue.write_buffer(&b.grips, 0, &grip_bytes);
            queue.write_buffer(&b.anchors, 0, &anchor_bytes);
        }
        queue.write_buffer(&b.counters, 0, &[0u8; COUNTER_WORDS * 4]);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let p = &gpu.pipelines;
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_bind_group(0, &b.bind_group, &[]);
            let groups = |n: usize| (n as u32).div_ceil(256).max(1);
            let (vg, tg) = (groups(self.vertices), groups(self.triangle_count as usize));
            for substep in 0..substeps {
                let mut continue_edges = true;
                let push = |pass: &mut wgpu::ComputePass<'_>, relax: f32, edges_on: bool| {
                    let mut bytes = substep.to_le_bytes().to_vec();
                    bytes.extend(bytes_f32(&[substep as f32 / substeps as f32, (substep + 1) as f32 / substeps as f32, relax]));
                    bytes.extend((edges_on as u32).to_le_bytes());
                    pass.set_push_constants(0, &bytes);
                };
                let mut run_with = |pass: &mut wgpu::ComputePass<'_>, pipeline: &wgpu::ComputePipeline, groups: u32, relax: f32, edges_on: bool| {
                    pass.set_pipeline(pipeline);
                    push(pass, relax, edges_on);
                    pass.dispatch_workgroups(groups, 1, 1);
                };
                let mut run = |pass: &mut wgpu::ComputePass<'_>, pipeline: &wgpu::ComputePipeline, groups: u32, relax: f32| {
                    run_with(pass, pipeline, groups, relax, false);
                };
                run(&mut pass, &p.materials, 1, 1.);
                if substep % o.rebuild_every.max(1) == 0 {
                    run(&mut pass, &p.hash_clear, (TABLE as u32).div_ceil(256), 1.);
                    run(&mut pass, &p.hash_count, vg, 1.);
                    run(&mut pass, &p.hash_scan, 1, 1.);
                    run(&mut pass, &p.hash_scatter, vg, 1.);
                    run(&mut pass, &p.find_candidates, tg, 1.);
                    let eg = groups(self.edges as usize);
                    if o.skip_edges || substep % o.edge_rebuild_every.max(1) != 0 { continue_edges = false; }
                    if continue_edges {
                    run(&mut pass, &p.edge_clear, (TABLE as u32).div_ceil(256), 1.);
                    run(&mut pass, &p.edge_count, eg, 1.);
                    run(&mut pass, &p.edge_scan, 1, 1.);
                    run(&mut pass, &p.edge_scatter, eg, 1.);
                    run(&mut pass, &p.edge_candidates, eg, 1.);
                    }
                }
                run(&mut pass, &p.collide_vertices, vg, 1.);
                for iteration in 0..o.contact_iterations {
                    // Edge-edge pairs once per substep (first pass): undoing a crossing needs one
                    // pass; at rest they mostly duplicate the vertex-face contacts of stacked
                    // sheets and cost as much again.
                    let edges_on = iteration < o.edge_passes.min(o.contact_iterations);
                    run_with(&mut pass, &p.contact_pairs, 128, o.relaxation, edges_on);
                    run(&mut pass, &p.apply_acc, vg, o.relaxation);
                }
                // Face samples once per substep, after the contact passes: repeating them in
                // every averaged pass (each with its own static friction) kept the bedding
                // pumping energy at the mattress's rounded corners (published motion 0.1-0.9 m/s
                // after 7 s); once per substep it rests by tick ~170, as the CPU reference does.
                if !o.skip_faces {
                    run(&mut pass, &p.collide_faces, tg, 1.);
                    run(&mut pass, &p.apply_acc, vg, 1.);
                }
                run(&mut pass, &p.finish, vg, 1.);
            }
        }
        let v = self.vertices as u64;
        encoder.copy_buffer_to_buffer(&b.pos, 0, &b.readback, 0, v * 16);
        encoder.copy_buffer_to_buffer(&b.vel, 0, &b.readback, v * 16, v * 16);
        let words = COUNTER_WORDS as u64 * 4;
        encoder.copy_buffer_to_buffer(&b.counters, 0, &b.readback, v * 32, words);
        if grip_count > 0 {
            encoder.copy_buffer_to_buffer(&b.grips, 0, &b.readback, v * 32 + words, grip_count as u64 * 64);
        }
        queue.submit([encoder.finish()]);
        let slice = b.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::Maintain::Wait);
        let data: Vec<f32> = slice
            .get_mapped_range()
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        b.readback.unmap();
        let word = |i: usize| data[i].to_bits();
        let base = self.vertices * 8;
        let counters: [u32; 10] = std::array::from_fn(|k| word(base + k));
        let [candidates, overflow, substep_motion, margin_motion, contacts, nonfinite, ambiguous, witness, edge_candidates, _] = counters;
        #[cfg(test)]
        if ambiguous > 0 {
            println!("GPU_AMBIGUOUS_WITNESS vertex {} triangle {}", witness & 0xfff, witness >> 12);
        }
        let _ = witness;
        self.last = GpuReport {
            frame_ms: started.elapsed().as_secs_f64() * 1e3,
            candidates,
            contacts,
            substep_motion_exceeded: substep_motion,
            margin_exceeded: margin_motion,
            ambiguous,
            edge_candidates,
            colliders: (collider_bytes.len() / COLLIDER_BYTES) as u32,
            resting: false,
        };
        // A failed frame publishes nothing and the GPU state is reloaded from the committed CPU
        // state before the next one.
        if nonfinite > 0 {
            self.dirty = true;
            return Err(ClothError::NonfiniteState);
        }
        if overflow > 0 || ambiguous > 0 {
            self.dirty = true;
            return Err(ClothError::SelfContactUnresolved);
        }
        let positions: Vec<DVec3> = (0..self.vertices)
            .map(|i| DVec3::new(data[i * 4] as f64, data[i * 4 + 1] as f64, data[i * 4 + 2] as f64) + self.origin)
            .collect();
        let velocities: Vec<DVec3> = (0..self.vertices)
            .map(|i| {
                let j = base / 2 + i * 4;
                DVec3::new(data[j] as f64, data[j + 1] as f64, data[j + 2] as f64)
            })
            .collect();
        if positions.iter().chain(&velocities).any(|p| !p.is_finite()) {
            self.dirty = true;
            return Err(ClothError::NonfiniteState);
        }
        // Commit: validate every surface first, then publish all layers and grips together.
        let surfaces = layers
            .iter()
            .zip(&self.layout)
            .map(|(l, r)| l.solver.surface.updated(positions[r.clone()].iter().map(|p| p.as_vec3()).collect()))
            .collect::<Result<Vec<_>, _>>();
        let surfaces = match surfaces {
            Ok(s) => s,
            Err(e) => {
                self.dirty = true;
                return Err(e.into());
            }
        };
        let mut grip_index = 0;
        for ((layer, range), surface) in layers.iter_mut().zip(&self.layout).zip(surfaces) {
            layer.solver.positions.copy_from_slice(&positions[range.clone()]);
            layer.solver.velocities.copy_from_slice(&velocities[range.clone()]);
            layer.solver.surface = surface;
            for grip in layer.grips.iter_mut() {
                let g = base + COUNTER_WORDS + grip_index * 16;
                grip.gpu_result(word(g + 3), data[g + 12] as f64, data[g + 13] as f64);
                grip.finish();
                grip_index += 1;
            }
        }
        self.dirty = false;
        self.published = layers.iter().map(|l| l.solver.surface.revision()).collect();
        if kinematic.is_empty() {
            self.rest.observe(layers, &tick_start, dt, inputs);
        }
        self.last.frame_ms = started.elapsed().as_secs_f64() * 1e3;
        self.last.resting = self.rest.asleep;
        Ok(())
    }

    fn upload_state(&mut self, gpu: &Gpu, layers: &[ClothLayerStep<'_>]) {
        let mut pos = Vec::with_capacity(self.vertices * 4);
        let mut vel = Vec::with_capacity(self.vertices * 4);
        for l in layers {
            for ((p, v), w) in l.solver.positions.iter().zip(&l.solver.velocities).zip(&l.solver.inverse_mass) {
                let r = *p - self.origin;
                pos.extend([r.x as f32, r.y as f32, r.z as f32, *w as f32]);
                vel.extend([v.x as f32, v.y as f32, v.z as f32, 0.]);
            }
        }
        gpu.queue.write_buffer(&self.buffers.pos, 0, &bytes_f32(&pos));
        gpu.queue.write_buffer(&self.buffers.vel, 0, &bytes_f32(&vel));
    }

    /// Fixed colliders of every layer (deduplicated, with a layer mask) near the cloth, and the
    /// kinematic colliders for all layers, in the shader's layout.
    fn colliders(
        &self,
        layers: &[ClothLayerStep<'_>],
        kinematic: &[KinematicCollider],
    ) -> Result<Vec<u8>, ClothError> {
        let (lo, hi) = layers
            .iter()
            .flat_map(|l| l.solver.positions.iter())
            .fold((DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)), |(lo, hi), p| {
                (lo.min(*p), hi.max(*p))
            });
        // Anything the cloth can reach this frame: its bounds grown by the fastest particle's
        // motion over the frame plus 5 cm (collider bounds are tested again per particle).
        let speed = layers.iter().flat_map(|l| l.solver.velocities.iter()).map(|v| v.length()).fold(0., f64::max);
        let reach = 0.05 + speed * self.frame_dt.max(1. / 60.) * 2.;
        let mut fixed: Vec<(Vec<u64>, &FixedClothCollider, u32)> = Vec::new();
        for (index, layer) in layers.iter().enumerate() {
            for c in layer.colliders {
                if !c.intersects_bounds(lo, hi, reach) {
                    continue;
                }
                let key: Vec<u64> = c
                    .center
                    .to_array()
                    .into_iter()
                    .chain(c.rotation.to_array())
                    .chain(c.half_extents.to_array())
                    .chain([c.border, c.friction])
                    .map(f64::to_bits)
                    .collect();
                match fixed.iter_mut().find(|(k, _, _)| *k == key) {
                    Some(entry) => entry.2 |= 1 << index,
                    None => fixed.push((key, c, 1 << index)),
                }
            }
        }
        if fixed.len() + kinematic.len() > MAX_COLLIDERS {
            return Err(ClothError::UnsupportedContact);
        }
        let all_layers = (1u64 << layers.len()) as u32 - 1;
        let grow = layers.iter().map(|l| l.solver.parameters.thickness_m as f64 * 0.5).fold(0., f64::max) + 1e-3;
        let mut out = Vec::new();
        let mut push = |p0: DVec3, q0: DQuat, p1: DVec3, q1: DQuat, half: [f64; 4], kind: u32, mask: u32, friction: f64, border: f64| {
            // World bounds over both poses.
            let extent = |q: DQuat| match kind {
                0 => (q * DVec3::X).abs() * half[0] + (q * DVec3::Y).abs() * half[1] + (q * DVec3::Z).abs() * half[2] + DVec3::splat(border),
                1 => DVec3::splat(half[3]),
                _ => (q * DVec3::Y).abs() * half[1] + DVec3::splat(half[3]),
            };
            let lo = (p0 - extent(q0)).min(p1 - extent(q1)) - DVec3::splat(grow) - self.origin;
            let hi = (p0 + extent(q0)).max(p1 + extent(q1)) + DVec3::splat(grow) - self.origin;
            out.extend(bytes_f32(&[lo.x as f32, lo.y as f32, lo.z as f32, 0.]));
            out.extend(bytes_f32(&[hi.x as f32, hi.y as f32, hi.z as f32, 0.]));
            let p0 = p0 - self.origin;
            let p1 = p1 - self.origin;
            out.extend(bytes_f32(&[p0.x as f32, p0.y as f32, p0.z as f32, 0.]));
            out.extend(bytes_f32(&q0.to_array().map(|x| x as f32)));
            out.extend(bytes_f32(&[p1.x as f32, p1.y as f32, p1.z as f32, 0.]));
            out.extend(bytes_f32(&q1.to_array().map(|x| x as f32)));
            out.extend(bytes_f32(&half.map(|x| x as f32)));
            out.extend(bytes_u32(&[kind, mask]));
            out.extend(bytes_f32(&[friction as f32, border as f32]));
        };
        for (_, c, mask) in &fixed {
            let h = c.half_extents;
            push(c.center, c.rotation, c.center, c.rotation, [h.x, h.y, h.z, 0.], 0, *mask, c.friction, c.border);
        }
        for k in kinematic {
            let (half, kind) = match k.shape {
                KinematicShape::Sphere { radius } => ([0., 0., 0., radius], 1),
                KinematicShape::Capsule { half_height, radius } => ([0., half_height, 0., radius], 2),
            };
            push(k.start.0, k.start.1, k.end.0, k.end.1, half, kind, all_layers, k.friction, 0.);
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "cloth_gpu_tests.rs"]
mod tests;
