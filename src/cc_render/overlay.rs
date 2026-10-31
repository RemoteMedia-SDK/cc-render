//! Desktop overlay presentation (`Present::Overlay`): the avatar drawn on
//! top of the desktop or a game, in a window that never takes focus and
//! lets clicks through.
//!
//! Bevy renders offscreen as usual (transparent clear, no backdrop); the
//! render app reads each frame back through a non-blocking ring
//! (`capture::copy_render_target_ring`) and sends it here. A Win32 layered
//! window shows it with `UpdateLayeredWindow` (per-pixel, premultiplied
//! alpha). Bevy's own swapchain can't be the overlay: on wgpu 23, DX12
//! offers only an opaque surface, and NVIDIA's Vulkan surface takes
//! premultiplied alpha but DWM composites it over white
//! (`examples/overlay_probe.rs`). A layered window composites correctly on
//! every GPU, and a face-sized frame is ~0.5 MB of readback.
//!
//! Window: `WS_EX_LAYERED | WS_EX_TRANSPARENT` (clicks pass through),
//! `WS_EX_NOACTIVATE` (never focused, also on click), `WS_EX_TOOLWINDOW`
//! (no taskbar button, not in Alt-Tab), `WS_EX_TOPMOST` (re-asserted every
//! few seconds: games raise themselves). Games in true exclusive fullscreen
//! hide it; borderless windowed shows it.
//!
//! Edit mode (the hotkey, default Ctrl+Alt+O, or [`OverlayHandle`]): the
//! window turns solid and tinted; drag it with the left button, scale it
//! with the wheel, lock it again with a right click or the hotkey. Locking
//! publishes the new [`OverlayGeometry`] for the host to save.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};

use super::bevy_app::capture::CapturedFrame;

/// Which corner (or the centre) of the monitor's work area the overlay
/// sits in, `margin` logical pixels in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
    Center,
}

impl std::str::FromStr for Anchor {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        serde_json::from_value(serde_json::Value::String(s.to_ascii_lowercase()))
            .map_err(|_| format!("{s}: expected top-left, top-right, bottom-left, bottom-right or center"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    pub anchor: Anchor,
    /// Logical pixels between the overlay and the work area's edges.
    pub margin: i32,
    /// Side of the (square) overlay in logical pixels; the render size is
    /// this times the monitor's scale.
    pub size: u32,
    /// Monitor index: 0 is the primary, then the others in Windows' order.
    pub monitor: usize,
    /// A saved top-left in physical (virtual-screen) pixels; overrides the
    /// anchor while it is on some monitor.
    pub position: Option<[i32; 2]>,
    /// Global hotkey toggling edit mode, e.g. "Ctrl+Alt+O"; `None`: none.
    pub hotkey: Option<String>,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            anchor: Anchor::BottomRight,
            margin: 24,
            size: 320,
            monitor: 0,
            position: None,
            hotkey: Some("Ctrl+Alt+O".into()),
        }
    }
}

/// Where the overlay is after an edit: top-left in physical pixels, side
/// in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayGeometry {
    pub x: i32,
    pub y: i32,
    pub size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayCmd {
    ToggleEdit,
    SetEdit(bool),
}

struct Shared {
    cmd_tx: Sender<OverlayCmd>,
    cmd_rx: Receiver<OverlayCmd>,
    geometry: tokio::sync::watch::Sender<Option<OverlayGeometry>>,
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (geometry, _) = tokio::sync::watch::channel(None);
        Shared { cmd_tx, cmd_rx, geometry }
    })
}

/// The host's side of the (one, per process) overlay: works before the
/// renderer has started; commands wait for it.
#[derive(Clone)]
pub struct OverlayHandle {
    cmd_tx: Sender<OverlayCmd>,
}

impl OverlayHandle {
    pub fn toggle_edit(&self) {
        let _ = self.cmd_tx.send(OverlayCmd::ToggleEdit);
    }
    pub fn set_edit(&self, on: bool) {
        let _ = self.cmd_tx.send(OverlayCmd::SetEdit(on));
    }
    /// The geometry published each time edit mode is locked.
    pub fn geometry(&self) -> tokio::sync::watch::Receiver<Option<OverlayGeometry>> {
        shared().geometry.subscribe()
    }
}

pub fn handle() -> OverlayHandle {
    OverlayHandle { cmd_tx: shared().cmd_tx.clone() }
}

