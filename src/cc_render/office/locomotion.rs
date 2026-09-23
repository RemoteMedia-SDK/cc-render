//! Continuous collision-checked trajectories; body articulation belongs to ARDY.
use super::{metadata::SceneMetadata, pathfinder::{visible, Waypoint}};
use bevy::math::Vec2;

/// Maximum arrival bearing error, measured from forward to the interaction target.
pub(super) fn arrival_angle() -> f32 {
    std::env::var("OFFICE_ARRIVAL_MAX_ANGLE_DEG").ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && (5. ..=80.).contains(v))
        .unwrap_or(35.).to_radians()
}

pub(super) fn target_direction(object: &super::OfficeObject) -> Vec2 {
    let goal = object.approach_position();
    let focus = object.grasp_transform().map_or(bevy::math::Vec3::from(object.position), |t| t.translation);
    Vec2::new(focus.x - goal[0], focus.z - goal[2]).normalize_or_zero()
}

pub(super) fn within_arrival_cone(direction: Vec2, facing: Vec2, angle: f32) -> bool {
    direction.length_squared() < 1e-6 || facing.length_squared() < 1e-6
        || direction.normalize().dot(facing.normalize()) >= angle.cos()
}

/// Share arrival geometry between preview queries and executed navigation.
/// Keep already suitable routes; otherwise route through a collision-clear lead-in.
pub(super) fn approach_path(scene: &SceneMetadata, start: Vec2, goal: Vec2,
    facing: Option<Vec2>, angle: f32) -> Result<Vec<Waypoint>, super::pathfinder::PathfindError> {
    use super::pathfinder::{find_path, PathfindError};
    let direct = find_path(scene, start, goal, None)?;
    let Some(facing) = facing.filter(|v| v.length_squared() > 1e-6) else { return Ok(direct); };
    let direction = direct.iter().rev().find(|p| p.xz.distance(goal) > 0.001)
        .map_or(Vec2::ZERO, |p| goal - p.xz);
    if within_arrival_cone(direction, facing, angle) { return Ok(direct); }
    let polygons: Vec<_> = scene.objects.iter().map(|o|
        (o.id.clone(), o.footprint.inflated_polygon(scene.avatar_radius + 0.05))).collect();
    let mut best: Option<(f32, Vec<Waypoint>)> = None;
    for length in [1.2, 0.9, 0.7, 0.4, 0.2] {
        for offset in [0., -angle * 0.5, angle * 0.5, -angle, angle] {
            let (sin, cos) = offset.sin_cos();
            let heading = Vec2::new(facing.x * cos - facing.y * sin, facing.x * sin + facing.y * cos);
            let entry = goal - heading * length;
            if !visible(entry, goal, &polygons) { continue; }
            let Ok(mut path) = find_path(scene, start, entry, None) else { continue; };
            path.push(Waypoint { xz: goal });
            let cost: f32 = path.windows(2).map(|p| p[0].xz.distance(p[1].xz)).sum();
            if best.as_ref().is_none_or(|(previous, _)| cost < *previous) { best = Some((cost, path)); }
        }
        // Preserve a useful moving turn; shorten only when clearance requires it.
        if best.is_some() { break; }
    }
    best.map(|(_, path)| path).ok_or(PathfindError::NoPath)
}

pub(super) fn walking(scene: &SceneMetadata, path: &[Waypoint], mut time: f32) -> Vec<[f32; 4]> {
    let points = rounded_path(scene, path);
    let mut roots = Vec::with_capacity(points.len());
    for (i, point) in points.iter().enumerate() {
        if i > 0 { time += point.distance(points[i - 1]) / 0.8; }
        roots.push([point.x, 0.95, point.y, time]);
    }
    roots
}

fn rounded_path(scene: &SceneMetadata, path: &[Waypoint]) -> Vec<Vec2> {
    let Some(first) = path.first() else { return vec![]; };
    let polygons: Vec<_> = scene.objects.iter().map(|o|
        (o.id.clone(), o.footprint.inflated_polygon(scene.avatar_radius + 0.05))).collect();
    let mut result = vec![first.xz];
    for triple in path.windows(3) {
        let [a, b, c] = [triple[0].xz, triple[1].xz, triple[2].xz];
        let radius = 0.5_f32.min(a.distance(b) * 0.35).min(b.distance(c) * 0.35);
        let entry = b + (a - b).normalize_or_zero() * radius;
        let exit = b + (c - b).normalize_or_zero() * radius;
        let arc: Vec<_> = (0..=12).map(|i| {
            let t = i as f32 / 12.;
            entry * (1. - t).powi(2) + b * (2. * t * (1. - t)) + exit * t * t
        }).collect();
        // Check every emitted segment with the same inflated footprint as A*.
        // A tight doorway keeps its original corner instead of cutting through it.
        if visible(*result.last().unwrap(), entry, &polygons)
            && arc.windows(2).all(|pair| visible(pair[0], pair[1], &polygons))
            && visible(exit, c, &polygons)
        {
            result.extend(arc);
        } else {
            result.push(b);
        }
    }
    if path.len() > 1 { result.push(path.last().unwrap().xz); }
    result.dedup_by(|a, b| a.distance(*b) < 1e-5);
    result
}

