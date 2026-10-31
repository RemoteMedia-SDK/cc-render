//! Render-target → CPU readback (RenderApp side).
//!
//! Wire-format: tightly-packed RGBA8 bytes (`width * height * 4`).
//! wgpu's `bytes_per_row` for `copy_texture_to_buffer` must be a
//! multiple of 256, so we copy into a padded staging buffer, map it,
//! then strip the row padding before pushing onto the frame channel.

use bevy::prelude::*;
use bevy::render::{
    extract_resource::ExtractResource,
    render_asset::RenderAssets,
    render_resource::{
        Buffer, BufferAsyncError, BufferDescriptor, BufferUsages, CommandEncoderDescriptor,
        Extent3d, ImageCopyBuffer, ImageDataLayout, Maintain, MapMode,
    },
    renderer::{RenderDevice, RenderQueue},
    texture::GpuImage,
};
use crossbeam_channel::Sender;

/// wgpu spec: `bytes_per_row` for `copy_texture_to_buffer` must be a
/// multiple of 256.
pub(crate) const COPY_BYTES_PER_ROW_ALIGNMENT: u32 = 256;

#[derive(Resource, Clone, ExtractResource)]
pub(crate) struct CaptureTarget {
    pub image: Handle<Image>,
    pub width: u32,
    pub height: u32,
}

/// Sender side lives on the RenderApp (frame producer); the receiver
/// is held by [`super::super::renderer::CcRenderer`].
#[derive(Resource, Clone)]
pub(crate) struct CaptureChannelTx {
    pub tx: Sender<CapturedFrame>,
}

/// One captured frame including its `pts_ms` (echo of the most-recent
/// applied pose so the streaming-node side can stamp the output
/// `RuntimeData::Video`).
pub(crate) struct CapturedFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pts_ms: u64,
}

/// Holds the most-recent `ArkitPose::pts_ms` applied by the main
/// app's pose system; copied across to the RenderApp via
/// `ExtractResourcePlugin` so the capture system can stamp the frame
/// it produces.
///
/// Stamped onto `CapturedFrame.pts_ms` so downstream consumers
/// (`CcRenderNode::tick`) know the *content time* of the visible
/// mouth pose at this capture. The streaming node uses this on the
/// first emitted V frame to fast-forward the audio buffer by the
/// matching number of samples, keeping the speech audio in sync with
/// the mouth movements (otherwise audio would start at content_time=0
/// while the visible mouth is already at content_time≈warmup_gap_ms).
#[derive(Resource, Clone, Copy, Default, ExtractResource)]
pub(crate) struct LastAppliedPts(pub u64);

/// Monotonic capture-time clock. Retained for diagnostics; the actual
/// frame `pts_ms` is the pose content pts (`LastAppliedPts`) so that
/// `CcRenderNode::tick` can align A/V at the start of the stream.
#[derive(Resource, Default)]
pub(crate) struct MonotonicCaptureClock {
    pub frame_count: u64,
    pub frame_interval_ms: u64,
}