// ---------------------------------------------------------------------------
// Pure helpers (tested on every platform)
// ---------------------------------------------------------------------------

/// A rectangle in physical pixels: left, top, right, bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

/// Top-left of a `side`-pixel square anchored in `work`, `margin` in.
pub(crate) fn anchored_position(work: Rect, side: i32, anchor: Anchor, margin: i32) -> (i32, i32) {
    let (left, right) = (work.l + margin, work.r - margin - side);
    let (top, bottom) = (work.t + margin, work.b - margin - side);
    let cx = work.l + (work.r - work.l - side) / 2;
    let cy = work.t + (work.b - work.t - side) / 2;
    match anchor {
        Anchor::TopLeft => (left, top),
        Anchor::TopRight => (right, top),
        Anchor::BottomLeft => (left, bottom),
        Anchor::BottomRight => (right, bottom),
        Anchor::Center => (cx, cy),
    }
}

pub(crate) fn scale(logical: i32, dpi: u32) -> i32 {
    ((logical as i64 * dpi.max(1) as i64 + 48) / 96) as i32
}

/// "Ctrl+Alt+O" → (MOD_* bits, virtual-key code). Letters, digits, F1-F24.
pub(crate) fn parse_hotkey(s: &str) -> Option<(u32, u32)> {
    const MOD_ALT: u32 = 0x1;
    const MOD_CONTROL: u32 = 0x2;
    const MOD_SHIFT: u32 = 0x4;
    const MOD_WIN: u32 = 0x8;
    let mut mods = 0;
    let mut vk = None;
    for part in s.split('+').map(|p| p.trim().to_ascii_uppercase()) {
        match part.as_str() {
            "CTRL" | "CONTROL" => mods |= MOD_CONTROL,
            "ALT" => mods |= MOD_ALT,
            "SHIFT" => mods |= MOD_SHIFT,
            "WIN" | "SUPER" => mods |= MOD_WIN,
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => {
                vk = Some(k.as_bytes()[0] as u32)
            }
            k if k.starts_with('F') => match k[1..].parse::<u32>() {
                Ok(n @ 1..=24) => vk = Some(0x70 + n - 1),
                _ => return None,
            },
            _ => return None,
        }
    }
    vk.map(|vk| (mods, vk))
}

/// The renderer writes `srgb(tonemapped colour × alpha)` over a transparent
/// clear (blending premultiplies in linear light); `UpdateLayeredWindow`
/// wants `srgb(colour) × alpha`. Indexed `[alpha][value]`.
fn premultiply_lut() -> &'static [[u8; 256]; 256] {
    static LUT: OnceLock<Box<[[u8; 256]; 256]>> = OnceLock::new();
    LUT.get_or_init(|| {
        fn decode(x: f32) -> f32 {
            if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
        }
        fn encode(x: f32) -> f32 {
            if x <= 0.003_130_8 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
        }
        let mut lut = Box::new([[0u8; 256]; 256]);
        for a in 1..256 {
            let af = a as f32 / 255.0;
            for v in 0..256 {
                lut[a][v] = if a == 255 {
                    v as u8
                } else {
                    let c = (decode(v as f32 / 255.0) / af).min(1.0);
                    (encode(c) * af * 255.0).round().min(a as f32) as u8
                };
            }
        }
        lut
    })
}

/// Edit mode's backdrop (premultiplied BGRA) and its 2 px border.
const TINT: [u8; 4] = [38, 26, 18, 110];
const BORDER: [u8; 4] = [200, 150, 60, 220];

/// RGBA from the renderer → premultiplied BGRA for the layered window,
/// over the edit tint when `edit`.
pub(crate) fn to_layered_bgra(rgba: &[u8], side: usize, edit: bool, out: &mut [u8]) {
    let lut = premultiply_lut();
    for (i, (src, dst)) in rgba.chunks_exact(4).zip(out.chunks_exact_mut(4)).enumerate() {
        let a = src[3] as usize;
        let row = &lut[a];
        let mut px = [row[src[2] as usize], row[src[1] as usize], row[src[0] as usize], src[3]];
        if edit {
            let (x, y) = (i % side, i / side);
            let under = if x < 2 || y < 2 || x + 2 >= side || y + 2 >= side { BORDER } else { TINT };
            let k = 255 - a as u32;
            for c in 0..4 {
                px[c] = (px[c] as u32 + (under[c] as u32 * k + 127) / 255).min(255) as u8;
            }
        }
        dst.copy_from_slice(&px);
    }
}