pub(super) fn reach_duration(origin: [f32; 3], goal: [f32; 3]) -> f32 {
    // Ease out from walking speed, rather than inserting a stationary align phase.
    (2. * Vec2::new(origin[0] - goal[0], origin[2] - goal[2]).length() / 0.8).max(1.2)
}

pub(super) fn reaching(origin: [f32; 3], goal: [f32; 3], elapsed: f32) -> Vec<[f32; 4]> {
    let duration = reach_duration(origin, goal);
    (0..=12).map(|i| {
        let t = i as f32 / 12.;
        let progress = t * (2. - t);
        [origin[0] + (goal[0] - origin[0]) * progress, origin[1],
            origin[2] + (goal[2] - origin[2]) * progress, duration * t - elapsed]
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::{scene_metadata, starter_objects};

    #[test]
    fn side_arrivals_are_rerouted_but_front_arrivals_stay_direct() {
        let scene = scene_metadata(&[]);
        let angle = 35_f32.to_radians();
        for start in [Vec2::new(-3., 0.), Vec2::new(0., 3.)] {
            let path = approach_path(&scene, start, Vec2::ZERO, Some(Vec2::Y), angle).unwrap();
            assert!(path.len() > 2);
            let roots = walking(&scene, &path, 0.);
            let last = &roots[roots.len()-2..];
            let direction = Vec2::new(last[1][0]-last[0][0], last[1][2]-last[0][2]);
            assert!(within_arrival_cone(direction, Vec2::Y, angle + 1e-5));
            assert!(roots.windows(2).all(|p| p[1][3] > p[0][3]));
        }
        let front = approach_path(&scene, -Vec2::Y * 3., Vec2::ZERO, Some(Vec2::Y), angle).unwrap();
        assert_eq!(front.len(), 2);
        let unspecified = approach_path(&scene, -Vec2::X * 3., Vec2::ZERO, None, angle).unwrap();
        assert_eq!(unspecified.len(), 2);
    }

    #[test]
    fn home_approaches_preserve_collision_clearance_and_arrival_cone() {
        let objects = starter_objects();
        let scene = scene_metadata(&objects);
        let angle = 35_f32.to_radians();
        let polygons: Vec<_> = scene.objects.iter().map(|o|
            (o.id.clone(), o.footprint.inflated_polygon(scene.avatar_radius + 0.05))).collect();
        for object in objects.iter().filter(|o| !o.id.starts_with("wall_")) {
            let p = object.approach_position();
            let path = approach_path(&scene, Vec2::ZERO, Vec2::new(p[0], p[2]),
                Some(target_direction(object)), angle).unwrap();
            let roots = walking(&scene, &path, 0.);
            for pair in roots.windows(2) {
                assert!(visible(Vec2::new(pair[0][0], pair[0][2]), Vec2::new(pair[1][0], pair[1][2]), &polygons));
            }
            let end = &roots[roots.len()-2..];
            assert!(within_arrival_cone(Vec2::new(end[1][0]-end[0][0], end[1][2]-end[0][2]),
                target_direction(object), angle + 1e-5), "{}", object.id);
        }
    }

    #[test]
    fn rounded_home_routes_keep_clearance_and_never_insert_a_pivot() {
        let scene = scene_metadata(&starter_objects());
        let start = Vec2::new(0., 0.2);
        for object in starter_objects() {
            let goal = object.approach_position();
            let Ok(path) = super::super::pathfinder::find_path(&scene, start, Vec2::new(goal[0], goal[2]), None) else { continue; };
            let roots = walking(&scene, &path, 0.);
            let polygons: Vec<_> = scene.objects.iter().map(|o|
                (o.id.clone(), o.footprint.inflated_polygon(scene.avatar_radius + 0.05))).collect();
            for pair in roots.windows(2) {
                assert!(pair[1][3] > pair[0][3]);
                let a = Vec2::new(pair[0][0], pair[0][2]);
                let b = Vec2::new(pair[1][0], pair[1][2]);
                assert!(a.distance(b) > 0.);
                assert!(visible(a, b, &polygons), "unsafe curve toward {}", object.id);
            }
        }
    }

    #[test]
    fn reach_moves_immediately_then_decelerates_without_forcing_a_crouch() {
        let roots = reaching([0., 0.95, 0.], [0.64, 0., 0.], 0.);
        assert!(roots[1][0] > 0.);
        assert_eq!(roots.last().unwrap()[0], 0.64);
        assert!(roots.iter().all(|r| r[1] == 0.95));
        let speeds: Vec<_> = roots.windows(2).map(|p| (p[1][0] - p[0][0]) / (p[1][3] - p[0][3])).collect();
        assert!(speeds.windows(2).all(|p| p[1] < p[0]));
    }
}
