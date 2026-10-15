//! Renderer-produced environmental context. Deliberately Serialize-only: client
//! JSON cannot construct this type or turn legacy pelvis/foot anchors into goals.
use super::{controller::OfficeSnapshot, OfficeObject};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct SceneSnapshot {
    version: u8,
    session_id: String,
    scene_revision: u64,
    displayed_pts_ms: u64,
    floor_y: f32,
    objects: Vec<SceneObject>,
}

#[derive(Clone, Debug, Serialize)]
struct SceneObject {
    id: String,
    position: [f32; 3],
    rotation_xyzw: [f32; 4],
    half_extents: [f32; 3],
    dynamic: bool,
    navigation_obstacle: bool,
    navigation_yaw_deg: f32,
    parts: Vec<SolidPart>,
    affordances: BTreeMap<&'static str, Affordance>,
    /// Declared interaction data (object-local); absent when an object declares none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    zones: Vec<super::interaction::Zone>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    interactions: Vec<super::interaction::Interaction>,
    /// Authored home pose [x, y, z, qx, qy, qz, qw] for `@home/zone` references
    /// (objects that declare interactions only).
    #[serde(skip_serializing_if = "Option::is_none")]
    home: Option<[f32; 7]>,
}

#[derive(Clone, Debug, Serialize)]
struct SolidPart {
    position: [f32; 3],
    half_extents: [f32; 3],
    shape: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum Affordance {
    Region { center: [f32; 3], radius: f32 },
    Seat { part_index: usize },
}

impl SceneSnapshot {
    pub(crate) fn matches(&self, goal: &super::motion_intent::MotionGoal) -> bool {
        self.session_id == goal.session_id() && self.displayed_pts_ms == goal.pts_ms()
    }

    pub(crate) fn from_office(office: &OfficeSnapshot, session_id: &str, revision: u64) -> Result<Self, String> {
        if session_id.is_empty() || session_id.chars().count() > 128 || revision > (1_u64 << 53)-1 {
            return Err("invalid scene snapshot identity".into());
        }
        let objects = office.objects.iter().map(|object| {
            let mut exported = SceneObject::from_object(object, office.navigation_object_ids.contains(&object.id))?;
            if !object.interactions.is_empty() { exported.home = office.homes.get(&object.id).copied(); }
            Ok::<_, String>(exported)
        }).collect::<Result<Vec<_>, _>>()?;
        Ok(Self { version: 1, session_id: session_id.into(), scene_revision: revision,
            displayed_pts_ms: office.avatar.displayed_pts_ms, floor_y: 0., objects })
    }
}

impl SceneObject {
    fn from_object(object: &OfficeObject, navigation_obstacle: bool) -> Result<Self, String> {
        let mut affordances = BTreeMap::new();
        let approach = object.local_keypoint("approach").map_or(object.approach, |t| t.translation.to_array());
        affordances.insert("approach", Affordance::Region { center: approach, radius: 0.25 });
        if let Some(center) = object.local_keypoint("grasp").map(|t| t.translation.to_array()).or(object.grasp) {
            affordances.insert("grasp", Affordance::Region { center, radius: 0.02 });
        }
        if let Some(part_index) = object.seat_surface_part {
            let part = object.parts.get(part_index).ok_or("seat surface references missing solid")?;
            if part.cylinder || part.half_extents[1] >= part.half_extents[0].min(part.half_extents[2]) {
                return Err("seat surface must reference a horizontal box slab".into());
            }
            affordances.insert("seat", Affordance::Seat { part_index });
        }
        Ok(Self {
            id: object.id.clone(), position: object.position,
            rotation_xyzw: object.transform().rotation.to_array(), half_extents: object.half_extents,
            dynamic: object.dynamic, navigation_obstacle, navigation_yaw_deg: object.yaw_deg,
            parts: object.parts.iter().map(|p| SolidPart { position: p.position,
                half_extents: p.half_extents, shape: if p.cylinder { "cylinder" } else { "box" } }).collect(),
            affordances,
            zones: object.zones.clone(),
            interactions: object.interactions.clone(),
            home: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::controller::OfficeController;

    #[test]
    fn context_exports_surface_not_pelvis_or_foot_targets() {
        let controller = OfficeController::default();
        let mut office = controller.snapshot();
        let before = serde_json::to_value(SceneSnapshot::from_office(&office, "test", 1).unwrap()).unwrap();
        let chair = office.objects.iter_mut().find(|o| o.id == "chair").unwrap();
        chair.seat = Some([100., 100., 100.]);
        chair.keypoints.retain(|_, key| key.role != "seat_pelvis");
        let after = serde_json::to_value(SceneSnapshot::from_office(&office, "test", 1).unwrap()).unwrap();
        assert_eq!(before, after);
        assert!(!after.to_string().contains("seat_pelvis"));
        let chair = after["objects"].as_array().unwrap().iter().find(|o| o["id"] == "chair").unwrap();
        assert_eq!(chair["affordances"]["seat"]["part_index"], 0);
        let top = chair["parts"][0]["position"][1].as_f64().unwrap()
            + chair["parts"][0]["half_extents"][1].as_f64().unwrap()
            + chair["position"][1].as_f64().unwrap();
        assert!((top - 0.46).abs() < 1e-6);
    }

    #[test]
    fn declared_interactions_round_trip_with_their_home_pose_only_when_declared() {
        let mut controller = OfficeController::default();
        let office = controller.snapshot();
        let value = serde_json::to_value(SceneSnapshot::from_office(&office, "test", 2).unwrap()).unwrap();
        let objects = value["objects"].as_array().unwrap();
        let broom = objects.iter().find(|o| o["id"] == "broom").unwrap();
        let parsed: Vec<super::super::interaction::Interaction> =
            serde_json::from_value(broom["interactions"].clone()).unwrap();
        assert_eq!(parsed, office.objects.iter().find(|o| o.id == "broom").unwrap().interactions);
        assert_eq!(broom["home"][0].as_f64().unwrap() as f32, -1.6);
        let shelf = objects.iter().find(|o| o["id"] == "shelf").unwrap();
        assert!(shelf.get("home").is_none() && shelf.get("zones").is_none() && shelf.get("interactions").is_none());
        // The home pose stays authored while the object itself moves.
        controller.objects.iter_mut().find(|o| o.id == "broom").unwrap().position = [0., 1., 0.];
        let moved = serde_json::to_value(SceneSnapshot::from_office(&controller.snapshot(), "test", 3).unwrap()).unwrap();
        let broom = moved["objects"].as_array().unwrap().iter().find(|o| o["id"] == "broom").unwrap();
        assert_eq!((broom["position"][0].as_f64(), broom["home"][0].as_f64().map(|v| v as f32)), (Some(0.), Some(-1.6)));
    }

    #[test]
    fn navigation_membership_is_authoritative_not_a_dynamic_heuristic() {
        let mut office = OfficeController::default().snapshot();
        office.navigation_object_ids.retain(|id| id != "mug");
        let snapshot = SceneSnapshot::from_office(&office, "test", 4).unwrap();
        assert!(!snapshot.objects.iter().find(|o| o.id == "mug").unwrap().navigation_obstacle);
        assert!(snapshot.objects.iter().find(|o| o.id == "book").unwrap().navigation_obstacle);
    }
}
