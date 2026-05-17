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
