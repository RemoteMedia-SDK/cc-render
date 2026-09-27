//! Geometry and observed-state bounds for a single floor-level furniture push.
use super::{Active, Observation, OfficeObject};
use crate::cc_render::renderer::SceneState;
use bevy::prelude::*;

#[derive(Clone, Debug)]
pub(super) struct PushPlan {
    pub id: String,
    pub goal: Vec3,
    pub direction: Vec3,
    pub contact_offset: Vec3,
    pub stage_offset: Vec3,
    pub start: Vec3,
}

/// Read-only furniture-center route toward a named room. This establishes
/// geometric feasibility, not an executable long push: each turn still needs
/// an observed contact-side change and avatar swept-clearance check.
pub(super) fn room_route(
    object: &OfficeObject,
    room_id: &str,
    others: &[OfficeObject],
) -> Result<Vec<Vec3>, String> {
    let room = super::super::home::ROOMS
        .iter()
        .find(|r| r.id == room_id)
        .ok_or_else(|| format!("unknown room: {room_id}"))?;
    if !object.dynamic || !object.pushable {
        return Err("target is not pushable furniture".into());
    }
    let goal = Vec3::new(room.entrance[0], object.position[1], room.entrance[1]);
    let half = (Vec3::from(object.metadata().world_aabb.max)
        - Vec3::from(object.metadata().world_aabb.min))
        * 0.5;
    if goal.x - half.x <= room.min_xz[0]
        || goal.x + half.x >= room.max_xz[0]
        || goal.z - half.z <= room.min_xz[1]
        || goal.z + half.z >= room.max_xz[1]
    {
        return Err("furniture cannot fit fully inside the requested room".into());
    }
    // The avatar visibility graph uses obstacle-corner vertices and cannot
    // express the chair's tight route around the desk and through the door.
    // Search the known home floor with a conservative circular chair envelope.
    // This is *only* a geometric preview: a push executor must also reserve
    // space behind each segment for the avatar and re-establish hand contact.
    grid_furniture_route(
        object,
        goal,
        Vec2::new(half.x, half.z).length() + 0.04,
        others,
    )
}

fn grid_furniture_route(
    object: &OfficeObject,
    goal: Vec3,
    radius: f32,
    others: &[OfficeObject],
) -> Result<Vec<Vec3>, String> {
    use std::{cmp::Reverse, collections::BinaryHeap};
    const STEP: f32 = 0.1;
    const WIDTH: usize = 101; // X: -4..6 m
    const HEIGHT: usize = 121; // Z: -9..3 m
    let id = |x: usize, z: usize| z * WIDTH + x;
    let cell = |p: Vec3| -> Option<(usize, usize)> {
        let x = ((p.x + 4.) / STEP).round() as isize;
        let z = ((p.z + 9.) / STEP).round() as isize;
        (x >= 0 && x < WIDTH as isize && z >= 0 && z < HEIGHT as isize)
            .then_some((x as usize, z as usize))
    };
    let chair_box = object.metadata().world_aabb;
    let obstacles: Vec<_> = others
        .iter()
        .filter(|o| o.id != object.id)
        .map(OfficeObject::metadata)
        .map(|o| o.world_aabb)
        .filter(|b| b.max[1] > chair_box.min[1] + 0.02 && b.min[1] < chair_box.max[1] - 0.02)
        .collect();
    let blocked = |x: usize, z: usize| {
        let p = Vec2::new(-4. + x as f32 * STEP, -9. + z as f32 * STEP);
        p.x < -4. + radius
            || p.x > 6. - radius
            || p.y < -9. + radius
            || p.y > 3. - radius
            || obstacles.iter().any(|b| {
                let dx = (b.min[0] - p.x).max(0.).max(p.x - b.max[0]);
                let dz = (b.min[2] - p.y).max(0.).max(p.y - b.max[2]);
                dx * dx + dz * dz < radius * radius
            })
    };
    let start = cell(Vec3::from(object.position)).ok_or("furniture outside home floor")?;
    let end = cell(goal).ok_or("room entrance outside home floor")?;
    if blocked(start.0, start.1) || blocked(end.0, end.1) {
        return Err("furniture start or room entrance is obstructed".into());
    }
    let goal_id = id(end.0, end.1);
    let mut distance = vec![u32::MAX; WIDTH * HEIGHT];
    let mut parent = vec![usize::MAX; WIDTH * HEIGHT];
    let mut queue = BinaryHeap::new();
    let start_id = id(start.0, start.1);
    distance[start_id] = 0;
    queue.push(Reverse((0_u32, start_id)));
    while let Some(Reverse((_, current))) = queue.pop() {
        if current == goal_id {
            break;
        }
        let x = current % WIDTH;
        let z = current / WIDTH;
        for (dx, dz) in [
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1),
        ] {
            let nx = x as isize + dx;
            let nz = z as isize + dz;
            if nx < 0 || nz < 0 || nx >= WIDTH as isize || nz >= HEIGHT as isize {
                continue;
            }
            let (nx, nz) = (nx as usize, nz as usize);
            if blocked(nx, nz) || (dx != 0 && dz != 0 && (blocked(nx, z) || blocked(x, nz))) {
                continue;
            }
            let next = id(nx, nz);
            let step = if dx != 0 && dz != 0 { 1414 } else { 1000 };
            let cost = distance[current].saturating_add(step);
            if cost >= distance[next] {
                continue;
            }
            distance[next] = cost;
            parent[next] = current;
            let hx = (end.0 as i32 - nx as i32) as f32;
            let hz = (end.1 as i32 - nz as i32) as f32;
            queue.push(Reverse((cost + (hx.hypot(hz) * 1000.) as u32, next)));
        }
    }
    if parent[goal_id] == usize::MAX && goal_id != start_id {
        return Err("no swept-clear furniture route to room".into());
    }
    let mut route = vec![goal];
    let mut current = goal_id;
    while current != start_id {
        current = parent[current];
        route.push(Vec3::new(
            -4. + (current % WIDTH) as f32 * STEP,
            goal.y,
            -9. + (current / WIDTH) as f32 * STEP,
        ));
    }
    route.reverse();
    route[0] = Vec3::from(object.position);
    Ok(route)
}

