//! Wireframe overlay of every Rapier collider in the scene, drawn via
//! Bevy gizmos so it appears in captured frames without needing the
//! `bevy_rapier3d/debug-render-3d` feature (which the workspace
//! intentionally doesn't enable — it pulls in a debug render plugin we
//! don't want in production builds).
//!
//! Color palette (matches the four collider categories produced by
//! the `physics` install path):
//! - **Cyan**    — `BodyBoneCollider` (avatar body capsule chain)
//! - **Yellow**  — `Prop` (dynamic gameplay objects)
//! - **Magenta** — `JiggleBone` (secondary-motion dynamic bodies)
//! - **Gray**    — `StaticWorldEntity` (ground + walls + ceiling)
//! - **Red**     — fallback (ungated colliders, useful for debugging
//!   mis-tagged spawns)
//!
//! Toggle via `CC_PHYSICS_DEBUG_OVERLAY=1`. Off by default so production
//! captures aren't littered with wireframes.

use bevy::color::palettes::css;
use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore};
use bevy::prelude::*;
use bevy_rapier3d::geometry::shape_views::ColliderView;
use bevy_rapier3d::prelude::*;

use super::body_colliders::BodyBoneCollider;
use super::props::Prop;
use super::secondary::JiggleBone;
use super::world::StaticWorldEntity;

#[derive(Resource, Clone, Copy)]
pub(crate) struct PhysicsDebugConfig {
    pub enabled: bool,
}

impl PhysicsDebugConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("CC_PHYSICS_DEBUG_OVERLAY").ok().as_deref() == Some("1"),
        }
    }
}