/// The Bevy app's end of a running overlay: where frames go, and the
/// render size the window wants (physical pixels per side).
pub(crate) struct OverlaySink {
    pub tx: Sender<CapturedFrame>,
    pub target_px: Arc<AtomicU32>,
}

// ---------------------------------------------------------------------------
// The presenter thread (Windows)
// ---------------------------------------------------------------------------

/// Start the overlay window on its own thread. Frames sent to the returned
/// sender (latest wins) are shown once their size matches `target_px`,
/// which the presenter sets from `size` × the monitor's scale and the Bevy
/// app follows. The thread ends when every sender is dropped.
#[cfg(windows)]
pub(crate) fn spawn_presenter(
    config: OverlayConfig,
    target_px: Arc<AtomicU32>,
) -> anyhow::Result<(Sender<CapturedFrame>, std::thread::JoinHandle<()>)> {
    let (tx, rx) = crossbeam_channel::bounded::<CapturedFrame>(2);
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<(), String>>(1);
    let join = std::thread::Builder::new().name("cc-render-overlay".into()).spawn(move || {
        win::run(config, target_px, rx, ready_tx);
    })?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok((tx, join)),
        Ok(Err(e)) => Err(anyhow::anyhow!("overlay window: {e}")),
        Err(_) => Err(anyhow::anyhow!("overlay window thread exited")),
    }
}

