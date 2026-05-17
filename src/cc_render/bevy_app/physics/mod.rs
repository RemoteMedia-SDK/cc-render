//! Rapier physics integration for the CC5 renderer.
//!
//! In the host crate this lived behind the `avatar-render-cc-physics`
//! cargo feature; in this Path-3 plugin it's always compiled in.

pub(crate) mod body_colliders;
pub(crate) mod cc5_json;
pub(crate) mod debug_overlay;
pub(crate) mod grab;
pub(crate) mod props;
pub(crate) mod secondary;
pub(crate) mod sidecar;
pub(crate) mod world;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::cc_render::renderer::PhysicsConfig;
use world::{spawn_ground, spawn_walls, GroundAutoDetect, StaticWorldEntity, WorldGroundY};

/// Install Rapier + world setup. Called from `bevy_app::run` when
/// `RendererConfig::physics` is `Some`. `glb_path` is the renderer's
/// avatar GLB path, used by sidecar auto-discovery.
pub(crate) fn install(app: &mut App, config: PhysicsConfig, glb_path: PathBuf) {
    // bevy_rapier3d 0.28: timestep_mode is a separate `Resource`,
    // gravity lives on the `RapierConfiguration` *Component* attached
    // to the RapierContext entity. Plugin add first; then mutate.
    app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default());

    // TimestepMode::Interpolated: physics runs at fixed dt, but render
    // frames sample interpolated body poses between physics steps when
    // the body has a `TransformInterpolation` component. Without
    // interpolation, a 60 Hz physics + 30 fps render would freeze the
    // body for one render frame out of every two — visible as
    // sub-frame jitter on fast-oscillating jiggle bodies. The
    // `time_scale: 1.0` keeps real-time speed; `substeps` from
    // PhysicsConfig::max_substeps_per_frame.
    app.insert_resource(TimestepMode::Interpolated {
        dt: 1.0 / config.timestep_hz.max(1) as f32,
        time_scale: 1.0,
        substeps: config.max_substeps_per_frame.max(1) as usize,
    });

    // Override gravity on every RapierConfiguration component the
    // plugin spawns. Runs once at PostStartup so the plugin's
    // `setup_rapier_configuration` (which inserts the component) has
    // executed first.
    let gravity = Vec3::new(config.gravity[0], config.gravity[1], config.gravity[2]);
    app.add_systems(
        PostStartup,
        move |mut q: Query<&mut RapierConfiguration>| {
            let mut count = 0usize;
            for mut cfg in q.iter_mut() {
                cfg.gravity = gravity;
                count += 1;
            }
            if count == 0 {
                tracing::warn!(
                    target: "cc_render",
                    "physics: no RapierConfiguration component found at PostStartup; \
                     gravity override will not apply. This indicates a bevy_rapier3d \
                     version mismatch — RapierConfiguration moved to lazy-init."
                );
            }
        },
    );

    // Debug renderer requires the `debug-render-3d` feature on
    // bevy_rapier3d, which the workspace doesn't enable. Log a hint
    // when CC_PHYSICS_DEBUG=1 is set so the user knows what to flip.
    if std::env::var("CC_PHYSICS_DEBUG").as_deref() == Ok("1") {
        tracing::info!(
            target: "cc_render",
            "CC_PHYSICS_DEBUG=1: Rapier debug renderer is gated on the \
             `bevy_rapier3d/debug-render-3d` feature, which is currently \
             disabled in this workspace build. Use \
             CC_PHYSICS_DEBUG_OVERLAY=1 for the gizmo-based overlay instead."
        );
    }

    // Gizmo-based collider overlay: zero overhead when disabled,
    // wireframes-on-top when CC_PHYSICS_DEBUG_OVERLAY=1.
    let dbg = debug_overlay::PhysicsDebugConfig::from_env();
    app.insert_resource(dbg);
    if dbg.enabled {
        tracing::info!(
            target: "cc_render",
            "CC_PHYSICS_DEBUG_OVERLAY=1: drawing collider wireframes \
             (cyan=body, yellow=props, magenta=jiggle, gray=static)"
        );
        // Configure gizmo group at startup so depth_bias is set before
        // any frame draws — keeps wireframes from popping in/out as
        // the avatar moves under them.
        app.add_systems(Startup, debug_overlay::configure_debug_overlay);
        // Draw in PostUpdate AFTER TransformPropagate so every
        // GlobalTransform we read reflects this frame's pose +
        // physics writeback.
        app.add_systems(
            bevy::app::PostUpdate,
            debug_overlay::draw_debug_overlay
                .after(bevy::transform::TransformSystem::TransformPropagate),
        );
    }

    // Stash config for later systems (auto-fit capsules, sidecar load, etc.)
    app.insert_resource(StoredPhysicsConfig(config.clone()));

    // Resolve ground_y. None ⇒ start at 0.0 + auto-detect from avatar
    // foot bones once `CC5BindRotations.captured` becomes true (the
    // `auto_detect_ground_from_feet` Update system below will despawn
    // the placeholder static world and respawn at the discovered Y).
    let auto_detect = config.ground_y.is_none();
    let resolved_ground = config.ground_y.unwrap_or(0.0);
    app.insert_resource(WorldGroundY(resolved_ground));
    app.insert_resource(GroundAutoDetect(auto_detect));

    let pv = config.play_volume.clone();
    let pv_for_startup = pv.clone();
    app.add_systems(
        Startup,
        move |mut commands: Commands, ground: Res<WorldGroundY>| {
            spawn_ground(
                &mut commands,
                ground.0,
                pv_for_startup
                    .as_ref()
                    .map(|p| p.half_extents_xz)
                    .unwrap_or(2.0),
            );
            if let Some(p) = &pv_for_startup {
                spawn_walls(&mut commands, ground.0, p);
            }
        },
    );

    if auto_detect {
        let pv_for_adjust = pv;
        app.add_systems(
            Update,
            move |mut commands: Commands,
                  bind: Res<super::pose::CC5BindRotations>,
                  mut world_ground: ResMut<WorldGroundY>,
                  mut auto: ResMut<GroundAutoDetect>,
                  transforms: Query<&GlobalTransform>,
                  statics: Query<Entity, With<StaticWorldEntity>>| {
                if !auto.0 || !bind.captured {
                    return;
                }
                // One-shot regardless of whether we successfully derived
                // a Y — avoids re-running every frame on edge cases.
                let new_y = world::lowest_foot_y(|smpl_idx| {
                    bind.entities[smpl_idx]
                        .and_then(|e| transforms.get(e).ok())
                        .map(|gt| gt.translation())
                });
                auto.0 = false;
                let Some(new_y) = new_y else {
                    tracing::warn!(
                        target: "cc_render",
                        "physics: auto-detect ground_y found no foot bones; \
                         keeping placeholder Y={:.3}",
                        world_ground.0,
                    );
                    return;
                };
                if (new_y - world_ground.0).abs() < 1e-4 {
                    // Already correct — no respawn needed.
                    tracing::debug!(
                        target: "cc_render",
                        "physics: auto-detect ground_y matched placeholder ({:.3})",
                        new_y,
                    );
                    return;
                }
                world_ground.0 = new_y;
                for e in statics.iter() {
                    commands.entity(e).despawn();
                }
                spawn_ground(
                    &mut commands,
                    new_y,
                    pv_for_adjust
                        .as_ref()
                        .map(|p| p.half_extents_xz)
                        .unwrap_or(2.0),
                );
                if let Some(p) = &pv_for_adjust {
                    spawn_walls(&mut commands, new_y, p);
                }
                tracing::info!(
                    target: "cc_render",
                    "physics: auto-detected ground_y = {:.3} from avatar foot bones",
                    new_y,
                );
            },
        );
    }

    app.init_resource::<body_colliders::BodyCollidersSpawned>();
    // `auto_fit_capsules` reads `CC5BindRotations`; the main bevy_app
    // setup also `init_resource`s it, but for tests / standalone
    // installs (no full pose pipeline) we make sure it exists.
    // `init_resource` is idempotent, so it's a no-op when already set.
    app.init_resource::<super::pose::CC5BindRotations>();

    // CC5 native physics JSON (e.g. <glb>.json from Reallusion CC5).
    // Sibling to the GLB, NOT the same as the physics sidecar.
    // Auto-discovered via `cc5_json::resolve_cc5_json_path`. Loaded
    // into the optional `LoadedCc5Physics` resource that
    // `body_colliders::auto_fit_capsules` consults BELOW the sidecar
    // override and ABOVE the auto-fit fallback.
    let glb_path_for_cc5 = glb_path.clone();
    app.add_systems(Startup, move |mut commands: Commands| {
        let Some(p) = cc5_json::resolve_cc5_json_path(&glb_path_for_cc5) else {
            return;
        };
        match cc5_json::load(&p) {
            Ok(Some(cc5)) => {
                let n = cc5.by_bone.values().map(|v| v.len()).sum::<usize>();
                tracing::info!(
                    target: "cc_render",
                    "physics: loaded CC5 native physics JSON {:?} \
                     ({} bones, {} authored shapes)",
                    p, cc5.by_bone.len(), n,
                );
                commands.insert_resource(body_colliders::LoadedCc5Physics(cc5));
            }
            Ok(None) => tracing::debug!(target: "cc_render", "physics: no CC5 JSON at {:?}", p),
            Err(e) => tracing::warn!(target: "cc_render", "physics: CC5 JSON parse failed at {:?}: {e}", p),
        }
    });

    // Load sidecar. Explicit `sidecar_path` wins; otherwise auto-discover
    // at `<glb>.physics.json` via `sidecar::resolve_sidecar_path`.
    let sidecar_explicit = config.sidecar_path.clone();
    app.add_systems(Startup, move |mut commands: Commands| {
        let resolved = sidecar::resolve_sidecar_path(&glb_path, sidecar_explicit.as_deref());
        match sidecar::load(&resolved) {
            Ok(Some(s)) => {
                tracing::info!(target: "cc_render", "physics: loaded sidecar {:?}", resolved);
                commands.insert_resource(body_colliders::LoadedSidecar(s));
            }
            Ok(None) => tracing::info!(target: "cc_render", "physics: no sidecar at {:?} — defaults", resolved),
            Err(e) => tracing::warn!(target: "cc_render", "physics: sidecar parse failed at {:?}: {e}", resolved),
        }
    });

    app.add_systems(Update, body_colliders::auto_fit_capsules);

    // Secondary-motion (hair/cloth/breast/tongue) jiggle bones.
    // `write_jiggle_bones` writes `Transform`, so it MUST run AFTER
    // PhysicsSet::Writeback (Rapier writes the dynamic body's Transform)
    // and BEFORE TransformPropagate (Bevy rebuilds GlobalTransform).
    app.init_resource::<secondary::JiggleSpawned>();
    app.add_systems(Update, secondary::auto_detect_chains);
    app.add_systems(
        bevy::app::PostUpdate,
        secondary::write_jiggle_bones
            .after(bevy_rapier3d::plugin::PhysicsSet::Writeback)
            .before(bevy::transform::TransformSystem::TransformPropagate),
    );
    // Inertia injection: compute parent angular velocity → centrifugal
    // pseudo-force on translation-mode bodies. Must run BEFORE
    // PhysicsSet::SyncBackend so the force is in place when Rapier
    // integrates this frame's step. Reads stale GlobalTransforms
    // (TransformPropagate hasn't run yet for this frame's pose); the
    // 1-frame lag is imperceptible at 30+ fps.
    app.add_systems(
        bevy::app::PostUpdate,
        secondary::apply_jiggle_inertia.before(bevy_rapier3d::plugin::PhysicsSet::SyncBackend),
    );

    // Prop lifecycle + scene_state publisher.
    app.init_resource::<props::PropRegistry>();
    app.init_resource::<grab::GrabFsms>();
    app.add_systems(Update, props::handle_prop_cmds);
    // Polls the AssetServer for MeshGlb prop scenes and replaces the
    // placeholder ball collider with a tight cuboid sized from the
    // loaded mesh AABB. No-op once the prop's `MeshGlbColliderPending`
    // tag has been removed (after a successful finalize OR a
    // zero-mesh scene), so the polling cost is bounded.
    app.add_systems(Update, props::finalize_meshglb_collider);
    app.add_systems(
        bevy::app::PostUpdate,
        props::publish_scene_state.after(bevy::transform::TransformSystem::TransformPropagate),
    );

    app.add_systems(
        bevy::app::PostUpdate,
        grab::handle_grab_cmds
            .after(bevy_rapier3d::plugin::PhysicsSet::Writeback)
            .before(props::publish_scene_state),
    );
}

#[derive(Resource, Clone, Debug)]
pub(crate) struct StoredPhysicsConfig(pub PhysicsConfig);
