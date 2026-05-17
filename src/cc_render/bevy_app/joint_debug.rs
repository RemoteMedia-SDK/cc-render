//! Visual debug overlay for the SMPL-22 → CC5 retarget pipeline.
//!
//! Two layers, gated by `CC_AVATAR_DEBUG_JOINTS=1`:
//!
//! 1. **Gizmos** — skeleton lines parent→child, joint markers, and a
//!    small RGB axis triad per joint (X=red/Y=green/Z=blue) so you can
//!    eyeball each bone's local rotation frame against the avatar.
//! 2. **UI text labels** — Euler-angle readout (`yaw/pitch/roll` in
//!    degrees, YXZ order) per joint, projected to screen space via
//!    `Camera::world_to_viewport`, anchored next to each joint marker.
//!
//! Off by default (zero overhead). Set `CC_AVATAR_DEBUG_JOINTS=1` to
//! render the overlay into the same MP4 the smoke binary writes.

use bevy::color::palettes::css;
use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore};
use bevy::prelude::*;
use bevy::ui::PositionType;

use super::pose::{CC5BindRotations, SMPL22_TO_CC5, SMPL_PARENTS};

/// Resource holding tunables for the debug overlay. Built once at app
/// boot from env vars (no live editing needed for the smoke flow).
#[derive(Resource, Clone, Copy)]
pub(crate) struct JointDebugConfig {
    pub enabled: bool,
    /// Length of the per-joint RGB axis triad, in scene units (meters).
    pub axis_len: f32,
    /// Radius of the per-joint marker sphere.
    pub marker_radius: f32,
    /// UI font size for the rotation labels.
    pub label_font_size: f32,
}

impl Default for JointDebugConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            axis_len: 0.06,
            marker_radius: 0.012,
            label_font_size: 11.0,
        }
    }
}

impl JointDebugConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("CC_AVATAR_DEBUG_JOINTS").ok().as_deref() == Some("1");
        let axis_len = std::env::var("CC_AVATAR_DEBUG_AXIS_LEN")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.06);
        let marker_radius = std::env::var("CC_AVATAR_DEBUG_MARKER_RADIUS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.012);
        let label_font_size = std::env::var("CC_AVATAR_DEBUG_FONT_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(11.0);
        Self {
            enabled,
            axis_len,
            marker_radius,
            label_font_size,
        }
    }
}

/// Configure the default gizmo group so skeleton lines + axis triads
/// + joint markers draw ON TOP of the avatar mesh (no depth test).
///
/// `depth_bias = -1.0` is Bevy's "always in front" sentinel
/// (range: -1.0 = always front, 0.0 = normal, 1.0 = always behind).
/// `line_width` is bumped a touch so the lines stay readable on top of
/// the heavily-shaded skin shader.
///
/// Runs once at Startup. No-op when debug overlay is disabled — the
/// bumped settings have zero cost when no gizmos are emitted.
pub(crate) fn configure_gizmo_overlay(
    cfg: Res<JointDebugConfig>,
    mut store: ResMut<GizmoConfigStore>,
) {
    if !cfg.enabled {
        return;
    }
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.depth_bias = -1.0;
    config.line_width = 2.0;
    tracing::info!(
        target: "cc_render",
        "joint-debug: gizmos forced-on-top (depth_bias=-1.0, line_width=2.0)"
    );
}

/// Marker on each spawned joint-label UI node. Carries the SMPL index
/// so the update system knows which bone it represents.
#[derive(Component)]
pub(crate) struct JointLabel(pub usize);

/// Tracks whether we've spawned the 22 UI labels yet. Done lazily once
/// `CC5BindRotations.captured == true` (i.e. after the bone entity
/// handles have been populated).
#[derive(Resource, Default)]
pub(crate) struct JointLabelsSpawned(pub bool);

