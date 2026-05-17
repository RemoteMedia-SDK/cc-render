//! Prop lifecycle: drains spawn/despawn cmds from the node side,
//! manages dynamic colliders, publishes `SceneState` each frame.

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::{mpsc, watch};

use crate::cc_render::renderer::{
    GrabFailure, GrabOrRelease, GrabSnapshot, GraspAnchor, Hand, PropCmd, PropId, PropShape,
    PropSnapshot, SceneState,
};

#[derive(Resource)]
pub(crate) struct PropCmdRx(pub Mutex<mpsc::UnboundedReceiver<PropCmd>>);

#[derive(Resource)]
pub(crate) struct DespawnCmdRx(pub Mutex<mpsc::UnboundedReceiver<PropId>>);

#[derive(Resource)]
pub(crate) struct GrabCmdRx(pub Mutex<mpsc::UnboundedReceiver<GrabOrRelease>>);

#[derive(Resource)]
pub(crate) struct SceneStateTx(pub watch::Sender<SceneState>);

#[derive(Component, Debug, Clone)]
pub(crate) struct Prop {
    pub id: PropId,
    pub loading: bool,
    /// Grasp anchor in prop local space, as a 4x4 (column-major).
    #[allow(dead_code)]
    pub grasp_local: Option<[[f32; 4]; 4]>,
}

/// Tag on a Prop whose visual GLB is still loading. Holds the scene
/// `Handle<Scene>` so `finalize_meshglb_collider` can poll
/// `recursive_dependency_load_state` and only swap the placeholder ball
/// once every transitive mesh is on disk and parsed.
#[derive(Component, Debug, Clone)]
pub(crate) struct MeshGlbColliderPending {
    pub scene_handle: Handle<Scene>,
}

#[derive(Resource, Default, Debug)]
pub(crate) struct PropRegistry {
    pub by_id: HashMap<PropId, Entity>,
    pub pending_failures: Vec<GrabFailure>,
}

pub(crate) fn handle_prop_cmds(
    mut commands: Commands,
    spawn_rx: Option<Res<PropCmdRx>>,
    despawn_rx: Option<Res<DespawnCmdRx>>,
    mut registry: ResMut<PropRegistry>,
    mut fsms: ResMut<super::grab::GrabFsms>,
    asset_server: Res<AssetServer>,
) {
    // Channels are inserted only when the parent thread wires them
    // (i.e. `bevy_app::run` saw `RendererConfig::physics = Some(_)` AND
    // the caller threaded the receivers through). Standalone tests
    // that just call `bevy_app_install_physics` without channels are
    // valid no-ops.
    if let Some(spawn_rx) = spawn_rx.as_ref() {
        let mut s_rx = spawn_rx.0.lock().unwrap();
        while let Ok(p) = s_rx.try_recv() {
            spawn_prop(&mut commands, &mut registry, &mut fsms, &asset_server, p);
        }
    }
    if let Some(despawn_rx) = despawn_rx.as_ref() {
        let mut d_rx = despawn_rx.0.lock().unwrap();
        while let Ok(id) = d_rx.try_recv() {
            despawn_prop(&mut commands, &mut registry, &id);
        }
    }
}

/// Returns the hand currently attached to `target`, if any. Used to
/// determine whether a `spawn_prop` replacement should emit a real
/// `prop_replaced` failure (and on which hand) versus silently
/// replacing an unheld prop.
fn attached_hand_for(fsms: &super::grab::GrabFsms, target: &PropId) -> Option<Hand> {
    if let super::grab::GrabState::Attached { target: t, .. } = &fsms.left {
        if t == target {
            return Some(Hand::Left);
        }
    }
    if let super::grab::GrabState::Attached { target: t, .. } = &fsms.right {
        if t == target {
            return Some(Hand::Right);
        }
    }
    None
}

