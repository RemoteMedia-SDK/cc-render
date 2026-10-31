//! Does a transparent, click-through, never-activating Bevy window composite
//! per-pixel alpha over the desktop on this machine's GPU and backend?
//!
//! Draws a lit sphere and a half-transparent ring on a fully transparent
//! clear in a borderless, always-on-top window.
//!
//! Result (Bevy 0.15.3 / wgpu 23.0.1, RTX 4090, driver 617.42, Windows 11):
//! **no**, so `Present::Overlay` presents through a layered Win32 window
//! instead (`cc_render::overlay`). DX12 offers only `Opaque` (black
//! background); NVIDIA's Vulkan surface accepts `PreMultiplied` but DWM
//! composites it over white; `post` / `inherit` fail validation. Focus
//! and click-through do work. Re-run after a Bevy / wgpu upgrade.
//!
//! Also found: changing the window's extended style from a system that runs
//! on a worker thread deadlocks (`SetWindowLongPtrW` sends WM_STYLECHANGED
//! to the winit thread, which waits on the schedule); `no_activate` takes
//! `NonSend<WinitWindows>` to run on the main thread.
//!
//! ```text
//! cargo run --release --example overlay_probe
//!   PROBE_ALPHA=auto|pre|post|opaque|inherit   (default auto)
//!   PROBE_TAA=1                                 TAA instead of MSAA
//!   PROBE_HDR=0                                 LDR camera
//!   PROBE_HITTEST=1                             not click-through
//!   PROBE_FOCUS=1                               focused when created
//!   PROBE_NOACT=0                               no WS_EX_NOACTIVATE
//!   PROBE_UPDATE=continuous|reactive            (default reactive_low_power)
//!   PROBE_SECS=N                                exit after N seconds (default 20)
//!   WGPU_BACKEND=vulkan|dx12
//! ```

use std::time::Duration;

use bevy::{
    core_pipeline::{experimental::taa::{TemporalAntiAliasPlugin, TemporalAntiAliasing}, tonemapping::Tonemapping},
    prelude::*,
    window::{CompositeAlphaMode, CursorOptions, WindowLevel, WindowResolution},
    winit::{UpdateMode, WinitSettings},
};

fn main() {
    let alpha = match std::env::var("PROBE_ALPHA").as_deref() {
        Ok("pre") => CompositeAlphaMode::PreMultiplied,
        Ok("post") => CompositeAlphaMode::PostMultiplied,
        Ok("opaque") => CompositeAlphaMode::Opaque,
        Ok("inherit") => CompositeAlphaMode::Inherit,
        _ => CompositeAlphaMode::Auto,
    };
    let taa = std::env::var("PROBE_TAA").as_deref() == Ok("1");
    let secs: f32 = std::env::var("PROBE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(20.0);
    eprintln!("overlay_probe: alpha={alpha:?} taa={taa} secs={secs}");
    let skip_noact = std::env::var("PROBE_NOACT").as_deref() == Ok("0");

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "overlay_probe".into(),
            resolution: WindowResolution::new(400.0, 400.0),
            position: WindowPosition::At(IVec2::new(200, 200)),
            transparent: true,
            decorations: false,
            resizable: false,
            focused: std::env::var("PROBE_FOCUS").as_deref() == Ok("1"),
            skip_taskbar: true,
            window_level: WindowLevel::AlwaysOnTop,
            composite_alpha_mode: alpha,
            cursor_options: CursorOptions { hit_test: std::env::var("PROBE_HITTEST").as_deref() == Ok("1"), ..default() },
            ..default()
        }),
        ..default()
    }))
    .insert_resource(match std::env::var("PROBE_UPDATE").as_deref() {
        Ok("continuous") => WinitSettings { focused_mode: UpdateMode::Continuous, unfocused_mode: UpdateMode::Continuous },
        Ok("reactive") => WinitSettings {
            focused_mode: UpdateMode::reactive(Duration::from_secs_f64(1.0 / 30.0)),
            unfocused_mode: UpdateMode::reactive(Duration::from_secs_f64(1.0 / 30.0)),
        },
        _ => WinitSettings {
            focused_mode: UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 30.0)),
            unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 30.0)),
        },
    })
    .insert_resource(ProbeSecs(secs))
    .add_systems(Startup, setup)
    .add_systems(Update, (spin, exit_after, heartbeat));
    if !skip_noact {
        app.add_systems(Update, no_activate);
    }
    if taa {
        app.add_plugins(TemporalAntiAliasPlugin);
    }
    app.insert_resource(ProbeTaa(taa));
    app.run();
}

#[derive(Resource)]
struct ProbeSecs(f32);
#[derive(Resource)]
struct ProbeTaa(bool);

#[derive(Component)]
struct Spin;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    taa: Res<ProbeTaa>,
) {
    let hdr = std::env::var("PROBE_HDR").as_deref() != Ok("0");
    let cam = commands
        .spawn((
            Camera3d::default(),
            Camera { hdr, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
            Tonemapping::AgX,
            Transform::from_xyz(0.0, 0.0, 3.2).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    if taa.0 {
        commands.entity(cam).insert((Msaa::Off, TemporalAntiAliasing::default()));
    } else {
        commands.entity(cam).insert(Msaa::Sample4);
    }
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.8).mesh().uv(64, 32))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.45, 0.2),
            perceptual_roughness: 0.4,
            ..default()
        })),
        Transform::default(),
        Spin,
    ));
    // A half-transparent ring, to see blended edges against the desktop.
    commands.spawn((
        Mesh3d(meshes.add(Torus::new(0.95, 1.15))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.2, 0.6, 1.0, 0.4),
            alpha_mode: AlphaMode::Blend,
            ..default()
        })),
        Transform::from_rotation(Quat::from_rotation_x(1.2)),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 8000.0, ..default() },
        Transform::from_xyz(2.0, 3.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spin(time: Res<Time>, mut q: Query<&mut Transform, With<Spin>>) {
    for mut t in &mut q {
        t.rotation = Quat::from_rotation_y(time.elapsed_secs());
    }
}

/// WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW once the window exists. On the main
/// (winit) thread: `SetWindowLongPtrW` sends WM_STYLECHANGED to the window's
/// thread and waits, so from a worker thread it deadlocks the schedule.
fn no_activate(
    winit: NonSend<bevy::winit::WinitWindows>,
    q: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(e) = q.get_single() else { return };
    let Some(w) = winit.get_window(e) else { return };
    #[cfg(windows)]
    {
        use raw_window_handle::HasWindowHandle;
        let Ok(h) = w.window_handle() else { return };
        if let raw_window_handle::RawWindowHandle::Win32(w) = h.as_raw() {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;
            // SAFETY: a live top-level window owned by this thread.
            unsafe {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize);
                let ex2 = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                eprintln!("overlay_probe: hwnd={hwnd:?} exstyle {ex:#x} -> {ex2:#x}");
            }
        }
    }
    *done = true;
}

fn heartbeat(time: Res<Time>, mut n: Local<u32>) {
    *n += 1;
    if *n == 1 || *n % 60 == 0 {
        eprintln!("overlay_probe: update #{} at {:.2}s", *n, time.elapsed_secs());
    }
}

fn exit_after(time: Res<Time>, secs: Res<ProbeSecs>, mut exit: EventWriter<AppExit>) {
    if time.elapsed_secs() > secs.0 {
        exit.send(AppExit::Success);
    }
}