/// Draw the skeleton: parent→child line for each SMPL bone with a
/// known parent, plus a small sphere marker at each joint and an RGB
/// axis triad showing the bone's animated local rotation frame.
///
/// Runs in `PostUpdate` AFTER `TransformSystem::TransformPropagate` so
/// that the `GlobalTransform` we read reflects the just-applied pose.
pub(crate) fn draw_joint_skeleton(
    cfg: Res<JointDebugConfig>,
    bind: Res<CC5BindRotations>,
    transforms: Query<&GlobalTransform>,
    mut gizmos: Gizmos,
) {
    if !cfg.enabled || !bind.captured {
        return;
    }

    // Cache joint world positions in SMPL index order.
    let mut joint_pos: [Option<Vec3>; 22] = [None; 22];
    for i in 0..22 {
        let Some(e) = bind.entities[i] else { continue };
        let Ok(gt) = transforms.get(e) else { continue };
        joint_pos[i] = Some(gt.translation());
    }

    // Skeleton lines: parent → child.
    for i in 0..22 {
        let Some(p) = joint_pos[i] else { continue };
        let parent_idx = SMPL_PARENTS[i];
        if parent_idx >= 0 {
            if let Some(parent_pos) = joint_pos[parent_idx as usize] {
                gizmos.line(parent_pos, p, css::LIME);
            }
        }
    }

    // Per-joint markers + local-frame axes.
    for i in 0..22 {
        let Some(p) = joint_pos[i] else { continue };
        let Some(e) = bind.entities[i] else { continue };
        let Ok(gt) = transforms.get(e) else { continue };

        // Marker sphere.
        gizmos.sphere(
            Isometry3d::from_translation(p),
            cfg.marker_radius,
            css::YELLOW,
        );

        // RGB axis triad in this bone's animated world rotation frame.
        let (_, rot, _) = gt.to_scale_rotation_translation();
        let rx = rot * Vec3::X * cfg.axis_len;
        let ry = rot * Vec3::Y * cfg.axis_len;
        let rz = rot * Vec3::Z * cfg.axis_len;
        gizmos.line(p, p + rx, css::RED);
        gizmos.line(p, p + ry, css::LIME);
        gizmos.line(p, p + rz, css::DEEP_SKY_BLUE);
    }
}

/// Spawn 22 UI text nodes (one per joint) on the first tick after bind
/// capture finishes. Idempotent via `JointLabelsSpawned`.
pub(crate) fn spawn_joint_labels(
    cfg: Res<JointDebugConfig>,
    bind: Res<CC5BindRotations>,
    mut spawned: ResMut<JointLabelsSpawned>,
    mut commands: Commands,
) {
    if !cfg.enabled || spawned.0 || !bind.captured {
        return;
    }
    spawned.0 = true;

    for i in 0..22 {
        commands.spawn((
            Text::new(""),
            TextFont {
                font_size: cfg.label_font_size,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.95, 0.55)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(-9999.0),
                top: Val::Px(-9999.0),
                ..default()
            },
            JointLabel(i),
        ));
    }
    tracing::info!(
        target: "cc_render",
        "joint-debug: spawned {} UI labels (CC_AVATAR_DEBUG_JOINTS=1)",
        22
    );
}

/// Update the 22 label nodes each frame: project bone world position
/// to viewport coords, format Euler readout from the bone's animated
/// world rotation, and reposition the UI node.
pub(crate) fn update_joint_labels(
    cfg: Res<JointDebugConfig>,
    bind: Res<CC5BindRotations>,
    transforms: Query<&GlobalTransform>,
    cam_q: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut labels: Query<(&JointLabel, &mut Text, &mut Node)>,
) {
    if !cfg.enabled || !bind.captured {
        return;
    }
    let Ok((cam, cam_xform)) = cam_q.get_single() else {
        return;
    };

    for (lbl, mut text, mut node) in labels.iter_mut() {
        let i = lbl.0;
        let Some(e) = bind.entities[i] else {
            node.left = Val::Px(-9999.0);
            node.top = Val::Px(-9999.0);
            continue;
        };
        let Ok(gt) = transforms.get(e) else {
            node.left = Val::Px(-9999.0);
            node.top = Val::Px(-9999.0);
            continue;
        };

        let world_pos = gt.translation();
        let viewport = match cam.world_to_viewport(cam_xform, world_pos) {
            Ok(v) => v,
            Err(_) => {
                // Off-screen / behind camera — park the node off-canvas.
                node.left = Val::Px(-9999.0);
                node.top = Val::Px(-9999.0);
                continue;
            }
        };

        let (_, rot, _) = gt.to_scale_rotation_translation();
        let (yaw, pitch, roll) = rot.to_euler(EulerRot::YXZ);
        let smpl_name = SMPL22_TO_CC5[i].0;
        text.0 = format!(
            "{smpl_name} {y:>4.0}/{p:>4.0}/{r:>4.0}",
            y = yaw.to_degrees(),
            p = pitch.to_degrees(),
            r = roll.to_degrees(),
        );

        // Anchor a few px right + below the joint marker so the label
        // doesn't sit directly on top of the gizmo dot.
        node.left = Val::Px(viewport.x + 6.0);
        node.top = Val::Px(viewport.y + 4.0);
    }
}