/// Forces overlay gizmos to draw ON TOP of everything (depth_bias=-1.0
/// is Bevy's "always in front" sentinel) so capsules don't get hidden
/// behind opaque body geometry. Bumps line_width a bit too.
pub(crate) fn configure_debug_overlay(mut store: ResMut<GizmoConfigStore>) {
    let (cfg, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    // Note: this overrides any previous depth_bias from joint_debug,
    // which is OK because both want the same "draw on top" behavior.
    if cfg.depth_bias > -1.0 {
        cfg.depth_bias = -1.0;
    }
    if cfg.line_width < 1.5 {
        cfg.line_width = 1.5;
    }
}

#[derive(Copy, Clone)]
enum Category {
    Body,
    Prop,
    Jiggle,
    Static,
    Other,
}

impl Category {
    fn color(self) -> Srgba {
        match self {
            Category::Body => css::AQUA,
            Category::Prop => css::YELLOW,
            Category::Jiggle => css::MAGENTA,
            Category::Static => css::GRAY,
            Category::Other => css::RED,
        }
    }
}

/// Decide which palette color a collider gets. Order matters: a body
/// capsule that somehow also carried a Prop tag should still draw as a
/// body. This shouldn't happen in practice (these tags are
/// mutually-exclusive by design) but keeps the dispatch deterministic.
fn classify(is_body: bool, is_prop: bool, is_jiggle: bool, is_static: bool) -> Category {
    if is_body {
        return Category::Body;
    }
    if is_jiggle {
        return Category::Jiggle;
    }
    if is_prop {
        return Category::Prop;
    }
    if is_static {
        return Category::Static;
    }
    Category::Other
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_debug_overlay(
    cfg: Res<PhysicsDebugConfig>,
    mut gizmos: Gizmos,
    colliders: Query<(
        Entity,
        &Collider,
        &GlobalTransform,
        Option<&BodyBoneCollider>,
        Option<&Prop>,
        Option<&JiggleBone>,
        Option<&StaticWorldEntity>,
    )>,
) {
    if !cfg.enabled {
        return;
    }
    for (_e, collider, gt, body, prop, jiggle, statik) in colliders.iter() {
        let cat = classify(
            body.is_some(),
            prop.is_some(),
            jiggle.is_some(),
            statik.is_some(),
        );
        let color = cat.color();
        let t = gt.compute_transform();
        draw_collider_view(
            &mut gizmos,
            collider.as_typed_shape(),
            t.translation,
            t.rotation,
            t.scale,
            color,
        );
    }
}

/// Recursive draw — `Compound` views call back into this with each
/// sub-shape's local offset composed onto the parent transform.
fn draw_collider_view(
    gizmos: &mut Gizmos,
    view: ColliderView,
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
    color: Srgba,
) {
    match view {
        ColliderView::Ball(ball) => {
            // Sphere gizmo takes (isometry, radius, color); world radius
            // is local radius × max-of-uniform-scale. Pure-uniform-scale
            // is the only case we expect on Rapier shapes (non-uniform
            // scale of a sphere is degenerate); fall back to the average
            // for safety.
            let r = ball.radius() * scale_for_sphere(scale);
            gizmos.sphere(Isometry3d::new(translation, rotation), r, color);
        }
        ColliderView::Cuboid(c) => {
            // `gizmos.cuboid` draws a unit cube transformed by the
            // Transform's full SRT — set scale so the unit cube
            // becomes 2×half_extents on each axis.
            let h = c.half_extents();
            let mut t = Transform::from_translation(translation);
            t.rotation = rotation;
            t.scale = scale * Vec3::new(h.x, h.y, h.z) * 2.0;
            gizmos.cuboid(t, color);
        }
        ColliderView::Capsule(cap) => {
            let r = cap.radius() * scale_for_sphere(scale);
            let seg = cap.segment();
            // Capsule endpoints are in local space; transform to world.
            let a_world = translation + rotation * (Vec3::from(seg.a()) * scale);
            let b_world = translation + rotation * (Vec3::from(seg.b()) * scale);
            // End caps + 4 longitudinal lines = a recognizable capsule
            // wireframe without needing a dedicated capsule gizmo.
            gizmos.sphere(Isometry3d::new(a_world, rotation), r, color);
            gizmos.sphere(Isometry3d::new(b_world, rotation), r, color);
            // Local capsule axis is +Y in Rapier's capsule_y convention
            // — pick the two perpendicular world axes from the rotation.
            let local_x = rotation * Vec3::X * r;
            let local_z = rotation * Vec3::Z * r;
            gizmos.line(a_world + local_x, b_world + local_x, color);
            gizmos.line(a_world - local_x, b_world - local_x, color);
            gizmos.line(a_world + local_z, b_world + local_z, color);
            gizmos.line(a_world - local_z, b_world - local_z, color);
        }
        ColliderView::Compound(compound) => {
            // Recurse into sub-shapes. Each sub-shape's `(offset, rot)`
            // is in the parent collider's local space, so compose with
            // the parent's world translation+rotation.
            for (sub_offset, sub_rot, sub_view) in compound.shapes() {
                let world_offset = translation + rotation * (Vec3::from(sub_offset) * scale);
                let world_rot = rotation * Quat::from(sub_rot);
                draw_collider_view(gizmos, sub_view, world_offset, world_rot, scale, color);
            }
        }
        // Other shapes (TriMesh, ConvexHull, …) aren't produced by our
        // current install path; fall back to a small marker so they're
        // visible if someone adds a new shape kind.
        _ => {
            gizmos.sphere(Isometry3d::new(translation, rotation), 0.05, color);
        }
    }
}

/// Pick a single radius scale for a sphere/capsule given a possibly
/// non-uniform Vec3 scale. We use max-of-abs so the wireframe always
/// circumscribes the actual collider — and so a negative scale (e.g.
/// X-mirroring) doesn't degenerate to a near-zero radius.
fn scale_for_sphere(scale: Vec3) -> f32 {
    let a = scale.abs();
    a.x.max(a.y).max(a.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_priority() {
        assert!(matches!(classify(true, true, true, true), Category::Body));
        assert!(matches!(
            classify(false, true, true, true),
            Category::Jiggle
        ));
        assert!(matches!(classify(false, true, false, true), Category::Prop));
        assert!(matches!(
            classify(false, false, false, true),
            Category::Static
        ));
        assert!(matches!(
            classify(false, false, false, false),
            Category::Other
        ));
    }

    #[test]
    fn category_colors_distinct() {
        let colors = [
            Category::Body.color(),
            Category::Prop.color(),
            Category::Jiggle.color(),
            Category::Static.color(),
            Category::Other.color(),
        ];
        // Sanity check: no two categories share a color, otherwise the
        // wireframe overlay would conflate categories at a glance.
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(
                    (colors[i].red, colors[i].green, colors[i].blue),
                    (colors[j].red, colors[j].green, colors[j].blue),
                    "categories {} and {} have identical color",
                    i,
                    j,
                );
            }
        }
    }

    #[test]
    fn scale_for_sphere_picks_max() {
        assert!((scale_for_sphere(Vec3::new(1.0, 2.0, 0.5)) - 2.0).abs() < 1e-6);
        assert!((scale_for_sphere(Vec3::new(-3.0, 1.0, 1.0)) - 3.0).abs() < 1e-6);
        assert!((scale_for_sphere(Vec3::splat(1.0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn debug_disabled_by_default_when_env_unset() {
        // SAFETY: tests share a process — only set/unset for the
        // duration of this test.
        // SAFETY: setting env is unsafe in newer Rust editions because
        // the runtime can read env from other threads. This test is
        // single-threaded and the value is read synchronously below.
        let prev = std::env::var("CC_PHYSICS_DEBUG_OVERLAY").ok();
        unsafe {
            std::env::remove_var("CC_PHYSICS_DEBUG_OVERLAY");
        }
        let cfg = PhysicsDebugConfig::from_env();
        assert!(!cfg.enabled);
        if let Some(v) = prev {
            unsafe {
                std::env::set_var("CC_PHYSICS_DEBUG_OVERLAY", v);
            }
        }
    }
}
