//! Runtime support discovery from the same cuboids used by the office colliders.
use super::OfficeObject;
use bevy::prelude::*;
use serde::Serialize;

// CSKel27 neutral dimensions, shared with the surface-jump probe. These are
// skeleton calibration, not furniture-specific contact coordinates.
pub(super) const ROOT_HEIGHT: f32 = 0.9544128;
pub(super) const ANKLE_HEIGHT: f32 = 0.0584749;

#[derive(Clone, Debug)]
pub(super) struct ColliderBox {
    pub owner: String,
    pub center: Vec3,
    pub half: Vec3,
    pub rotation: Quat,
}
impl ColliderBox {
    pub fn intersects(&self, start: Vec3, end: Vec3, radius: f32) -> bool {
        let a = self.rotation.inverse() * (start - self.center);
        let delta = self.rotation.inverse() * (end - start);
        let extent = self.half + Vec3::splat(radius);
        // Negative radius erodes the box. An exhausted axis is empty, not a
        // reversed slab whose endpoints can be sorted back into a solid.
        if extent.min_element() < 0. { return false; }
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for axis in 0..3 {
            if delta[axis].abs() < 1e-7 {
                if a[axis].abs() > extent[axis] {
                    return false;
                }
            } else {
                let near = (-extent[axis] - a[axis]) / delta[axis];
                let far = (extent[axis] - a[axis]) / delta[axis];
                lo = lo.max(near.min(far));
                hi = hi.min(near.max(far));
            }
        }
        lo <= hi
    }
}

pub(super) fn boxes(objects: &[OfficeObject]) -> Vec<ColliderBox> {
    let mut result = Vec::new();
    for object in objects {
        let transform = object.transform();
        let parts: Vec<_> = if object.parts.is_empty() {
            vec![(Vec3::ZERO, Vec3::from(object.half_extents))]
        } else {
            object
                .parts
                .iter()
                .map(|p| (Vec3::from(p.position), Vec3::from(p.half_extents)))
                .collect()
        };
        for (position, half) in parts {
            result.push(ColliderBox {
                owner: object.id.clone(),
                center: transform.transform_point(position),
                half,
                rotation: transform.rotation,
            });
        }
    }
    result
}

pub(super) fn stance_clear(point: Vec3, boxes: &[ColliderBox]) -> bool {
    !boxes
        .iter()
        .any(|b| b.intersects(point + Vec3::Y * 0.255, point + Vec3::Y * 1.57, 0.23))
}

#[derive(Clone, Debug, Serialize)]
pub struct StandingSurface {
    pub object_id: String,
    pub position: [f32; 3],
    pub floor_position: [f32; 3],
    pub heading: f32,
}

