//! Optional explicit GPU selection for the Bevy renderer.
//!
//! By default Bevy lets wgpu pick the adapter via `WgpuSettings::default()`
//! (high-power preference). On a multi-GPU host that picks one of the
//! discrete adapters non-deterministically — and on WSL2 that's typically
//! the same physical GPU the Windows compositor / Snipping Tool / NVENC
//! are also using, which causes contention.
//!
//! Setting `AVATAR_BEVY_GPU_INDEX=N` enables this module's manual adapter
//! path: enumerate native adapters and pick the Nth, deterministically.
//! Index ordering matches what's logged at startup so users can pick by
//! reading the log once. The rest of the wgpu device init mirrors
//! `bevy_render::initialize_renderer` — features, limits, memory hints —
//! so flipping the env var only changes the physical device, nothing
//! else about renderer behaviour.
//!
//! Why this lives here and not as a free env var:
//! - wgpu's `WGPU_ADAPTER_NAME` is a name-substring match — it can't
//!   disambiguate two GPUs of the same model.
//! - Bevy's `WgpuSettings` exposes `backends` + `power_preference` but
//!   no adapter index — manual is the only deterministic way to pin to
//!   a specific physical device.

use std::sync::Arc;

use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::{
    Backends, RenderCreation, WgpuFeatures, WgpuSettings, WgpuSettingsPriority,
};
use wgpu_pin::{DeviceDescriptor, DeviceType, Instance, InstanceDescriptor};

const GPU_INDEX_ENV: &str = "AVATAR_BEVY_GPU_INDEX";

/// Build a `RenderCreation` to hand to Bevy's `RenderPlugin`.
///
/// Reads `AVATAR_BEVY_GPU_INDEX`. If unset, returns
/// `RenderCreation::Automatic(default_settings)` and Bevy picks the
/// adapter as it always has. If set, enumerates adapters, logs the
/// list, picks the Nth, and returns `RenderCreation::Manual(...)`.
///
/// `default_settings` is what we'd pass to `Automatic`; we copy its
/// instance flags / features / limits / memory hints onto the manual
/// path so a user flipping the env var on/off doesn't accidentally
/// change other renderer behaviour.
pub fn build_render_creation(default_settings: WgpuSettings) -> RenderCreation {
    let Some(idx) = std::env::var(GPU_INDEX_ENV)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    else {
        return RenderCreation::Automatic(default_settings);
    };

    match build_manual(idx, &default_settings) {
        Ok(manual) => manual,
        Err(e) => {
            tracing::warn!(
                target: "cc_render",
                "AVATAR_BEVY_GPU_INDEX={} failed: {}; falling back to default adapter selection",
                idx, e
            );
            RenderCreation::Automatic(default_settings)
        }
    }
}

fn build_manual(idx: usize, settings: &WgpuSettings) -> Result<RenderCreation, String> {
    // Honour `WgpuSettings::backends` if set; otherwise default to
    // `Backends::PRIMARY` (Vulkan + DX12 + Metal). GL / WebGPU don't
    // make sense for headless Bevy on WSL2/Linux/Windows so we don't
    // include them when defaulting.
    let backends = settings.backends.unwrap_or(Backends::PRIMARY);

    let instance = Instance::new(InstanceDescriptor {
        backends,
        flags: settings.instance_flags,
        dx12_shader_compiler: settings.dx12_shader_compiler.clone(),
        gles_minor_version: settings.gles3_minor_version,
    });

    let adapters: Vec<_> = instance.enumerate_adapters(backends);
    let count = adapters.len();
    if count == 0 {
        return Err("no adapters enumerated for requested backends".into());
    }
    tracing::info!(
        target: "cc_render",
        "AVATAR_BEVY_GPU_INDEX={}: {} adapter(s) visible (backends={:?})",
        idx, count, backends
    );
    for (i, a) in adapters.iter().enumerate() {
        let info = a.get_info();
        tracing::info!(
            target: "cc_render",
            "  [{}] {} ({:?}, vendor=0x{:04x}, device=0x{:04x}, backend={:?})",
            i, info.name, info.device_type, info.vendor, info.device, info.backend,
        );
    }

    let adapter = adapters.into_iter().nth(idx).ok_or_else(|| {
        format!(
            "only {} adapter(s) visible, AVATAR_BEVY_GPU_INDEX={} out of range",
            count, idx
        )
    })?;
    let adapter_info = adapter.get_info();
    if adapter_info.device_type == DeviceType::Cpu {
        tracing::warn!(
            target: "cc_render",
            "selected adapter '{}' is software/CPU — performance will be poor",
            adapter_info.name
        );
    }
    tracing::info!(
        target: "cc_render",
        "selected adapter [{}] '{}' ({:?}, backend={:?})",
        idx, adapter_info.name, adapter_info.device_type, adapter_info.backend,
    );

    // Match `bevy_render::renderer::initialize_renderer`'s feature /
    // limit selection so the manual path is observationally identical
    // to Automatic apart from the device choice.
    let mut features = WgpuFeatures::empty();
    let mut limits = settings.limits.clone();
    if matches!(settings.priority, WgpuSettingsPriority::Functionality) {
        features = adapter.features();
        if adapter_info.device_type == DeviceType::DiscreteGpu {
            features -= WgpuFeatures::MAPPABLE_PRIMARY_BUFFERS;
        }
        features -= WgpuFeatures::RAY_QUERY;
        features -= WgpuFeatures::RAY_TRACING_ACCELERATION_STRUCTURE;
        limits = adapter.limits();
    }
    if let Some(disabled) = settings.disabled_features {
        features -= disabled;
    }
    features |= settings.features;

    let (device, queue) = pollster::block_on(adapter.request_device(
        &DeviceDescriptor {
            label: settings.device_label.as_deref(),
            required_features: features,
            required_limits: limits,
            memory_hints: settings.memory_hints.clone(),
        },
        settings.trace_path.as_deref(),
    ))
    .map_err(|e| format!("request_device on '{}' failed: {}", adapter_info.name, e))?;

    // `RenderInstance / RenderAdapter / RenderQueue` are tuple structs
    // with `pub` fields (see bevy_render-0.15.3/src/renderer/mod.rs),
    // so manual construction outside the crate is supported.
    let render_instance = RenderInstance(Arc::new(WgpuWrapper::new(instance)));
    let render_adapter = RenderAdapter(Arc::new(WgpuWrapper::new(adapter)));
    let render_adapter_info = RenderAdapterInfo(WgpuWrapper::new(adapter_info));
    let render_queue = RenderQueue(Arc::new(WgpuWrapper::new(queue)));
    let render_device = RenderDevice::from(device);

    Ok(RenderCreation::manual(
        render_device,
        render_queue,
        render_adapter_info,
        render_adapter,
        render_instance,
    ))
}
