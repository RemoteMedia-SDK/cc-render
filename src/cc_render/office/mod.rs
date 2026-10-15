//! Authored office and observed-state action controller. Coordinates are meters, Y-up.
pub(crate) mod avatar_geometry;
/// Asset-bound, read-only geometry; this cannot authorize or produce motion.
pub use avatar_geometry::AvatarGeometry;
pub mod controller;
pub mod home;
pub mod home_hud;
mod locomotion;
pub(crate) mod native_seat;
pub mod metadata;
pub mod motion_timeline;
pub mod navigation;
pub mod pathfinder;
pub(crate) mod systems;
pub mod timeline;
pub mod support;

use bevy::prelude::*;
use metadata::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OfficeObject {
    #[serde(default)]
    pub parts: Vec<OfficePart>,
    pub id: String,
    pub label: String,
    pub position: [f32; 3],
    pub half_extents: [f32; 3],
    pub color: [f32; 3],
    #[serde(default)]
    pub yaw_deg: f32,
    #[serde(default)]
    pub dynamic: bool,
    /// Dynamic furniture that may be displaced by observed hand contact.
    #[serde(default)]
    pub pushable: bool,
    #[serde(default)]
    pub rotation_xyzw: Option<[f32; 4]>,
    #[serde(default)]
    pub keypoints: std::collections::BTreeMap<String, InteractionKeypoint>,
    /// All anchors are object-local and transformed with the geometry.
    pub approach: [f32; 3],
    pub seat: Option<[f32; 3]>,
    /// Environmental support solid, independent of the retired pelvis anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat_surface_part: Option<usize>,
    pub grasp: Option<[f32; 3]>,
    pub surface: Option<[f32; 3]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OfficePart {
    /// Optional material override for upholstery, screens, ceramics and trim.
    #[serde(default)]
    pub color: Option<[f32; 3]>,
    #[serde(default)]
    pub cylinder: bool,
    pub position: [f32; 3],
    pub half_extents: [f32; 3],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InteractionKeypoint {
    pub role: String,
    pub position: [f32; 3],
    pub rotation_xyzw: [f32; 4],
}
impl OfficeObject {
    pub fn transform(&self) -> Transform {
        Transform::from_translation(Vec3::from(self.position)).with_rotation(
            self.rotation_xyzw.map_or_else(
                || Quat::from_rotation_y(self.yaw_deg.to_radians()),
                |q| Quat::from_array(q).normalize(),
            ),
        )
    }
    pub fn local_keypoint(&self, role: &str) -> Option<Transform> {
        self.keypoints.values().find(|p| p.role == role).map(|p| {
            Transform::from_translation(Vec3::from(p.position))
                .with_rotation(Quat::from_array(p.rotation_xyzw).normalize())
        })
    }
    pub fn world_keypoint(&self, role: &str) -> Option<Transform> {
        self.local_keypoint(role)
            .map(|p| self.transform().mul_transform(p))
    }
    /// Publish the same resolved affordances the controller executes, including
    /// legacy anchors on furniture that has no named keypoint map yet.
    pub fn resolved_world_keypoints(&self) -> std::collections::BTreeMap<String, InteractionKeypoint> {
        let mut points: std::collections::BTreeMap<_, _> = self.keypoints.iter().map(|(name, p)| {
            let world = self.transform().mul_transform(
                Transform::from_translation(p.position.into())
                    .with_rotation(Quat::from_array(p.rotation_xyzw).normalize()));
            (name.clone(), InteractionKeypoint { role: p.role.clone(),
                position: world.translation.to_array(), rotation_xyzw: world.rotation.to_array() })
        }).collect();
        for (role, position) in [
            ("approach", Some(self.approach_position())),
            ("grasp", self.grasp_transform().map(|t| t.translation.to_array())),
            ("seat_pelvis", self.seat_position()),
            ("placement", self.placement_position()),
        ] {
            if points.values().any(|p| p.role == role) { continue; }
            if let Some(position) = position {
                points.insert(role.into(), InteractionKeypoint { role: role.into(), position,
                    rotation_xyzw: self.transform().rotation.to_array() });
            }
        }
        points
    }

    pub fn grasp_transform(&self) -> Option<Transform> {
        self.world_keypoint("grasp").or_else(|| {
            self.grasp.map(|p| {
                self.transform()
                    .mul_transform(Transform::from_translation(Vec3::from(p)))
            })
        })
    }
    pub fn seat_position(&self) -> Option<[f32; 3]> {
        self.world_keypoint("seat_pelvis")
            .map(|t| t.translation.to_array())
            .or_else(|| self.seat.map(|p| self.anchor(p)))
    }
    /// Front edge of the seat-bearing collider in the object's local frame.
    /// Prefer the broad support immediately below the pelvis over chair legs
    /// or a taller backrest; transformed objects retain the same local gap.
    fn seat_front_z(&self) -> Option<f32> {
        let seat = self.seat_position()?;
        let transform = self.transform();
        let local = transform.rotation.inverse() * (Vec3::from(seat) - transform.translation);
        self.parts.iter()
            .filter(|p| {
                let below = local.y - (p.position[1] + p.half_extents[1]);
                (0.0..=0.30).contains(&below)
            })
            .max_by(|a, b| {
                let area = |p: &OfficePart| p.half_extents[0] * p.half_extents[2];
                area(a).partial_cmp(&area(b)).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.position[2] + p.half_extents[2])
    }
    /// Stage the pelvis beyond the seat front with room for the legs and
    /// normal model overshoot. This is geometry, not a body-joint pose.
    pub fn seat_staging_position(&self) -> [f32; 3] {
        let front = self.seat_front_z().map_or(0.35, |z| z + 0.10);
        self.anchor([0., 0., front.max(0.35)])
    }
    pub fn seat_floor_foot(&self, world_position: [f32; 3]) -> [f32; 3] {
        let Some(front) = self.seat_front_z() else { return world_position };
        let transform = self.transform();
        let mut local = transform.rotation.inverse() * (Vec3::from(world_position) - transform.translation);
        local.z = local.z.max(front + 0.28);
        transform.transform_point(local).to_array()
    }
    pub fn approach_position(&self) -> [f32; 3] {
        self.world_keypoint("approach")
            .map_or_else(|| self.anchor(self.approach), |t| t.translation.to_array())
    }
    pub fn placement_position(&self) -> Option<[f32; 3]> {
        self.world_keypoint("placement")
            .map(|t| t.translation.to_array())
            .or_else(|| self.surface.map(|p| self.anchor(p)))
    }
    pub fn anchor(&self, local: [f32; 3]) -> [f32; 3] {
        self.transform()
            .transform_point(Vec3::from(local))
            .to_array()
    }
    pub fn metadata(&self) -> SceneObject {
        let h = Vec3::from(self.half_extents);
        let t = self.transform();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let boxes = std::iter::once(([0.; 3], self.half_extents)).chain(
            self.parts
                .iter()
                .map(|part| (part.position, part.half_extents)),
        );
        for (center, half) in boxes {
            let half = Vec3::from(half);
            for x in [-half.x, half.x] {
                for y in [-half.y, half.y] {
                    for z in [-half.z, half.z] {
                        let p = t.transform_point(Vec3::from(center) + Vec3::new(x, y, z));
                        min = min.min(p);
                        max = max.max(p);
                    }
                }
            }
        }
        SceneObject {
            id: self.id.clone(),
            object_type: self.label.clone(),
            transform: ObjectTransform {
                position: self.position,
                yaw_deg: self.yaw_deg,
            },
            world_aabb: Aabb {
                min: min.to_array(),
                max: max.to_array(),
            },
            // Footprint's 2D rotation uses the opposite sign to right-handed Y rotation.
            footprint: Footprint::Rect {
                center_xz: [self.position[0], self.position[2]],
                size: [2.0 * h.x, 2.0 * h.z],
                yaw_deg: -self.yaw_deg,
            },
            anchors: Some(Anchors {
                seat_pelvis: self.seat_position(),
                stand_to_sit: Some(self.approach_position()),
                facing_yaw_deg: Some(self.yaw_deg),
            }),
            sit_anchor_source: self.seat.map(|_| "authored".into()),
        }
    }
}
pub fn starter_objects() -> Vec<OfficeObject> {
    let mut objects: Vec<OfficeObject> =
        serde_json::from_str(include_str!("starter.json")).expect("bundled office is valid");
    objects.extend(
        serde_json::from_str::<Vec<OfficeObject>>(include_str!("home.json"))
            .expect("bundled home is valid"),
    );
    objects
}
pub fn validate_objects(objects: &[OfficeObject]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for o in objects {
        if o.id.is_empty() || !ids.insert(&o.id) {
            return Err("office object IDs must be nonempty and unique".into());
        }
        let valid_rotation = |q: [f32; 4]| {
            q.iter().all(|v| v.is_finite()) && q.iter().map(|v| v * v).sum::<f32>() > 1e-6
        };
        if (o.pushable && !o.dynamic)
            || o.rotation_xyzw.is_some_and(|q| !valid_rotation(q))
            || o.keypoints.iter().any(|(name, p)| {
                name.is_empty()
                    || p.role.is_empty()
                    || p.position.iter().any(|v| !v.is_finite())
                    || !valid_rotation(p.rotation_xyzw)
            })
        {
            return Err(format!("{} has invalid interaction keypoints", o.id));
        }
        let vectors = [
            Some(o.position),
            Some(o.approach),
            o.seat,
            o.grasp,
            o.surface,
        ];
        if !o.yaw_deg.is_finite()
            || vectors
                .into_iter()
                .flatten()
                .flatten()
                .any(|v| !v.is_finite())
            || o.color
                .iter()
                .copied()
                .chain(o.parts.iter().filter_map(|p| p.color).flatten())
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(&v))
        {
            return Err(format!("{} has invalid transform or anchor values", o.id));
        }
        for (position, half) in std::iter::once(([0.; 3], o.half_extents))
            .chain(o.parts.iter().map(|p| (p.position, p.half_extents)))
        {
            if position.iter().any(|v| !v.is_finite())
                || half.iter().any(|v| !v.is_finite() || *v <= 0.)
                || (position[0].abs() + half[0] > o.half_extents[0] + 1e-5)
                || (position[2].abs() + half[2] > o.half_extents[2] + 1e-5)
            {
                return Err(format!(
                    "{} has invalid geometry or exceeds its navigation footprint",
                    o.id
                ));
            }
        }
    }
    Ok(())
}
pub fn scene_metadata(objects: &[OfficeObject]) -> SceneMetadata {
    SceneMetadata {
        schema_version: 1,
        floor_y: 0.0,
        avatar_radius: 0.30,
        objects: objects.iter().map(OfficeObject::metadata).collect(),
        spawn_points: vec![],
    }
}

// Retired correction implementation is retained only for historical unit tests.
// It cannot be linked into production playback.
#[cfg(test)]
pub(crate) mod contact;
pub mod motion_intent;
pub mod scene_context;
pub(crate) mod scene_lifecycle;
pub(crate) mod scene_delivery;
pub(crate) mod scene_signing;

pub(crate) fn compose_expression(
    weights: &mut std::collections::HashMap<String, f32>,
    office: Option<&controller::OfficeController>,
) {
    let Some(office) = office else { return };
    for channel in ["mouthSmileLeft", "mouthSmileRight"] {
        let speech = weights.get(channel).copied().unwrap_or(0.);
        weights.insert(channel.into(), speech + (1. - speech) * office.smile());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_geometry_and_anchors_share_transform() {
        let objects = starter_objects();
        validate_objects(&objects).unwrap();
        let sofa = objects.iter().find(|o| o.id == "sofa").unwrap();
        assert!((sofa.anchor(sofa.approach)[0] + 1.45).abs() < 1e-5);
        let chair = objects.iter().find(|o| o.id == "chair").unwrap();
        assert!((chair.metadata().world_aabb.max[1] - 1.02).abs() < 1e-5);
        let mut bad = objects.clone();
        bad[0].half_extents[0] = -1.;
        assert!(validate_objects(&bad).is_err());
        bad = objects.clone();
        bad[0].id = bad[1].id.clone();
        assert!(validate_objects(&bad).is_err());
    }
    #[test]
    fn mug_handle_keypoint_follows_full_prop_rotation() {
        let mut mug = starter_objects()
            .into_iter()
            .find(|o| o.id == "mug")
            .unwrap();
        mug.position = [1., 2., 3.];
        let rotation = Quat::from_rotation_z(0.7) * Quat::from_rotation_y(1.2);
        mug.rotation_xyzw = Some(rotation.to_array());
        let local = mug.local_keypoint("grasp").unwrap();
        let world = mug.grasp_transform().unwrap();
        assert!(
            world
                .translation
                .distance(Vec3::from(mug.position) + rotation * local.translation)
                < 1e-5
        );
        assert!(world.rotation.angle_between(rotation * local.rotation) < 1e-3);
        assert!(
            local.translation.x > 0.045,
            "grasp is on the handle, outside the cup body"
        );
        mug.keypoints.get_mut("handle_grasp").unwrap().rotation_xyzw = [0.; 4];
        assert!(validate_objects(&[mug]).is_err());
    }
}
