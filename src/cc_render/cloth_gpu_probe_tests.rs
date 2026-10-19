//! Feasibility probe for a GPU cloth frame (no solver code): how long does one 60 Hz frame of
//! `substeps x stages` dependent passes over ~3.2k particles take on this machine, when issued
//! (a) as one single-workgroup dispatch that loops over all stages with storage barriers, and
//! (b) as one dispatch per stage? Run: `cargo test --release --lib gpu_frame_structure_probe
//! -- --ignored --nocapture` (CC_GPU_PROBE_ADAPTER=<index>, default the last discrete GPU).
use wgpu_pin as wgpu;

const PARTICLES: u32 = 3249;
const SUBSTEPS: u32 = 24;
const STAGES: u32 = 40;

fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapters: Vec<_> = instance
        .enumerate_adapters(wgpu::Backends::VULKAN)
        .into_iter()
        .filter(|a| a.get_info().device_type == wgpu::DeviceType::DiscreteGpu)
        .collect();
    let index = std::env::var("CC_GPU_PROBE_ADAPTER")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(adapters.len().saturating_sub(1));
    let adapter = adapters.into_iter().nth(index)?;
    let info = format!("{:?}", adapter.get_info());
    let limits = adapter.limits();
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::TIMESTAMP_QUERY,
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .ok()?;
    Some((device, queue, info))
}

const FUSED: &str = r#"
@group(0) @binding(0) var<storage, read_write> x: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> y: array<vec4<f32>>;
const N: u32 = 3249u;
@compute @workgroup_size(1024)
fn main(@builtin(local_invocation_index) lid: u32) {
    for (var s = 0u; s < 24u; s++) {
        for (var k = 0u; k < 40u; k++) {
            for (var i = lid; i < N; i += 1024u) {
                let j = (i * 7u + k) % N;
                x[i] = x[i] * 0.999 + y[j] * 0.001 + vec4<f32>(0.0, -1e-6, 0.0, 0.0);
            }
            storageBarrier();
            workgroupBarrier();
        }
    }
}
"#;

const STAGE: &str = r#"
@group(0) @binding(0) var<storage, read_write> x: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> y: array<vec4<f32>>;
const N: u32 = 3249u;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= N) { return; }
    let j = (i * 7u) % N;
    x[i] = x[i] * 0.999 + y[j] * 0.001 + vec4<f32>(0.0, -1e-6, 0.0, 0.0);
}
"#;

#[test]
#[ignore = "GPU probe: run in release with --nocapture"]
fn gpu_frame_structure_probe() {
    let Some((device, queue, info)) = device() else {
        eprintln!("GPU_PROBE no discrete Vulkan adapter; skipped");
        return;
    };
    println!("GPU_PROBE adapter {info}");
    println!("GPU_PROBE limits workgroup_storage {} push_constants {} storage_buffers {}",
        device.limits().max_compute_workgroup_storage_size, device.limits().max_push_constant_size,
        device.limits().max_storage_buffers_per_shader_stage);
    let bytes = (PARTICLES as u64) * 16;
    let buffer = |label| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let (x, y) = (buffer("x"), buffer("y"));
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipeline = |source: &str| {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let (fused, stage) = (pipeline(FUSED), pipeline(STAGE));
    let bind = |p: &wgpu::ComputePipeline| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &p.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: x.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: y.as_entire_binding() },
            ],
        })
    };
    let (fused_group, stage_group) = (bind(&fused), bind(&stage));
    // One frame: encode, submit, copy the positions back and wait for the map (what a live
    // frame needs before publishing the surface).
    let frame = |single: bool| {
        let started = std::time::Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            if single {
                pass.set_pipeline(&fused);
                pass.set_bind_group(0, &fused_group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            } else {
                pass.set_pipeline(&stage);
                pass.set_bind_group(0, &stage_group, &[]);
                for _ in 0..SUBSTEPS * STAGES {
                    pass.dispatch_workgroups(PARTICLES.div_ceil(256), 1, 1);
                }
            }
        }
        encoder.copy_buffer_to_buffer(&x, 0, &readback, 0, bytes);
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::Maintain::Wait);
        let sum: f32 = readback.slice(..).get_mapped_range().chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).sum();
        readback.unmap();
        (started.elapsed().as_secs_f64() * 1e3, sum)
    };
    for (name, single) in [("single_workgroup_fused", true), ("dispatch_per_stage", false)] {
        let _ = frame(single);
        let mut ms: Vec<f64> = (0..50).map(|_| frame(single).0).collect();
        ms.sort_by(f64::total_cmp);
        println!(
            "GPU_PROBE {name}: {SUBSTEPS} substeps x {STAGES} dependent stages over {PARTICLES} particles, frame incl. readback ms: median {:.3} p90 {:.3} max {:.3}",
            ms[25], ms[45], ms[49]
        );
    }
}