/// The first traversal version approaches the object's local +Z side. Search
/// its actual part tops for two-foot area, headroom, and a clear floor stance.
pub(super) fn discover(objects: &[OfficeObject], target: &str) -> Result<StandingSurface, String> {
    let object = objects
        .iter()
        .find(|o| o.id == target)
        .ok_or("unknown jump target")?;
    if object.dynamic {
        return Err("jump support must be static".into());
    }
    if (object.transform().rotation * Vec3::Y).distance(Vec3::Y) > 1e-4 {
        return Err("jump support must be upright".into());
    }
    let colliders = boxes(objects);
    let mut candidates = Vec::new();
    for b in colliders.iter().filter(|b| b.owner == target) {
        let usable = b.half - Vec3::new(0.20, 0., 0.18);
        let top = b.center.y + b.half.y;
        if usable.x < 0. || usable.z < 0. || !(0.15..=0.85).contains(&top) {
            continue;
        }
        for zi in 0..5 {
            for xi in 0..5 {
                let x = usable.x * (xi as f32 / 2. - 1.);
                let z = usable.z * (1. - zi as f32 / 2.);
                let landing = b.center + b.rotation * Vec3::new(x, b.half.y, z);
                let mut floor = landing + b.rotation * Vec3::Z * 0.8;
                floor.y = 0.;
                if stance_clear(landing, &colliders) && stance_clear(floor, &colliders) {
                    let direction = landing - floor;
                    candidates.push((
                        (top, -z, x.abs()),
                        StandingSurface {
                            object_id: target.into(),
                            position: landing.to_array(),
                            floor_position: floor.to_array(),
                            heading: direction.x.atan2(direction.z),
                        },
                    ));
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    candidates
        .into_iter()
        .next()
        .map(|(_, support)| support)
        .ok_or_else(|| "no support with two-foot area, headroom and clear takeoff".into())
}

/// Observed ankles must actually project onto a supporting top face; distance
/// from a planned center alone could complete a landing over empty air.
pub(super) fn feet_inside_patch(
    feet: &[[f32; 3]; 2], surface: &StandingSurface, objects: &[OfficeObject],
) -> bool {
    let top = surface.position[1];
    let patches = boxes(objects);
    feet.iter().all(|foot| patches.iter().any(|b| {
        if b.owner != surface.object_id || (b.center.y + b.half.y - top).abs() > 0.04 {
            return false;
        }
        let local = b.rotation.inverse() * (Vec3::from(*foot) - b.center);
        local.x.abs() <= b.half.x - 0.035 && local.z.abs() <= b.half.z - 0.035
    }))
}

pub(super) fn flight_duration(from: Vec3, to: Vec3) -> u64 {
    let dy = to.y - from.y;
    let rise = (2. * (dy.max(0.) + 0.10) / 9.81).sqrt();
    let fall = (2. * ((-dy).max(0.) + 0.10) / 9.81).sqrt();
    let horizontal = Vec2::new(to.x - from.x, to.z - from.z).length() / 2.5;
    ((0.30 + (rise + fall).max(horizontal)) * 20.).ceil() as u64 * 50
}

pub(super) fn foot_key(
    point: [f32; 3],
    heading: f32,
    start: u64,
    end: Option<u64>,
) -> serde_json::Value {
    let right = Quat::from_rotation_y(heading) * Vec3::X * 0.12;
    let ankle = Vec3::from(point) + Vec3::Y * ANKLE_HEIGHT;
    let mut key = serde_json::json!({"kind":"end_effectors", "pts_ms":start, "heading":heading,
        "keypoints":[{"joint":"LeftFoot", "position":(ankle + right).to_array()},
                     {"joint":"RightFoot", "position":(ankle - right).to_array()}]});
    if let Some(end) = end {
        key["end_ms"] = end.into();
    } else {
        key["hold"] = true.into();
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::{pathfinder, scene_metadata, starter_objects};

    #[test]
    fn eroded_thin_colliders_are_empty_without_losing_real_contacts() {
        for rotation in [Quat::IDENTITY, Quat::from_rotation_z(0.4)] {
            let slab = ColliderBox { owner:"shelf".into(),center:Vec3::new(1.,2.,3.),
                half:Vec3::new(0.08,0.009,0.28),rotation };
            let start=slab.center+rotation*Vec3::new(0.,-0.1,0.);
            let end=slab.center+rotation*Vec3::new(0.01,0.1,0.01);
            assert!(slab.intersects(start,end,0.), "actual shelf contact remains");
            assert!(slab.intersects(start,end,-0.005), "nonempty erosion remains a solid");
            assert!(!slab.intersects(start,end,-0.05), "eroded thin shelf has no interior");
            assert!(!slab.intersects(slab.center,slab.center,-0.05));
            assert!(!slab.intersects(start+rotation*Vec3::X,end+rotation*Vec3::X,0.));
        }
    }

    #[test]
    fn sofa_contacts_come_from_parts_and_survive_scene_transforms() {
        for variant in 0..4 {
            let mut objects = starter_objects();
            let sofa = objects.iter_mut().find(|o| o.id == "living_sofa").unwrap();
            match variant {
                1 => {
                    sofa.position[0] += 0.25;
                    sofa.position[2] -= 0.15;
                }
                2 => {
                    sofa.rotation_xyzw =
                        Some(Quat::from_rotation_y(200_f32.to_radians()).to_array());
                }
                3 => {
                    let scale = Vec3::new(1.1, 1.15, 1.05);
                    sofa.position[1] *= scale.y;
                    for part in &mut sofa.parts {
                        part.position = (Vec3::from(part.position) * scale).to_array();
                        part.half_extents = (Vec3::from(part.half_extents) * scale).to_array();
                    }
                }
                _ => {}
            }
            let support = discover(&objects, "living_sofa").unwrap();
            assert!((support.position[1] - if variant == 3 { 0.667 } else { 0.58 }).abs() < 0.001);
            assert!(stance_clear(support.position.into(), &boxes(&objects)));
            let center = Vec3::from(support.position) + Vec3::Y * ANKLE_HEIGHT;
            let across = Quat::from_rotation_y(support.heading) * Vec3::X;
            let feet = [(center + across * 0.12).to_array(),
                        (center - across * 0.12).to_array()];
            assert!(feet_inside_patch(&feet, &support, &objects));
            let empty_air = [(center + across * 1.2).to_array(), feet[1]];
            assert!(!feet_inside_patch(&empty_air, &support, &objects));
            let scene = scene_metadata(&objects);
            assert!(pathfinder::find_path(
                &scene,
                Vec2::ZERO,
                Vec2::new(support.floor_position[0], support.floor_position[2]),
                None
            )
            .is_ok());
        }
    }

    #[test]
    fn seat_staging_and_feet_clear_front_edge_under_scene_transforms() {
        for variant in 0..3 {
            let mut objects = starter_objects();
            let chair = objects.iter_mut().find(|o| o.id == "chair").unwrap();
            if variant == 1 {
                chair.position[0] += 0.4;
                chair.rotation_xyzw = Some(Quat::from_rotation_y(0.6).to_array());
            } else if variant == 2 {
                for part in &mut chair.parts {
                    part.position[2] *= 1.25;
                    part.half_extents[2] *= 1.25;
                }
            }
            let transform = chair.transform();
            let local = |world: [f32; 3]| {
                transform.rotation.inverse() * (Vec3::from(world) - transform.translation)
            };
            let stage = local(chair.seat_staging_position());
            let foot = local(chair.seat_floor_foot(
                chair.world_keypoint("left_foot").unwrap().translation.to_array()));
            let front = if variant == 2 { 0.375 } else { 0.3 };
            assert!(stage.z >= front + 0.09, "pelvis needs room for bent legs");
            assert!(foot.z >= front + 0.27, "feet need clearance from seat front");
            assert!(foot.z >= stage.z);
        }
    }

    #[test]
    fn unsupported_targets_fail_and_timing_tracks_height() {
        let mut objects = starter_objects();
        assert!(discover(&objects, "missing").is_err());
        assert!(discover(&objects, "mug").is_err());
        let sofa = objects.iter_mut().find(|o| o.id == "living_sofa").unwrap();
        sofa.rotation_xyzw = Some(Quat::from_rotation_z(0.3).to_array());
        assert!(discover(&objects, "living_sofa").is_err());
        assert_eq!(flight_duration(Vec3::ZERO, Vec3::new(0., 0.58, 0.8)), 850);
        assert_eq!(flight_duration(Vec3::new(0., 0.58, 0.8), Vec3::ZERO), 850);
    }
}