fn spawn_prop(
    commands: &mut Commands,
    registry: &mut PropRegistry,
    fsms: &mut super::grab::GrabFsms,
    asset_server: &AssetServer,
    p: PropCmd,
) {
    if let Some(existing) = registry.by_id.remove(&p.id) {
        // Bevy 0.15: despawn() is recursive by default.
        commands.entity(existing).despawn();
        tracing::warn!(
            target: "cc_render",
            "physics: prop id={} replaced (existing entity despawned)",
            p.id,
        );
        // Only emit `prop_replaced` failure if a hand was actually
        // grabbing this prop — and reset that hand's FSM to Idle since
        // its joint partner just got despawned. (No need to remove the
        // ImpulseJoint; it's on the despawned entity.)
        if let Some(hand) = attached_hand_for(fsms, &p.id) {
            registry.pending_failures.push(GrabFailure {
                hand,
                target: p.id.clone(),
                reason: "prop_replaced".into(),
            });
            match hand {
                Hand::Left => fsms.left = super::grab::GrabState::Idle,
                Hand::Right => fsms.right = super::grab::GrabState::Idle,
            }
        }
    }

    let log_id = p.id.clone();
    let log_mass = p.mass_kg;
    let log_shape = format!("{:?}", &p.shape);

    let mut xform = transform_from_4x4(p.initial_transform);
    let mut mesh_pending: Option<(Handle<Scene>, f32)> = None;
    let collider = match &p.shape {
        PropShape::Sphere { radius } => Collider::ball(*radius),
        PropShape::Box { half_extents } => {
            Collider::cuboid(half_extents[0], half_extents[1], half_extents[2])
        }
        PropShape::Capsule {
            radius,
            half_height,
        } => Collider::capsule_y(*half_height, *radius),
        PropShape::MeshGlb { path, scale } => {
            // Apply requested scale to parent transform; the visual scene
            // child inherits it via GlobalTransform. Collider is sized in
            // parent-local space, so `finalize_meshglb_collider` divides
            // out this scale when computing local half-extents.
            xform.scale *= *scale;
            // AssetServer interprets the path against the default
            // asset source root (set in `bevy_app::run` to the avatar's
            // GLB dir). Relative paths under that dir Just Work; absolute
            // paths and paths outside the source root will fail to load,
            // in which case `finalize_meshglb_collider` never fires and
            // the prop keeps the placeholder ball + `loading=true`.
            let scene_path = format!("{}#Scene0", path.to_string_lossy());
            let handle: Handle<Scene> = asset_server.load(&scene_path);
            mesh_pending = Some((handle, *scale));
            // Placeholder while the scene streams in. Small ball at the
            // origin keeps the body collidable so it doesn't fall through
            // the ground during the load window.
            Collider::ball(0.05)
        }
    };
    let loading = mesh_pending.is_some();
    let grasp_local = p.grasp.as_ref().map(grasp_to_4x4);

    // Bevy 0.15: spawn Transform directly; GlobalTransform is auto-required.
    // bevy_rapier3d 0.28: AdditionalMassProperties::Mass overrides the
    // collider-density-derived mass for dynamic bodies.
    let mut entity_cmds = commands.spawn((
        Name::new(format!("Prop_{}", p.id)),
        Prop {
            id: p.id.clone(),
            loading,
            grasp_local,
        },
        xform,
        Visibility::default(),
        RigidBody::Dynamic,
        collider,
        AdditionalMassProperties::Mass(p.mass_kg.max(1e-3)),
        Friction::coefficient(p.friction),
        Restitution::coefficient(p.restitution),
        Velocity::default(),
    ));

    if let Some((scene_handle, _scale)) = mesh_pending.as_ref() {
        entity_cmds.insert(MeshGlbColliderPending {
            scene_handle: scene_handle.clone(),
        });
        // SceneRoot child carries the visible mesh; the parent owns
        // RigidBody so child mesh entities' colliders (if any) attach
        // to it via Rapier's nearest-rigid-body-ancestor rule. We don't
        // use AsyncSceneCollider here — instead `finalize_meshglb_collider`
        // computes a single tight cuboid from the mesh AABBs once
        // loaded, which keeps mass/inertia predictable.
        entity_cmds.with_children(|parent| {
            parent.spawn((
                Name::new("PropScene"),
                SceneRoot(scene_handle.clone()),
                Transform::default(),
                Visibility::default(),
            ));
        });
    }

    let e = entity_cmds.id();
    registry.by_id.insert(p.id, e);

    tracing::info!(
        target: "cc_render",
        "physics: spawned prop id={} shape={} mass={}kg loading={}",
        log_id, log_shape, log_mass, loading,
    );
}

