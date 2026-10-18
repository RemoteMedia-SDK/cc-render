use super::*;
use bevy::time::{create_time_channels, TimePlugin, TimeSender};
use bevy::utils::{Duration, Instant};

fn app_with_render_clock(step: Option<Duration>) -> (App, TimeSender, Instant) {
    let mut app = App::new();
    app.add_plugins(TimePlugin);
    app.init_resource::<crate::cc_render::office::timeline::PoseTimeline>();
    let startup = Instant::now() - Duration::from_secs(1);
    app.insert_resource(Time::<Real>::new(startup));
    let (sender, receiver) = create_time_channels();
    app.insert_resource(receiver);
    install_body_clock(&mut app, step);
    (app, sender, startup)
}

#[test]
fn headless_frame_clock_uses_current_wall_time_despite_stale_render_completion() {
    let (mut app, sender, startup) = app_with_render_clock(None);
    for offset in [100, 105, 180] {
        // Deliberately uneven, stale GPU timestamps. The complete App/TimePlugin
        // schedule must choose the actual current instant and drain the channel.
        sender.0.send(startup + Duration::from_millis(offset)).unwrap();
        let before = Instant::now();
        app.update();
        let after = Instant::now();
        let selected = app.world().resource::<Time<Real>>().last_update().unwrap();
        assert!(selected >= before && selected <= after);
        assert!(sender.0.is_empty(), "render timing channel must not accumulate stale frames");
    }
}

#[test]
fn headless_frame_clock_preserves_exact_offline_steps() {
    let step = Duration::from_secs_f64(1.0 / 30.0);
    let (mut app, sender, startup) = app_with_render_clock(Some(step));
    for (index, offset) in [100, 105, 180].into_iter().enumerate() {
        sender.0.send(startup + Duration::from_millis(offset)).unwrap();
        app.update();
        let real = app.world().resource::<Time<Real>>();
        assert_eq!(real.elapsed(), step * index as u32);
        assert_eq!(app.world().resource::<Time>().elapsed(), step * index as u32);
        assert!(sender.0.is_empty());
    }
}

#[test]
fn headless_frame_clock_preserves_virtual_pause_while_real_time_advances() {
    let (mut app, sender, startup) = app_with_render_clock(None);
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    for offset in [100, 105] {
        sender.0.send(startup + Duration::from_millis(offset)).unwrap();
        let before = Instant::now();
        app.update();
        assert!(app.world().resource::<Time<Real>>().last_update().unwrap() >= before);
        assert_eq!(app.world().resource::<Time>().elapsed(), Duration::ZERO);
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
    }
}
