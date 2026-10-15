//! Corner panel for the display-only home overlay. It rebuilds UI nodes from
//! the latest producer-reported state and never touches avatar or scene entities.
use crate::cc_render::office::home_hud::{HomeHud, StepStatus};
use bevy::prelude::*;
use tokio::sync::watch;

#[derive(Resource)]
pub(super) struct HomeHudRx(pub watch::Receiver<Option<HomeHud>>);

#[derive(Component)]
pub(super) struct HomeHudRoot;

const PANEL: Color = Color::srgba(0.06, 0.07, 0.09, 0.78);
const TEXT: Color = Color::srgb(0.93, 0.93, 0.9);
const MUTED: Color = Color::srgb(0.62, 0.64, 0.66);
const TRACK: Color = Color::srgba(1., 1., 1., 0.12);

pub(super) fn spawn_home_hud(mut commands: Commands) {
    commands.spawn((
        Name::new("Home HUD"),
        HomeHudRoot,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(16.),
            top: Val::Px(16.),
            width: Val::Px(300.),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.),
            padding: UiRect::all(Val::Px(12.)),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderRadius::all(Val::Px(8.)),
        Visibility::Hidden,
    ));
}

pub(super) fn update_home_hud(
    mut rx: ResMut<HomeHudRx>,
    mut commands: Commands,
    mut root: Query<(Entity, &mut Visibility), With<HomeHudRoot>>,
) {
    if !rx.0.has_changed().unwrap_or(false) {
        return;
    }
    let hud = rx.0.borrow_and_update().clone();
    let Ok((entity, mut visibility)) = root.get_single_mut() else { return };
    commands.entity(entity).despawn_descendants();
    let Some(hud) = hud else {
        *visibility = Visibility::Hidden;
        return;
    };
    *visibility = Visibility::Inherited;
    commands.entity(entity).with_children(|panel| {
        text(panel, if hud.autonomy { "Home - autonomous" } else { "Home - queued" }, 13., MUTED);
        for need in &hud.needs {
            panel
                .spawn(Node { column_gap: Val::Px(8.), align_items: AlignItems::Center, ..default() })
                .with_children(|row| {
                    row.spawn((Node { width: Val::Px(84.), ..default() },))
                        .with_children(|cell| text(cell, &need.label, 14., TEXT));
                    row.spawn((
                        Node { width: Val::Px(176.), height: Val::Px(10.), ..default() },
                        BackgroundColor(TRACK),
                        BorderRadius::all(Val::Px(5.)),
                    ))
                    .with_children(|track| {
                        track.spawn((
                            Node { width: Val::Percent(need.value * 100.), height: Val::Percent(100.), ..default() },
                            BackgroundColor(need_color(need.value)),
                            BorderRadius::all(Val::Px(5.)),
                        ));
                    });
                });
        }
        if let Some(current) = &hud.current {
            panel.spawn(Node { height: Val::Px(4.), ..default() });
            text(panel, &current.label, 16., TEXT);
            for step in &current.steps {
                let (mark, color) = step_style(step.status);
                text(panel, &format!("{mark} {}", step.label), 14., color);
            }
            if let Some(detail) = &current.detail {
                text(panel, detail, 13., step_style(StepStatus::Failed).1);
            }
        }
        if !hud.queue.is_empty() {
            panel.spawn(Node { height: Val::Px(4.), ..default() });
            text(panel, &format!("Next: {}", hud.queue.join(", ")), 13., MUTED);
        }
    });
}

fn text(parent: &mut ChildBuilder, value: &str, size: f32, color: Color) {
    parent.spawn((Text::new(value), TextFont { font_size: size, ..default() }, TextColor(color)));
}

fn need_color(value: f32) -> Color {
    if value > 0.6 {
        Color::srgb(0.36, 0.78, 0.45)
    } else if value > 0.3 {
        Color::srgb(0.95, 0.73, 0.25)
    } else {
        Color::srgb(0.92, 0.34, 0.3)
    }
}

// ASCII marks: the embedded default font has no check-mark glyphs.
fn step_style(status: StepStatus) -> (&'static str, Color) {
    match status {
        StepStatus::Done => ("[x]", Color::srgb(0.36, 0.78, 0.45)),
        StepStatus::Running => ("[>]", Color::srgb(0.98, 0.9, 0.55)),
        StepStatus::Pending => ("[ ]", MUTED),
        StepStatus::Failed => ("[!]", Color::srgb(0.95, 0.42, 0.38)),
        StepStatus::Blocked => ("[-]", Color::srgb(0.72, 0.55, 0.9)),
    }
}