pub(crate) fn copy_render_target_to_cpu(
    target: Option<Res<CaptureTarget>>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    tx: Res<CaptureChannelTx>,
    pts: Option<Res<LastAppliedPts>>,
    mut clock: ResMut<MonotonicCaptureClock>,
    mut count: bevy::ecs::system::Local<u64>,
) {
    *count += 1;
    let Some(t) = target else {
        if *count == 1 || *count % 60 == 0 {
            bevy::log::debug!(
                target: "cc_render",
                "[render-app] tick #{}: no CaptureTarget yet",
                *count
            );
        }
        return;
    };
    let Some(gpu_image) = images.get(&t.image) else {
        if *count == 1 || *count % 60 == 0 {
            bevy::log::debug!(
                target: "cc_render",
                "[render-app] tick #{}: target present but GpuImage not yet uploaded",
                *count
            );
        }
        return;
    };
    if *count == 1 {
        bevy::log::info!(
            target: "cc_render",
            "[render-app] tick #{}: capturing {}×{} (first frame)",
            *count,
            t.width,
            t.height
        );
    } else if *count % 60 == 0 {
        bevy::log::debug!(
            target: "cc_render",
            "[render-app] tick #{}: capturing {}×{}",
            *count,
            t.width,
            t.height
        );
    }

    let unpadded_bpr: u32 = t.width * 4;
    let align = COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bpr = unpadded_bpr.div_ceil(align) * align;
    let buffer_size = (padded_bpr as u64) * (t.height as u64);

    let buffer: Buffer = device.create_buffer(&BufferDescriptor {
        label: Some("cc_render_readback"),
        size: buffer_size,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("cc_render_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        gpu_image.texture.as_image_copy(),
        ImageCopyBuffer {
            buffer: &buffer,
            layout: ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(t.height),
            },
        },
        Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = buffer.slice(..);
    let (cb_tx, cb_rx) = crossbeam_channel::bounded::<Result<(), BufferAsyncError>>(1);
    slice.map_async(MapMode::Read, move |res| {
        let _ = cb_tx.send(res);
    });
    device.poll(Maintain::Wait);
    let _ = cb_rx.recv();

    let view = slice.get_mapped_range();
    let mut tight = Vec::with_capacity((unpadded_bpr * t.height) as usize);
    for row in 0..t.height as usize {
        let start = row * padded_bpr as usize;
        let end = start + unpadded_bpr as usize;
        tight.extend_from_slice(&view[start..end]);
    }
    drop(view);
    buffer.unmap();

    // Stamp the captured frame with the pose's *content* pts. The
    // streaming node uses this on the first emitted V to fast-forward
    // its audio buffer so wav sample 0 aligns with the visible mouth
    // pose's content time (otherwise audio plays from content_time=0
    // while the mouth is already at content_time≈warmup_gap_ms).
    // Monotonic clock is bumped for diagnostics only.
    clock.frame_count += 1;
    let pose_pts_ms = pts.map(|p| p.0).unwrap_or(0);
    let _ = tx.tx.try_send(CapturedFrame {
        pixels: tight,
        width: t.width,
        height: t.height,
        pts_ms: pose_pts_ms,
    });
}

/// Non-blocking readback for the desktop overlay: a few staging buffers,
/// reused. Each frame copies into a free one and asks for it to be mapped;
/// mapped ones are harvested (oldest first) on later frames. Unlike
/// [`copy_render_target_to_cpu`], the render thread never waits on the GPU
/// and nothing is allocated per frame on the GPU side; a frame reaches the
/// screen a frame or two later.
#[derive(Resource, Default)]
pub(crate) struct ReadbackRing {
    slots: Vec<RingSlot>,
    seq: u64,
}

struct RingSlot {
    buffer: Buffer,
    width: u32,
    height: u32,
    padded_bpr: u32,
    state: std::sync::Arc<std::sync::atomic::AtomicU8>,
    pts_ms: u64,
    seq: u64,
}

const SLOT_FREE: u8 = 0;
const SLOT_PENDING: u8 = 1;
const SLOT_MAPPED: u8 = 2;
const SLOT_FAILED: u8 = 3;
const RING_SLOTS: usize = 3;

pub(crate) fn copy_render_target_ring(
    target: Option<Res<CaptureTarget>>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    tx: Res<CaptureChannelTx>,
    pts: Option<Res<LastAppliedPts>>,
    mut ring: ResMut<ReadbackRing>,
) {
    use std::sync::atomic::Ordering;

    // Collect what the GPU has finished, without waiting.
    device.poll(Maintain::Poll);
    let mut ready: Vec<usize> = (0..ring.slots.len())
        .filter(|&i| ring.slots[i].state.load(Ordering::Acquire) == SLOT_MAPPED)
        .collect();
    ready.sort_by_key(|&i| ring.slots[i].seq);
    for i in ready {
        let s = &ring.slots[i];
        let unpadded = (s.width * 4) as usize;
        let mut tight = Vec::with_capacity(unpadded * s.height as usize);
        {
            let view = s.buffer.slice(..).get_mapped_range();
            for row in view.chunks_exact(s.padded_bpr as usize).take(s.height as usize) {
                tight.extend_from_slice(&row[..unpadded]);
            }
        }
        s.buffer.unmap();
        s.state.store(SLOT_FREE, Ordering::Release);
        let _ = tx.tx.try_send(CapturedFrame { pixels: tight, width: s.width, height: s.height, pts_ms: s.pts_ms });
    }
    for s in &ring.slots {
        if s.state.load(Ordering::Acquire) == SLOT_FAILED {
            s.state.store(SLOT_FREE, Ordering::Release);
        }
    }

    let Some(t) = target else { return };
    let Some(gpu_image) = images.get(&t.image) else { return };
    // The GPU image's own size: after a resize it can lag the target's by a frame.
    let (width, height) = (gpu_image.size.x, gpu_image.size.y);
    let padded_bpr = (width * 4).div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
    let free = |s: &RingSlot| s.state.load(Ordering::Acquire) == SLOT_FREE;
    let slot = match ring.slots.iter().position(|s| free(s) && s.width == width && s.height == height) {
        Some(i) => i,
        None => {
            let fresh = RingSlot {
                buffer: device.create_buffer(&BufferDescriptor {
                    label: Some("cc_render_overlay_readback"),
                    size: padded_bpr as u64 * height as u64,
                    usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                width,
                height,
                padded_bpr,
                state: Default::default(),
                pts_ms: 0,
                seq: 0,
            };
            if ring.slots.len() < RING_SLOTS {
                ring.slots.push(fresh);
                ring.slots.len() - 1
            } else if let Some(i) = ring.slots.iter().position(free) {
                // A free slot of an old size: replace it.
                ring.slots[i] = fresh;
                i
            } else {
                // All in flight: the GPU is behind; skip this frame.
                return;
            }
        }
    };

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("cc_render_overlay_readback") });
    let s = &ring.slots[slot];
    encoder.copy_texture_to_buffer(
        gpu_image.texture.as_image_copy(),
        ImageCopyBuffer {
            buffer: &s.buffer,
            layout: ImageDataLayout { offset: 0, bytes_per_row: Some(padded_bpr), rows_per_image: Some(height) },
        },
        Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit(std::iter::once(encoder.finish()));
    let state = s.state.clone();
    state.store(SLOT_PENDING, Ordering::Release);
    s.buffer.slice(..).map_async(MapMode::Read, move |res| {
        state.store(if res.is_ok() { SLOT_MAPPED } else { SLOT_FAILED }, Ordering::Release);
    });
    ring.seq += 1;
    let seq = ring.seq;
    let s = &mut ring.slots[slot];
    s.seq = seq;
    s.pts_ms = pts.map(|p| p.0).unwrap_or(0);
}
