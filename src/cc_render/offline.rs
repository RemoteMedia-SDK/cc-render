//! Off-line rendering (`Present::Offline`): one frame per pose, as fast as
//! the GPU goes, each a fixed `1/fps` after the last, on a transparent
//! background. For rendering a recorded performance to video: physics,
//! the idle animation and TAA see exactly the time steps they would at
//! `fps` in real time.
//!
//! ```ignore
//! let mut r = OfflineRenderer::spawn(config)?;
//! r.wait_ready(Duration::from_secs(120))?;
//! for pose in poses {
//!     let mut frame = r.render(pose)?;
//!     straight_alpha(&mut frame.pixels);
//!     // ... frame.pixels: RGBA8, sRGB, straight alpha
//! }
//! r.shutdown()?;
//! ```

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};

use super::renderer::{ArkitPose, CcRenderer, Present, RenderedFrame, RendererConfig};

/// How long one frame may take before the render counts as stuck.
const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OfflineRenderer {
    inner: CcRenderer,
    /// The pts the next pose is tagged with; frames echo it.
    next_pts: u64,
}

impl OfflineRenderer {
    /// Start the renderer (`config.present` is set to `Offline`).
    pub fn spawn(mut config: RendererConfig) -> Result<Self> {
        config.present = Present::Offline;
        Ok(Self { inner: CcRenderer::spawn(config)?, next_pts: 1 })
    }

    pub fn config(&self) -> &RendererConfig {
        self.inner.config()
    }

    fn channels(&self) -> &super::renderer::OfflineChannels {
        self.inner.offline.as_ref().expect("spawned with Present::Offline")
    }

    /// Wait for the avatar to load (frames rendered meanwhile are dropped).
    pub fn wait_ready(&self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        let frames = &self.channels().frames;
        while !self.inner.is_ready() {
            while frames.try_recv().is_ok() {}
            if start.elapsed() > timeout {
                bail!("the avatar did not load within {:.0} s", timeout.as_secs_f64());
            }
            if self.inner.thread_finished() {
                bail!("the renderer stopped while loading");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        while frames.try_recv().is_ok() {}
        Ok(())
    }

    /// Render `pose` as the next frame. Premultiplied RGBA (as the
    /// renderer writes it); see [`straight_alpha`].
    pub fn render(&mut self, mut pose: ArkitPose) -> Result<RenderedFrame> {
        let pts = self.next_pts;
        self.next_pts += 1;
        pose.pts_ms = pts;
        self.inner.push_pose(pose);
        let ch = self.channels();
        ch.step_tx
            .lock()
            .as_ref()
            .ok_or_else(|| anyhow!("the renderer is shut down"))?
            .send_timeout((), FRAME_TIMEOUT)
            .map_err(|_| anyhow!("the renderer stopped taking frames"))?;
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let f = ch.frames.recv_timeout(left).map_err(|_| anyhow!("no frame within {FRAME_TIMEOUT:?}"))?;
            if f.pts_ms == pts {
                return Ok(RenderedFrame { pixels: f.pixels, width: f.width, height: f.height, pts_ms: pts });
            }
            // An older frame (from loading): skip it.
        }
    }

    /// Render `pose` `n` times and drop the frames, to let physics and TAA
    /// settle before the first frame that counts.
    pub fn warm_up(&mut self, pose: &ArkitPose, n: usize) -> Result<()> {
        for _ in 0..n {
            self.render(pose.clone())?;
        }
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        self.inner.shutdown()
    }
}

impl Drop for OfflineRenderer {
    fn drop(&mut self) {
        let _ = self.inner.shutdown();
    }
}

/// The renderer's pixels to straight (unassociated) alpha, in place.
///
/// It writes `srgb(colour × alpha)` over a transparent clear, blending
/// premultiplied in linear light. This returns `srgb(colour)` with alpha
/// unchanged, the same colour the desktop overlay shows (its lookup
/// table makes `srgb(colour) × alpha` from the same bytes).
pub fn straight_alpha(rgba: &mut [u8]) {
    let lut = straight_lut();
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as usize;
        if a == 255 {
            continue;
        }
        let row = &lut[a];
        px[0] = row[px[0] as usize];
        px[1] = row[px[1] as usize];
        px[2] = row[px[2] as usize];
    }
}

/// Indexed `[alpha][value]`.
fn straight_lut() -> &'static [[u8; 256]; 256] {
    static LUT: OnceLock<Box<[[u8; 256]; 256]>> = OnceLock::new();
    LUT.get_or_init(|| {
        fn decode(x: f32) -> f32 {
            if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
        }
        fn encode(x: f32) -> f32 {
            if x <= 0.003_130_8 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
        }
        let mut lut = Box::new([[0u8; 256]; 256]);
        for (a, row) in lut.iter_mut().enumerate().skip(1) {
            let af = a as f32 / 255.0;
            for (v, out) in row.iter_mut().enumerate() {
                let c = (decode(v as f32 / 255.0) / af).min(1.0);
                *out = (encode(c) * 255.0).round() as u8;
            }
        }
        lut
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_alpha_undoes_the_linear_premultiply() {
        // White at half coverage is written as srgb(0.5) = 188.
        let mut px = [188u8, 188, 188, 128, 10, 20, 30, 255, 0, 0, 0, 0];
        straight_alpha(&mut px);
        assert!(px[0] >= 253, "{px:?}");
        assert_eq!(px[3], 128);
        assert_eq!(&px[4..8], &[10, 20, 30, 255], "opaque pixels untouched");
        assert_eq!(&px[8..], &[0, 0, 0, 0], "empty stays empty");
    }
}