/// Once the prop's child SceneRoot has fully loaded (every mesh +
/// material + texture in the GLB is on the GPU), walk descendants to
/// build a single combined world-space AABB, convert it to parent-local
/// space, and replace the placeholder ball collider with a tight
/// cuboid. Clears `Prop.loading` and removes the `MeshGlbColliderPending`
/// tag so the system goes back to idle.
///
/// Why a single cuboid (not `AsyncSceneCollider`'s per-mesh ConvexHull)?
/// Per-mesh colliders fight `AdditionalMassProperties::Mass` (each
/// child contributes its own density-derived mass on top of the
/// caller's authored value), and a complex GLB can spawn dozens of
/// child colliders that hammer broadphase. A single cuboid gives
/// stable mass and 1-shape broadphase cost; if a future caller needs
/// finer collision geometry they can author it via the sidecar.
pub(crate) fn finalize_meshglb_collider(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut props: Query<(
        Entity,
        &mut Prop,
        &MeshGlbColliderPending,
        &Children,
        &GlobalTransform,
    )>,
    children_query: Query<&Children>,
    mesh_query: Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
) {
    for (prop_e, mut prop, pending, children, prop_gt) in props.iter_mut() {
        // Wait for every transitive dependency of the scene asset to be
        // loaded. `Loaded` matches the gate used by `mark_ready_when_settled`.
        let load_state = asset_server.recursive_dependency_load_state(pending.scene_handle.id());
        if !matches!(
            load_state,
            bevy::asset::RecursiveDependencyLoadState::Loaded
        ) {
            continue;
        }

        // Walk descendants for entities with a Mesh3d + a render Aabb.
        // Bevy attaches `bevy::render::primitives::Aabb` automatically
        // to every mesh entity once the mesh asset has been parsed.
        let mut stack: Vec<Entity> = children.iter().copied().collect();
        let mut world_min = Vec3::splat(f32::INFINITY);
        let mut world_max = Vec3::splat(f32::NEG_INFINITY);
        let mut mesh_count = 0;
        while let Some(e) = stack.pop() {
            if let Ok((mesh_gt, aabb)) = mesh_query.get(e) {
                let center = Vec3::from(aabb.center);
                let half = Vec3::from(aabb.half_extents);
                let m = mesh_gt.compute_matrix();
                for sx in [-1.0_f32, 1.0] {
                    for sy in [-1.0_f32, 1.0] {
                        for sz in [-1.0_f32, 1.0] {
                            let local = center + Vec3::new(sx * half.x, sy * half.y, sz * half.z);
                            let world = m.transform_point3(local);
                            world_min = world_min.min(world);
                            world_max = world_max.max(world);
                        }
                    }
                }
                mesh_count += 1;
            }
            if let Ok(grandkids) = children_query.get(e) {
                stack.extend(grandkids.iter().copied());
            }
        }

        if mesh_count == 0 {
            // Loaded but no Mesh3d descendants — empty scene. Not
            // worth retrying; keep the placeholder + loading flag so
            // downstream consumers know the prop didn't materialize.
            tracing::warn!(
                target: "cc_render",
                "physics: prop {} GLB loaded with zero meshes; \
                 keeping placeholder ball collider",
                prop.id,
            );
            commands.entity(prop_e).remove::<MeshGlbColliderPending>();
            continue;
        }

        // World→parent-local. Parent's GlobalTransform absorbs the
        // user's `initial_transform` AND the per-prop `scale` (set in
        // spawn_prop). Dividing world half-extents by parent scale
        // yields the right local cuboid size; multiplying by the
        // parent's inverse positions the cuboid center at the mesh's
        // visual center even when meshes are off-axis.
        let parent_inv = prop_gt.compute_matrix().inverse();
        let world_center = (world_min + world_max) * 0.5;
        let world_half = (world_max - world_min) * 0.5;
        let local_center = parent_inv.transform_point3(world_center);
        let parent_scale = prop_gt.compute_transform().scale.abs();
        let safe_scale = parent_scale.max(Vec3::splat(1e-6));
        let local_half = world_half / safe_scale;

        // Use `Collider::compound` only when the local cuboid center is
        // off the prop origin — a centered shape doesn't need the extra
        // wrapper allocation.
        let new_collider = if local_center.length_squared() < 1e-8 {
            Collider::cuboid(local_half.x, local_half.y, local_half.z)
        } else {
            Collider::compound(vec![(
                local_center,
                Quat::IDENTITY,
                Collider::cuboid(local_half.x, local_half.y, local_half.z),
            )])
        };

        // Replacing the existing Collider component overwrites it
        // in-place; bevy_rapier's collider sync system picks up the
        // change next tick.
        commands.entity(prop_e).insert(new_collider);
        commands.entity(prop_e).remove::<MeshGlbColliderPending>();
        prop.loading = false;
        tracing::info!(
            target: "cc_render",
            "physics: finalized MeshGlb collider for prop {} \
             ({} meshes, half_extents={:?}, center_offset={:?})",
            prop.id, mesh_count, local_half, local_center,
        );
    }
}