pub(super) fn long_probe_enabled() -> bool {
    std::env::var("OFFICE_PUSH_LONG_EXPERIMENT").as_deref() == Ok("1")
        && std::env::var("OFFICE_PUSH_WAYPOINT_GAIT").as_deref() == Ok("1")
}

pub(super) fn hand_constraint_times(total_ms: u64, continuous: bool) -> Vec<u64> {
    if !continuous { return vec![total_ms]; }
    // Sparse positions across the same absolute-time stroke guide the palms
    // without imposing a full-body pose or restarting the prompt at a marker.
    let mut times: Vec<_> = (1600..total_ms).step_by(1200).collect();
    times.push(total_ms);
    times
}

pub(super) fn prepare(
    object: &OfficeObject,
    goal: [f32; 3],
    others: &[OfficeObject],
) -> Result<PushPlan, String> {
    if !object.dynamic || !object.pushable {
        return Err("target is not pushable furniture".into());
    }
    let start = Vec3::from(object.position);
    let goal = Vec3::from(goal);
    let delta = goal - start;
    let length = Vec2::new(delta.x, delta.z).length();
    let maximum = if long_probe_enabled() { 1.0 } else { 0.35 };
    if !(0.15..=maximum).contains(&length) || delta.y.abs() > 0.05 {
        return Err(format!("one push stroke needs a horizontal goal 0.15–{maximum:.2} m from the furniture"));
    }
    let direction = Vec3::new(delta.x / length, 0., delta.z / length);
    let box_aabb = object.metadata().world_aabb;
    // The broadest exposed *part* on the pushing side is the contact: the
    // seat edge from in front, the backrest from behind. Thin chair legs do
    // not become hand targets merely because they protrude furthest.
    let local_dir = object.transform().rotation.inverse() * direction;
    let along_x = local_dir.x.abs() > local_dir.z.abs();
    let sign = if along_x {
        local_dir.x.signum()
    } else {
        local_dir.z.signum()
    };
    let parts: Vec<_> = if object.parts.is_empty() {
        vec![super::super::OfficePart {
            color: None,
            cylinder: false,
            position: [0.; 3],
            half_extents: object.half_extents,
        }]
    } else {
        object.parts.clone()
    };
    let volume = |p: &super::super::OfficePart| p.half_extents.iter().product::<f32>();
    let largest = parts.iter().map(volume).fold(0_f32, f32::max);
    let face = parts
        .iter()
        .filter(|p| volume(p) >= largest * 0.25 && p.half_extents[0] >= 0.12)
        .max_by(|a, b| {
            let edge = |p: &super::super::OfficePart| {
                if along_x {
                    -sign * p.position[0] + p.half_extents[0]
                } else {
                    -sign * p.position[2] + p.half_extents[2]
                }
            };
            let separation = edge(a) - edge(b);
            if separation.abs() > 0.02 {
                separation
                    .partial_cmp(&0.)
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else {
                a.half_extents[1]
                    .partial_cmp(&b.half_extents[1])
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        })
        .ok_or("push target has no broad contact part")?;
    let mut local_contact = Vec3::from(face.position);
    if along_x {
        local_contact.x -= sign * face.half_extents[0];
    } else {
        local_contact.z -= sign * face.half_extents[2];
    }
    // Palms meet the upholstered top/edge, not the thin seat's center plane.
    let wrist_clearance = if face.half_extents[1] < 0.10 {
        0.16
    } else {
        0.08
    };
    local_contact.y = (local_contact.y + face.half_extents[1] + wrist_clearance)
        .clamp(0.48 - start.y, 1.10 - start.y);
    let contact_offset = object.transform().rotation * local_contact;
    let stage_offset = Vec3::new(contact_offset.x, 0., contact_offset.z) - direction * 0.30;
    // Validate the swept furniture AABB, not merely the endpoint. Dynamic
    // obstacles are not assumed to move out of the way.
    for other in others.iter().filter(|o| o.id != object.id) {
        let a = other.metadata().world_aabb;
        let min = Vec3::from(a.min);
        let max = Vec3::from(a.max);
        if max.y <= box_aabb.min[1] + 0.02 || min.y >= box_aabb.max[1] - 0.02 {
            continue;
        }
        for i in 1..=10 {
            let shift = delta * (i as f32 / 10.);
            let lo = Vec3::from(box_aabb.min) + shift;
            let hi = Vec3::from(box_aabb.max) + shift;
            if hi.x > min.x + 0.03
                && lo.x < max.x - 0.03
                && hi.z > min.z + 0.03
                && lo.z < max.z - 0.03
            {
                return Err(format!("push path intersects {}", other.id));
            }
        }
    }
    Ok(PushPlan {
        id: object.id.clone(),
        goal,
        direction,
        contact_offset,
        stage_offset,
        start,
    })
}

pub(super) fn contact_ready(
    action: &mut Active,
    object: Option<&OfficeObject>,
    observed: &Observation,
) -> bool {
    let plan = action.push.as_ref().expect("push plan validated at start");
    let reached = object.is_some_and(|o| {
        let contact = Vec3::from(o.position) + plan.contact_offset;
        let left = Vec3::new(plan.direction.z, 0., -plan.direction.x) * 0.18;
        observed
            .wrists
            .iter()
            .zip([left, -left])
            .all(|(w, offset)| Vec3::from(*w).distance(contact + offset) < 0.16)
            && observed.pelvis[1] > 0.72
            && observed.torso_up[1] > 0.15
    });
    if !reached {
        action.stable_since_ms = None;
        return false;
    }
    observed.displayed_pts_ms.saturating_sub(
        *action
            .stable_since_ms
            .get_or_insert(observed.displayed_pts_ms),
    ) >= 250
}

pub(super) fn settled(action: &mut Active, observed: &Observation, world: &SceneState) -> bool {
    let plan = action.push.as_ref().expect("push plan validated at start");
    let Some(position) = observed_position(world, &plan.id) else {
        return false;
    };
    let previous = action
        .placement_sample
        .replace((observed.displayed_pts_ms, position.to_array()));
    let steady = previous.is_some_and(|(pts, p)| {
        let dt = observed.displayed_pts_ms.saturating_sub(pts);
        dt > 0 && dt <= 150 && position.distance(Vec3::from(p)) < 0.025
    });
    action.moved |= position.distance(plan.start) > 0.15;
    let progress = (position - plan.start).dot(plan.direction);
    let avatar_progress =
        (Vec3::from(observed.pelvis) - Vec3::from(action.execute_origin)).dot(plan.direction);
    let face = position + plan.contact_offset;
    let wrists_follow = observed
        .wrists
        .iter()
        .all(|w| Vec3::from(*w).distance(face) < 0.27);
    if action.moved
        && position.distance(plan.goal) < 0.06
        && steady
        && avatar_progress >= progress - 0.06
        && wrists_follow
    {
        return observed.displayed_pts_ms.saturating_sub(
            *action
                .stable_since_ms
                .get_or_insert(observed.displayed_pts_ms),
        ) >= 350;
    }
    action.stable_since_ms = None;
    false
}

pub(super) fn recovered(action: &mut Active, observed: &Observation, elapsed: f64) -> bool {
    if elapsed < 1.5 || observed.pelvis[1] <= 0.85 || observed.torso_up[1] <= 0.8 {
        action.stable_since_ms = None;
        return false;
    }
    observed.displayed_pts_ms.saturating_sub(
        *action
            .stable_since_ms
            .get_or_insert(observed.displayed_pts_ms),
    ) >= 300
}

pub(super) fn observed_position(world: &SceneState, id: &str) -> Option<Vec3> {
    world
        .props
        .iter()
        .find(|p| p.id == id && !p.loading)
        .map(|p| Vec3::new(p.transform[3][0], p.transform[3][1], p.transform[3][2]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn push_requires_dynamic_furniture_clear_sweep_and_bounded_goal() {
        let mut objects = super::super::starter_objects();
        let chair = objects.iter_mut().find(|o| o.id == "chair").unwrap();
        chair.dynamic = true;
        chair.pushable = true;
        let chair = chair.clone();
        assert!(
            prepare(&chair, [0., 0.23, -1.1], &objects).is_err(),
            "desk blocks chair"
        );
        objects.retain(|o| o.id != "desk");
        let plan = prepare(&chair, [0., 0.23, -1.02], &objects).unwrap();
        assert!(plan.stage_offset.z > plan.contact_offset.z);
        assert!(plan.direction.z < 0.);
        assert!(
            (plan.contact_offset.z - 0.30).abs() < 0.01,
            "front hand contact is on the seat"
        );
        assert!(
            plan.contact_offset.y < 0.4,
            "seat edge is lower than the backrest"
        );
        let back = prepare(&chair, [0., 0.23, -0.55], &objects).unwrap();
        assert!(
            back.contact_offset.z < -0.28,
            "rear hand contact is on the backrest"
        );
        assert!(back.contact_offset.y > plan.contact_offset.y + 0.15);
        assert!(prepare(&chair, [0., 0.23, -3.], &objects).is_err());
        let mut static_chair = chair.clone();
        static_chair.pushable = false;
        assert!(prepare(&static_chair, [0., 0.23, -1.02], &objects).is_err());
    }
    #[test]
    fn kitchen_route_avoids_the_desk_and_fits_the_chair_through_the_door() {
        let mut objects = super::super::super::starter_objects();
        let chair = objects.iter_mut().find(|o| o.id == "chair").unwrap();
        chair.dynamic = true;
        chair.pushable = true;
        let chair = chair.clone();
        let route = room_route(&chair, "kitchen", &objects).unwrap();
        assert!(route.len() > 2, "the direct route crosses the office desk");
        let distance: f32 = route.windows(2).map(|p| p[0].distance(p[1])).sum();
        assert!(
            (5.0..12.).contains(&distance),
            "chair needs a substantial detour around the desk: {distance:.2} m"
        );
        assert!(
            route
                .iter()
                .filter(|p| (p.z + 4.).abs() < 0.11)
                .all(|p| (-2.3..=-1.7).contains(&p.x)),
            "chair must pass through the kitchen door"
        );
        let last = route.last().unwrap();
        assert!((last.x + 2.).abs() < 0.01 && (last.z + 4.65).abs() < 0.01);
        assert!(room_route(&chair, "garage", &objects).is_err());
        let mut blocked = objects.clone();
        let mut barrier = blocked
            .iter()
            .find(|o| o.id == "wall_kitchen_left")
            .unwrap()
            .clone();
        barrier.id = "closed_kitchen_door".into();
        barrier.position[0] = -2.;
        barrier.half_extents[0] = 0.6;
        blocked.push(barrier);
        assert!(
            room_route(&chair, "kitchen", &blocked).is_err(),
            "a closed doorway cannot be routed through"
        );
        let mut too_wide = chair.clone();
        too_wide.half_extents[0] = 2.;
        assert!(room_route(&too_wide, "kitchen", &objects).is_err());
    }
    #[test]
    fn a_long_stroke_guides_both_hands_through_one_continuous_timeline() {
        assert_eq!(hand_constraint_times(6000, true), vec![1600, 2800, 4000, 5200, 6000]);
        assert_eq!(hand_constraint_times(6000, false), vec![6000]);
        assert_eq!(hand_constraint_times(1200, true), vec![1200]);
    }
    #[test]
    fn rotated_chair_uses_the_same_seat_and_backrest_parts() {
        let mut chair = super::super::super::starter_objects()
            .into_iter()
            .find(|o| o.id == "chair")
            .unwrap();
        chair.dynamic = true;
        chair.pushable = true;
        chair.position = [1., 0.23, 0.];
        chair.yaw_deg = 90.;
        let front = prepare(&chair, [0.78, 0.23, 0.], &[]).unwrap();
        let rear = prepare(&chair, [1.22, 0.23, 0.], &[]).unwrap();
        assert!(front.contact_offset.x > 0.28);
        assert!(rear.contact_offset.x < -0.28);
        assert!(rear.contact_offset.y > front.contact_offset.y + 0.15);
    }
}
