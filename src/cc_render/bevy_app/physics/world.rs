//! Static-collider world setup: invisible ground plane + invisible
//! play-volume walls. All colliders are `RigidBody::Fixed` — they only
//! receive collisions, never move.

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::cc_render::renderer::PlayVolume;

/// Resource set by `setup_world` so other systems can read the resolved
/// ground Y. May be updated by `auto_detect_ground_from_feet` once the
/// avatar's bind pose has been captured.
#[derive(Resource, Clone, Debug)]
pub(crate) struct WorldGroundY(pub f32);

/// Whether `physics::install` should auto-detect ground from foot bones
/// the moment `CC5BindRotations.captured` becomes true (and despawn /
/// respawn the static world at the discovered Y). Set when the caller
/// passes `PhysicsConfig.ground_y = None`.
#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct GroundAutoDetect(pub bool);

/// Marker for entities that compose the static world (ground + walls +
/// ceiling). Used by `auto_detect_ground_from_feet` to despawn-and-
/// respawn at the discovered Y. Spawn-time helpers below attach this.
#[derive(Component)]
pub(crate) struct StaticWorldEntity;

pub(crate) fn spawn_ground(commands: &mut Commands, ground_y: f32, half_extents_xz: f32) {
    // 50m × 50m static plane regardless of play_volume — keeps props
    // from falling through if walls are disabled.
    let plane_half = half_extents_xz.max(25.0);
    commands.spawn((
        Name::new("PhysicsGround"),
        StaticWorldEntity,
        Transform::from_xyz(0.0, ground_y, 0.0),
        RigidBody::Fixed,
        Collider::cuboid(plane_half, 0.005, plane_half),
        Friction::coefficient(0.9),
        Restitution::coefficient(0.0),
    ));
}

pub(crate) fn spawn_walls(commands: &mut Commands, ground_y: f32, pv: &PlayVolume) {
    let h = pv.half_extents_xz;
    let ceiling_y = ground_y + pv.ceiling_y;
    let mid_y = (ground_y + ceiling_y) * 0.5;
    let half_height = (ceiling_y - ground_y) * 0.5;

    // 4 walls (cuboid colliders, 5cm thick)
    let configs = [
        // (name, x, z, half_x, half_z)
        ("WallNorth", 0.0, h, h + 0.05, 0.025),
        ("WallSouth", 0.0, -h, h + 0.05, 0.025),
        ("WallEast", h, 0.0, 0.025, h + 0.05),
        ("WallWest", -h, 0.0, 0.025, h + 0.05),
    ];
    for (name, x, z, hx, hz) in configs {
        commands.spawn((
            Name::new(name),
            StaticWorldEntity,
            Transform::from_xyz(x, mid_y, z),
            RigidBody::Fixed,
            Collider::cuboid(hx, half_height, hz),
            Friction::coefficient(0.4),
            Restitution::coefficient(0.2),
        ));
    }
    // Ceiling
    commands.spawn((
        Name::new("WallCeiling"),
        StaticWorldEntity,
        Transform::from_xyz(0.0, ceiling_y, 0.0),
        RigidBody::Fixed,
        Collider::cuboid(h + 0.05, 0.025, h + 0.05),
        Friction::coefficient(0.4),
        Restitution::coefficient(0.2),
    ));
}

/// Lowest world-Y across SMPL-22 ankle/foot bones. Returns `None` if no
/// foot bone has a captured GlobalTransform yet (caller falls back to
/// the existing `ground_y`). Pure-fn over a closure that resolves bone
/// entities to GlobalTransform — keeps the system body unit-testable.
///
/// SMPL indices: 7 = L_Ankle, 8 = R_Ankle, 10 = L_Foot, 11 = R_Foot.
pub(crate) fn lowest_foot_y<F>(resolve_translation: F) -> Option<f32>
where
    F: Fn(usize) -> Option<Vec3>,
{
    const FOOT_INDICES: [usize; 4] = [7, 8, 10, 11];
    FOOT_INDICES
        .iter()
        .filter_map(|i| resolve_translation(*i).map(|v| v.y))
        .reduce(f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowest_foot_y_picks_min() {
        let translations: std::collections::HashMap<usize, Vec3> = [
            (7, Vec3::new(0.1, 0.05, 0.0)),
            (8, Vec3::new(-0.1, 0.04, 0.0)),
            (10, Vec3::new(0.1, 0.02, 0.05)),
            (11, Vec3::new(-0.1, 0.03, 0.05)),
        ]
        .into_iter()
        .collect();
        let y = lowest_foot_y(|i| translations.get(&i).copied()).unwrap();
        assert!((y - 0.02).abs() < 1e-5, "expected 0.02, got {}", y);
    }

    #[test]
    fn lowest_foot_y_returns_none_when_no_bones() {
        let y = lowest_foot_y(|_| None);
        assert!(y.is_none());
    }

    #[test]
    fn lowest_foot_y_skips_missing_bones() {
        let translations: std::collections::HashMap<usize, Vec3> =
            [(10, Vec3::new(0.0, -0.05, 0.0))].into_iter().collect();
        let y = lowest_foot_y(|i| translations.get(&i).copied()).unwrap();
        assert!((y - (-0.05)).abs() < 1e-5);
    }
}
