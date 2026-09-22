//! Visibility-graph A* pathfinder over scene-object footprints.
//!
//! For a small number of static obstacles (4-10 objects), the visibility
//! graph approach gives optimal paths in microseconds:
//!
//!   1. Inflate each obstacle's footprint by `avatar_radius`.
//!   2. Vertex set = (inflated polygon corners) ∪ (start) ∪ (goal).
//!   3. Edge between two vertices iff the connecting segment doesn't cross
//!      any inflated polygon's interior.
//!   4. A* with Euclidean heuristic on this graph.
//!
//! Output: list of `Vec3` waypoints (XZ plane, y from `floor_y`). The
//! locomotion controller follows them sequentially.
//!
//! Future: arc-smooth corners with Catmull-Rom or Bezier so the avatar
//! curves around obstacles instead of pivoting on a corner. For now we
//! emit raw polyline waypoints.

use bevy::math::Vec2;
use std::collections::BinaryHeap;
use std::fmt;

use super::metadata::{Footprint, SceneMetadata, SceneObject};

#[derive(Debug, Clone, Copy)]
pub struct Waypoint {
    pub xz: Vec2,
}

#[derive(Debug, Clone)]
pub enum PathfindError {
    StartBlocked(String),
    GoalBlocked(String),
    NoPath,
}

impl fmt::Display for PathfindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartBlocked(id) => write!(f, "start point is inside obstacle '{id}'"),
            Self::GoalBlocked(id) => write!(f, "goal point is inside obstacle '{id}'"),
            Self::NoPath => write!(f, "no path found"),
        }
    }
}

impl std::error::Error for PathfindError {}

/// Find a collision-checked route. Destination obstacles are never removed.
pub fn find_path(
    scene: &SceneMetadata,
    start_xz: Vec2,
    goal_xz: Vec2,
    _ignore_object_id: Option<&str>,
) -> Result<Vec<Waypoint>, PathfindError> {
    if !start_xz.is_finite() || !goal_xz.is_finite() {
        return Err(PathfindError::NoPath);
    }
    // Match the collision envelope. Corner vertices below reserve additional
    // room for pelvis sway: ARDY's smooth root track is not the pelvis itself.
    let inflation = scene.avatar_radius + 0.05;
    let polygons: Vec<_> = scene
        .objects
        .iter()
        .map(|o| (o.id.clone(), o.footprint.inflated_polygon(inflation)))
        .collect();
    for (id, poly) in &polygons {
        if point_in_polygon(start_xz, poly) {
            return Err(PathfindError::StartBlocked(id.clone()));
        }
    }

    // Sanity: goal not inside any remaining obstacle.
    for (id, poly) in &polygons {
        if point_in_polygon(goal_xz, poly) {
            return Err(PathfindError::GoalBlocked(id.clone()));
        }
    }

    // Build vertex set: corners + start + goal.
    let mut vertices: Vec<Vec2> = Vec::new();
    vertices.push(start_xz);
    vertices.push(goal_xz);
    for object in &scene.objects {
        vertices.extend(object.footprint.inflated_polygon(inflation + 0.20));
    }

    let n = vertices.len();
    let mut adjacency: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    for i in 0..n {
        for j in (i + 1)..n {
            if visible(vertices[i], vertices[j], &polygons) {
                let cost = (vertices[i] - vertices[j]).length();
                adjacency[i].push((j, cost));
                adjacency[j].push((i, cost));
            }
        }
    }

    // A* from vertex 0 (start) to vertex 1 (goal).
    let start = 0;
    let goal = 1;
    let path_indices = a_star(&adjacency, &vertices, start, goal).ok_or(PathfindError::NoPath)?;

    Ok(path_indices
        .into_iter()
        .map(|i| Waypoint { xz: vertices[i] })
        .collect())
}

/// Convenience: resolve a destination object id to its `stand_to_sit`
/// anchor (or footprint center if no anchor) and pathfind to it.
pub fn path_to_object(
    scene: &SceneMetadata,
    start_xz: Vec2,
    object: &SceneObject,
) -> Result<Vec<Waypoint>, PathfindError> {
    let goal_xz = object
        .anchors
        .as_ref()
        .and_then(|a| a.stand_to_sit)
        .map(|p| Vec2::new(p[0], p[2]))
        .ok_or(PathfindError::NoPath)?;
    find_path(scene, start_xz, goal_xz, None)
}

// ─── Geometry helpers ────────────────────────────────────────────────────

pub(crate) fn visible(a: Vec2, b: Vec2, polygons: &[(String, Vec<Vec2>)]) -> bool {
    if polygons
        .iter()
        .any(|(_, poly)| point_in_polygon(a, poly) || point_in_polygon(b, poly))
    {
        return false;
    }
    if (a - b).length_squared() < 1e-8 {
        return true;
    }
    let mid = (a + b) * 0.5;
    for (_, poly) in polygons {
        // Endpoint inside check (start/goal already validated, but other
        // vertices belong TO a polygon — exclude self-edges by checking the
        // midpoint, not endpoints).
        if point_in_polygon(mid, poly) {
            return false;
        }
        for i in 0..poly.len() {
            let p1 = poly[i];
            let p2 = poly[(i + 1) % poly.len()];
            // Ignore exact-shared endpoints (a = polygon corner).
            if shares_endpoint(a, b, p1, p2) {
                continue;
            }
            if segments_cross(a, b, p1, p2) {
                return false;
            }
        }
    }
    true
}