fn despawn_prop(commands: &mut Commands, registry: &mut PropRegistry, id: &PropId) {
    if let Some(e) = registry.by_id.remove(id) {
        commands.entity(e).despawn();
        tracing::info!(target: "cc_render", "physics: despawned prop id={}", id);
    } else {
        tracing::debug!(target: "cc_render", "physics: despawn_prop id={} not found", id);
    }
}

pub(crate) fn publish_scene_state(
    tx: Option<Res<SceneStateTx>>,
    mut registry: ResMut<PropRegistry>,
    props: Query<(&Prop, &GlobalTransform, Option<&Collider>)>,
    fsms: Res<super::grab::GrabFsms>,
    last_pts: Option<Res<crate::cc_render::bevy_app::capture::LastAppliedPts>>,
) {
    // No tx ⇒ caller didn't wire scene_state publishing; nothing to do.
    let Some(tx) = tx else { return };
    // Skip the publish entirely when there's nothing to report. This is
    // the steady state before any prop is spawned (and after the LLM
    // has finished playing with the scene): no allocations, no watch
    // wakeups for downstream consumers.
    let any_props = props.iter().next().is_some();
    let any_grabs = matches!(fsms.left, super::grab::GrabState::Attached { .. })
        || matches!(fsms.right, super::grab::GrabState::Attached { .. });
    let any_failures = !registry.pending_failures.is_empty();
    if !any_props && !any_grabs && !any_failures {
        return;
    }
    let mut snapshots = Vec::new();
    for (prop, gt, collider) in props.iter() {
        let t = gt.compute_transform();
        let mat = transform_to_4x4(&t);
        let (aabb_min, aabb_max) = collider
            .map(|c| world_aabb_from_collider(c, gt))
            .unwrap_or(([0.0; 3], [0.0; 3]));
        snapshots.push(PropSnapshot {
            id: prop.id.clone(),
            transform: mat,
            aabb_min,
            aabb_max,
            grasp_world: prop.grasp_local.map(|local| mat_mul_4x4(mat, local)),
            loading: prop.loading,
        });
    }
    let mut grabs = Vec::new();
    if let super::grab::GrabState::Attached { target, .. } = &fsms.left {
        grabs.push(GrabSnapshot {
            hand: Hand::Left,
            prop: target.clone(),
        });
    }
    if let super::grab::GrabState::Attached { target, .. } = &fsms.right {
        grabs.push(GrabSnapshot {
            hand: Hand::Right,
            prop: target.clone(),
        });
    }
    let failed = std::mem::take(&mut registry.pending_failures);
    let pts_ms = last_pts.as_ref().map(|p| p.0).unwrap_or(0);
    let state = SceneState {
        props: snapshots,
        grabs,
        failed_grabs: failed,
        pts_ms,
    };
    let _ = tx.0.send(state);
}