#[cfg(not(windows))]
pub(crate) fn spawn_presenter(
    _config: OverlayConfig,
    _target_px: Arc<AtomicU32>,
) -> anyhow::Result<(Sender<CapturedFrame>, std::thread::JoinHandle<()>)> {
    anyhow::bail!("the desktop overlay is only implemented on Windows")
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{
        GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        MDT_EFFECTIVE_DPI,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, ReleaseCapture, SetCapture, UnregisterHotKey, MOD_NOREPEAT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const HOTKEY_ID: i32 = 0x0CC5;
    const MIN_SIDE: u32 = 96;
    const MAX_SIDE: u32 = 1440;

    struct Monitor {
        handle: HMONITOR,
        work: Rect,
        primary: bool,
    }

    fn monitors() -> Vec<Monitor> {
        unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
            let list = &mut *(data as *mut Vec<Monitor>);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(m, &mut info) != 0 {
                let w = info.rcWork;
                list.push(Monitor {
                    handle: m,
                    work: Rect { l: w.left, t: w.top, r: w.right, b: w.bottom },
                    primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
                });
            }
            1
        }
        let mut list: Vec<Monitor> = Vec::new();
        // SAFETY: the callback only runs during this call, on this thread.
        unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(each), &mut list as *mut _ as LPARAM) };
        list.sort_by_key(|m| !m.primary); // stable: primary first, then Windows' order
        list
    }

    fn dpi_of(m: HMONITOR) -> u32 {
        let (mut x, mut y) = (96u32, 96u32);
        // SAFETY: plain query on a monitor handle.
        let hr = unsafe { GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
        if hr < 0 { 96 } else { x }
    }

    fn monitor_at(x: i32, y: i32) -> HMONITOR {
        // SAFETY: plain query.
        unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_MOUSEACTIVATE {
            return MA_NOACTIVATE as LRESULT;
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }

    struct Dib {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
        bits: *mut u8,
        side: u32,
    }

    impl Dib {
        fn new(side: u32) -> Option<Self> {
            // SAFETY: a top-down 32 bpp DIB section selected into a memory DC
            // owned by this struct; freed in Drop.
            unsafe {
                let dc = CreateCompatibleDC(std::ptr::null_mut());
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = side as i32;
                bmi.bmiHeader.biHeight = -(side as i32);
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB;
                let mut bits = std::ptr::null_mut();
                let bitmap = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
                if bitmap.is_null() || bits.is_null() {
                    DeleteDC(dc);
                    return None;
                }
                let old = SelectObject(dc, bitmap);
                Some(Self { dc, bitmap, old, bits: bits as *mut u8, side })
            }
        }
        fn pixels(&mut self) -> &mut [u8] {
            // SAFETY: the DIB section holds side² BGRA pixels for our lifetime.
            unsafe { std::slice::from_raw_parts_mut(self.bits, (self.side * self.side * 4) as usize) }
        }
    }

    impl Drop for Dib {
        fn drop(&mut self) {
            // SAFETY: objects created in `new`.
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    struct Overlay {
        hwnd: HWND,
        x: i32,
        y: i32,
        /// Logical side and the monitor scale it is shown at.
        logical: u32,
        dpi: u32,
        target_px: Arc<AtomicU32>,
        edit: bool,
        /// While dragging: the cursor's offset from the window's top-left.
        drag: Option<(i32, i32)>,
        last: Option<(Vec<u8>, u32)>,
        dib: Option<Dib>,
    }

    fn cursor() -> (i32, i32) {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: plain query.
        unsafe { GetCursorPos(&mut p) };
        (p.x, p.y)
    }

    impl Overlay {
        fn side(&self) -> u32 {
            scale(self.logical as i32, self.dpi).clamp(16, 4096) as u32
        }

        fn present(&mut self) {
            let side = self.side();
            if self.dib.as_ref().map(|d| d.side) != Some(side) {
                self.dib = Dib::new(side);
            }
            let edit = self.edit;
            let Some(dib) = self.dib.as_mut() else { return };
            match &self.last {
                Some((rgba, s)) if *s == side => to_layered_bgra(rgba, side as usize, edit, dib.pixels()),
                // No frame at this size yet (loading, or just resized).
                _ => {
                    let blank = vec![0u8; (side * side * 4) as usize];
                    to_layered_bgra(&blank, side as usize, edit, dib.pixels());
                }
            }
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let pos = POINT { x: self.x, y: self.y };
            let size = SIZE { cx: side as i32, cy: side as i32 };
            let src = POINT { x: 0, y: 0 };
            // SAFETY: our window and DIB.
            unsafe {
                UpdateLayeredWindow(self.hwnd, std::ptr::null_mut(), &pos, &size, dib.dc, &src, 0, &blend, ULW_ALPHA);
            }
        }

        fn set_edit(&mut self, on: bool) {
            if on == self.edit {
                return;
            }
            self.edit = on;
            if self.drag.take().is_some() {
                // SAFETY: our capture.
                unsafe { ReleaseCapture() };
            }
            // SAFETY: our window, on its own thread.
            unsafe {
                let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
                let ex = if on { ex & !(WS_EX_TRANSPARENT as isize) } else { ex | WS_EX_TRANSPARENT as isize };
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex);
            }
            self.present();
            if on {
                tracing::info!(target: "cc_render", "overlay: edit mode (drag to move, wheel to scale, right-click to lock)");
            } else {
                let g = OverlayGeometry { x: self.x, y: self.y, size: self.logical };
                tracing::info!(target: "cc_render", "overlay: locked at {g:?}");
                shared().geometry.send_replace(Some(g));
            }
        }

        /// Scale around the centre by one wheel notch.
        fn rescale(&mut self, up: bool) {
            let old = self.side() as i32;
            let logical = if up { self.logical as f32 * 1.1 } else { self.logical as f32 / 1.1 };
            self.logical = (logical.round() as u32).clamp(MIN_SIDE, MAX_SIDE);
            let new = self.side() as i32;
            self.x -= (new - old) / 2;
            self.y -= (new - old) / 2;
            self.target_px.store(new as u32, Ordering::Release);
            self.present();
        }

        /// Follow the cursor while dragging.
        fn drag_to(&mut self, (cx, cy): (i32, i32)) {
            let Some((ox, oy)) = self.drag else { return };
            (self.x, self.y) = (cx - ox, cy - oy);
            // SAFETY: our window.
            unsafe {
                SetWindowPos(self.hwnd, std::ptr::null_mut(), self.x, self.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }

        /// After a drag: its monitor's scale.
        fn moved(&mut self) {
            let side = self.side() as i32;
            let m = monitor_at(self.x + side / 2, self.y + side / 2);
            if !m.is_null() {
                let dpi = dpi_of(m);
                if dpi != self.dpi {
                    self.dpi = dpi;
                    self.target_px.store(self.side(), Ordering::Release);
                    self.present();
                }
            }
        }
    }

    pub(super) fn run(
        config: OverlayConfig,
        target_px: Arc<AtomicU32>,
        frames: Receiver<CapturedFrame>,
        ready: Sender<Result<(), String>>,
    ) {
        // SAFETY: affects this thread only; coordinates below are physical.
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let mons = monitors();
        let Some(mon) = mons.get(config.monitor).or_else(|| mons.first()) else {
            let _ = ready.send(Err("no monitors".into()));
            return;
        };
        if config.monitor >= mons.len() {
            tracing::warn!(target: "cc_render", "overlay: no monitor {} ({} found); using the primary", config.monitor, mons.len());
        }
        let logical = config.size.clamp(MIN_SIDE, MAX_SIDE);
        // A saved position wins while it is still on a monitor.
        let saved = config.position.and_then(|[x, y]| {
            let probe = scale(logical as i32, 96) / 2;
            let m = monitor_at(x + probe, y + probe);
            (!m.is_null()).then(|| (x, y, dpi_of(m)))
        });
        let (x, y, dpi) = saved.unwrap_or_else(|| {
            let dpi = dpi_of(mon.handle);
            let side = scale(logical as i32, dpi);
            let (x, y) = anchored_position(mon.work, side, config.anchor, scale(config.margin, dpi));
            (x, y, dpi)
        });

        // SAFETY: standard window creation on this thread; the class is
        // registered once per process (a second registration fails harmlessly).
        let hwnd = unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = wide("cc_render_overlay");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc);
            let title = wide("Avatar overlay");
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                x,
                y,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            let _ = ready.send(Err(format!("CreateWindowExW failed: {}", std::io::Error::last_os_error())));
            return;
        }
        let mut ov = Overlay { hwnd, x, y, logical, dpi, target_px, edit: false, drag: None, last: None, dib: None };
        ov.target_px.store(ov.side(), Ordering::Release);
        let hotkey = config.hotkey.as_deref().filter(|s| !s.trim().is_empty());
        let mut hotkey_registered = false;
        if let Some(h) = hotkey {
            match parse_hotkey(h) {
                // SAFETY: our window.
                Some((mods, vk)) => {
                    hotkey_registered = unsafe { RegisterHotKey(hwnd, HOTKEY_ID, mods | MOD_NOREPEAT, vk) } != 0;
                    if !hotkey_registered {
                        tracing::warn!(target: "cc_render", "overlay: hotkey {h} is taken by another program");
                    }
                }
                None => tracing::warn!(target: "cc_render", "overlay: can't parse hotkey {h:?}"),
            }
        }
        // SAFETY: our window.
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        ov.present();
        tracing::info!(
            target: "cc_render",
            "overlay: {}x{} px at ({x}, {y}), {dpi} dpi{}",
            ov.side(),
            ov.side(),
            if hotkey_registered { format!("; {} toggles edit mode", hotkey.unwrap_or_default()) } else { String::new() }
        );
        let _ = ready.send(Ok(()));

        let cmds = shared().cmd_rx.clone();
        let mut last_topmost = Instant::now();
        'run: loop {
            // SAFETY: standard message pump for this thread's window.
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    match msg.message {
                        WM_QUIT => break 'run,
                        WM_HOTKEY if msg.wParam as i32 == HOTKEY_ID => ov.set_edit(!ov.edit),
                        // Our own drag (the system's caption move loop doesn't
                        // move a no-activate layered window), so frames keep
                        // coming while it moves.
                        WM_LBUTTONDOWN if ov.edit => {
                            let (cx, cy) = cursor();
                            ov.drag = Some((cx - ov.x, cy - ov.y));
                            SetCapture(hwnd);
                        }
                        WM_MOUSEMOVE if ov.drag.is_some() => ov.drag_to(cursor()),
                        WM_LBUTTONUP if ov.drag.is_some() => {
                            ov.drag_to(cursor());
                            ov.drag = None;
                            ReleaseCapture();
                            ov.moved();
                        }
                        WM_RBUTTONUP if ov.edit => ov.set_edit(false),
                        WM_MOUSEWHEEL if ov.edit => {
                            let delta = ((msg.wParam >> 16) & 0xffff) as u16 as i16;
                            ov.rescale(delta > 0);
                        }
                        _ => {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }
            }
            while let Ok(cmd) = cmds.try_recv() {
                match cmd {
                    OverlayCmd::ToggleEdit => ov.set_edit(!ov.edit),
                    OverlayCmd::SetEdit(on) => ov.set_edit(on),
                }
            }
            let mut newest = match frames.recv_timeout(Duration::from_millis(15)) {
                Ok(f) => Some(f),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };
            while let Ok(f) = frames.try_recv() {
                newest = Some(f);
            }
            if let Some(f) = newest {
                // Frames from before a resize are dropped.
                if f.width == f.height && f.width == ov.side() {
                    ov.last = Some((f.pixels, f.width));
                    ov.present();
                }
            }
            if last_topmost.elapsed() > Duration::from_secs(2) {
                last_topmost = Instant::now();
                // SAFETY: our window.
                unsafe {
                    SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
            }
        }
        drop(ov.dib.take());
        // SAFETY: our window, destroyed on its own thread.
        unsafe {
            if hotkey_registered {
                UnregisterHotKey(hwnd, HOTKEY_ID);
            }
            DestroyWindow(hwnd);
        }
        tracing::info!(target: "cc_render", "overlay: closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect { l: 0, t: 0, r: 1920, b: 1040 };

    #[test]
    fn anchors_sit_margin_in_from_the_work_area() {
        assert_eq!(anchored_position(WORK, 300, Anchor::BottomRight, 24), (1596, 716));
        assert_eq!(anchored_position(WORK, 300, Anchor::TopLeft, 24), (24, 24));
        assert_eq!(anchored_position(WORK, 300, Anchor::Center, 24), (810, 370));
        let second = Rect { l: 1920, t: -200, r: 3840, b: 880 };
        assert_eq!(anchored_position(second, 100, Anchor::TopRight, 10), (3730, -190));
    }

    #[test]
    fn logical_pixels_scale_with_dpi() {
        assert_eq!(scale(320, 96), 320);
        assert_eq!(scale(320, 144), 480);
        assert_eq!(scale(24, 120), 30);
    }

    #[test]
    fn hotkeys_parse() {
        assert_eq!(parse_hotkey("Ctrl+Alt+O"), Some((0x3, b'O' as u32)));
        assert_eq!(parse_hotkey("shift + f9"), Some((0x4, 0x78)));
        assert_eq!(parse_hotkey("Win+1"), Some((0x8, b'1' as u32)));
        assert_eq!(parse_hotkey("Ctrl+Alt"), None);
        assert_eq!(parse_hotkey("Ctrl+F25"), None);
        assert_eq!(parse_hotkey("Hyper+O"), None);
    }

    #[test]
    fn anchors_parse_from_flags() {
        assert_eq!("bottom-left".parse::<Anchor>(), Ok(Anchor::BottomLeft));
        assert_eq!("CENTER".parse::<Anchor>(), Ok(Anchor::Center));
        assert!("left".parse::<Anchor>().is_err());
    }

    #[test]
    fn premultiplied_conversion() {
        // opaque: swizzled, unchanged; transparent: zero
        let mut out = [0u8; 8];
        to_layered_bgra(&[10, 20, 30, 255, 99, 99, 99, 0], 2, false, &mut out);
        assert_eq!(out, [30, 20, 10, 255, 0, 0, 0, 0]);
        // half-covered white: srgb(1 × 0.5) ≈ 188 from the renderer → 1 × 0.5 ≈ 128
        to_layered_bgra(&[188, 188, 188, 128, 0, 0, 0, 0], 2, false, &mut out);
        assert!((out[0] as i32 - 128).abs() <= 2, "{out:?}");
        assert!(out[0] <= out[3]);
    }

    #[test]
    fn edit_mode_shows_the_tint_where_transparent() {
        let side = 8;
        let rgba = vec![0u8; side * side * 4];
        let mut out = vec![0u8; side * side * 4];
        to_layered_bgra(&rgba, side, true, &mut out);
        let centre = (4 * side + 4) * 4;
        assert_eq!(&out[centre..centre + 4], &TINT);
        assert_eq!(&out[0..4], &BORDER);
    }

    #[test]
    fn config_defaults_and_json() {
        let c: OverlayConfig = serde_json::from_str(r#"{"anchor":"top-left","size":256}"#).unwrap();
        assert_eq!(c.anchor, Anchor::TopLeft);
        assert_eq!(c.size, 256);
        assert_eq!(c.margin, 24);
        assert_eq!(c.hotkey.as_deref(), Some("Ctrl+Alt+O"));
    }
}