fn shares_endpoint(a: Vec2, b: Vec2, p1: Vec2, p2: Vec2) -> bool {
    let close = |x: Vec2, y: Vec2| (x - y).length_squared() < 1e-8;
    close(a, p1) || close(a, p2) || close(b, p1) || close(b, p2)
}

fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    fn ccw(p: Vec2, q: Vec2, r: Vec2) -> i32 {
        let v = (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
        if v > 1e-9 {
            1
        } else if v < -1e-9 {
            -1
        } else {
            0
        }
    }
    let d1 = ccw(c, d, a);
    let d2 = ccw(c, d, b);
    let d3 = ccw(a, b, c);
    let d4 = ccw(a, b, d);
    if d1 != d2 && d3 != d4 {
        return true;
    }
    // Collinear cases (rare with float coords; skip for now).
    false
}

fn point_in_polygon(p: Vec2, poly: &[Vec2]) -> bool {
    // Boundary is traversable: graph edges follow the inflated perimeter.
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let ab = b - a;
        let ap = p - a;
        if ab.perp_dot(ap).abs() < 1e-6
            && ap.dot(ab) >= -1e-6
            && ap.dot(ab) <= ab.length_squared() + 1e-6
        {
            return false;
        }
    }
    // Ray cast along +X, count crossings of polygon edges.
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let pi = poly[i];
        let pj = poly[j];
        let crosses = (pi.y > p.y) != (pj.y > p.y);
        if crosses {
            let t = (p.y - pi.y) / (pj.y - pi.y);
            let x_at = pi.x + t * (pj.x - pi.x);
            if p.x < x_at {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

// ─── A* ──────────────────────────────────────────────────────────────────

fn a_star(
    adj: &[Vec<(usize, f32)>],
    pts: &[Vec2],
    start: usize,
    goal: usize,
) -> Option<Vec<usize>> {
    use std::cmp::Ordering;
    #[derive(Clone, Copy)]
    struct Node(f32, usize);
    impl PartialEq for Node {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }
    impl Eq for Node {}
    impl PartialOrd for Node {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            // BinaryHeap is max-heap; invert for min-heap behavior.
            other.0.partial_cmp(&self.0)
        }
    }
    impl Ord for Node {
        fn cmp(&self, other: &Self) -> Ordering {
            self.partial_cmp(other).unwrap_or(Ordering::Equal)
        }
    }

    let n = pts.len();
    let h = |i: usize| (pts[i] - pts[goal]).length();
    let mut g_score = vec![f32::INFINITY; n];
    let mut came_from: Vec<Option<usize>> = vec![None; n];
    g_score[start] = 0.0;
    let mut open: BinaryHeap<Node> = BinaryHeap::new();
    open.push(Node(h(start), start));
    while let Some(Node(_, u)) = open.pop() {
        if u == goal {
            let mut path = vec![goal];
            let mut cur = goal;
            while let Some(prev) = came_from[cur] {
                path.push(prev);
                cur = prev;
            }
            path.reverse();
            return Some(path);
        }
        for &(v, w) in &adj[u] {
            let tentative = g_score[u] + w;
            if tentative < g_score[v] {
                g_score[v] = tentative;
                came_from[v] = Some(u);
                open.push(Node(tentative + h(v), v));
            }
        }
    }
    None
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn obstacle_at(id: &str, x: f32, z: f32, radius: f32) -> SceneObject {
        SceneObject {
            id: id.into(),
            object_type: "test".into(),
            transform: super::super::metadata::ObjectTransform {
                position: [x, 0.0, z],
                yaw_deg: 0.0,
            },
            world_aabb: super::super::metadata::Aabb {
                min: [x - radius, 0.0, z - radius],
                max: [x + radius, 1.0, z + radius],
            },
            footprint: Footprint::Circle {
                center_xz: [x, z],
                radius,
            },
            anchors: None,
            sit_anchor_source: None,
        }
    }

    #[test]
    fn straight_line_when_clear() {
        let scene = SceneMetadata {
            schema_version: 1,
            floor_y: 0.0,
            avatar_radius: 0.30,
            objects: vec![],
            spawn_points: vec![],
        };
        let path = find_path(&scene, Vec2::new(-2.0, 0.0), Vec2::new(2.0, 0.0), None).unwrap();
        assert_eq!(path.len(), 2);
    }

    #[test]
    fn submillimeter_motion_cannot_cross_or_remain_inside_obstacle() {
        let polygon = vec![("desk".into(), vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y])];
        assert!(!visible(
            Vec2::new(-0.00001, 0.5),
            Vec2::new(0.00001, 0.5),
            &polygon
        ));
        assert!(!visible(Vec2::splat(0.5), Vec2::splat(0.5), &polygon));
        assert!(visible(
            Vec2::new(-0.00001, 0.5),
            Vec2::new(-0.00002, 0.5),
            &polygon
        ));
    }

    #[test]
    fn detours_around_obstacle() {
        let scene = SceneMetadata {
            schema_version: 1,
            floor_y: 0.0,
            avatar_radius: 0.30,
            objects: vec![obstacle_at("blocker", 0.0, 0.0, 0.5)],
            spawn_points: vec![],
        };
        let path = find_path(&scene, Vec2::new(-2.0, 0.0), Vec2::new(2.0, 0.0), None).unwrap();
        assert!(
            path.len() > 2,
            "expected detour around obstacle, got {} waypoints",
            path.len()
        );
    }
}