fn transform_from_4x4(m: [[f32; 4]; 4]) -> Transform {
    let mat = Mat4::from_cols_array_2d(&m);
    let (s, r, t) = mat.to_scale_rotation_translation();
    Transform {
        translation: t,
        rotation: r,
        scale: s,
    }
}

fn transform_to_4x4(t: &Transform) -> [[f32; 4]; 4] {
    Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.translation).to_cols_array_2d()
}

fn grasp_to_4x4(g: &GraspAnchor) -> [[f32; 4]; 4] {
    let q = Quat::from_xyzw(
        g.local_rotation[0],
        g.local_rotation[1],
        g.local_rotation[2],
        g.local_rotation[3],
    );
    let t = Vec3::from(g.local_offset);
    Mat4::from_rotation_translation(q, t).to_cols_array_2d()
}

fn mat_mul_4x4(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    (Mat4::from_cols_array_2d(&a) * Mat4::from_cols_array_2d(&b)).to_cols_array_2d()
}

/// World-space AABB for a prop. Walks the 8 corners of the collider's
/// local AABB through the prop's full GlobalTransform (covers rotation
/// + non-uniform scale, which `Aabb::transform_by(&Isometry)` would
/// drop).
fn world_aabb_from_collider(collider: &Collider, gt: &GlobalTransform) -> ([f32; 3], [f32; 3]) {
    let local = collider.raw.compute_local_aabb();
    let m = gt.compute_matrix();
    let mut mn = Vec3::splat(f32::INFINITY);
    let mut mx = Vec3::splat(f32::NEG_INFINITY);
    for v in local.vertices().iter() {
        let world = m.transform_point3(Vec3::new(v.x, v.y, v.z));
        mn = mn.min(world);
        mx = mx.max(world);
    }
    (mn.to_array(), mx.to_array())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_aabb_ball_at_origin_unit_scale() {
        let collider = Collider::ball(0.1);
        let gt = GlobalTransform::from_translation(Vec3::ZERO);
        let (mn, mx) = world_aabb_from_collider(&collider, &gt);
        assert!((mn[0] - -0.1).abs() < 1e-5);
        assert!((mx[0] - 0.1).abs() < 1e-5);
        assert!((mn[1] - -0.1).abs() < 1e-5);
        assert!((mx[2] - 0.1).abs() < 1e-5);
    }

    #[test]
    fn world_aabb_ball_translated() {
        let collider = Collider::ball(0.05);
        let gt = GlobalTransform::from_translation(Vec3::new(1.0, 2.0, -3.0));
        let (mn, mx) = world_aabb_from_collider(&collider, &gt);
        assert!((mn[0] - 0.95).abs() < 1e-5);
        assert!((mx[0] - 1.05).abs() < 1e-5);
        assert!((mn[1] - 1.95).abs() < 1e-5);
        assert!((mn[2] - -3.05).abs() < 1e-5);
    }

    #[test]
    fn world_aabb_box_rotated_grows_extents() {
        // Unit cube rotated 45° about Y: world AABB should be sqrt(2)
        // wide in X and Z (~1.414), still 1.0 tall in Y.
        let collider = Collider::cuboid(0.5, 0.5, 0.5);
        let mut t = Transform::default();
        t.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let gt = GlobalTransform::from(t);
        let (mn, mx) = world_aabb_from_collider(&collider, &gt);
        let sqrt2_half = std::f32::consts::SQRT_2 / 2.0;
        assert!(
            (mx[0] - sqrt2_half).abs() < 1e-4,
            "x max {} vs {}",
            mx[0],
            sqrt2_half
        );
        assert!((mn[0] + sqrt2_half).abs() < 1e-4);
        assert!((mx[1] - 0.5).abs() < 1e-5);
        assert!((mn[1] + 0.5).abs() < 1e-5);
    }
}
