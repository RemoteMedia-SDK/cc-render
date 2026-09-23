//! Read-only path queries over the latest renderer snapshot.
use super::{
    controller::walking_waypoints, controller::OfficeSnapshot, pathfinder, scene_metadata,
};
use crate::cc_render::renderer::SceneState;
use bevy::math::Vec2;
use serde::{Deserialize, Serialize};

/// An exact object ID, "avatar", or world XYZ coordinates in metres.
/// Navigation projects coordinates onto the scene floor (XZ); it cannot climb.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NavigationTarget {
    Name(String),
    Position([f32; 3]),
}
impl From<&str> for NavigationTarget {
    fn from(value: &str) -> Self {
        Self::Name(value.into())
    }
}
impl From<[f32; 3]> for NavigationTarget {
    fn from(value: [f32; 3]) -> Self {
        Self::Position(value)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaypointPath {
    /// The endpoints and intervening corners, at floor height in world metres.
    pub waypoints: Vec<[f32; 3]>,
    /// Walking pelvis XYZ and relative seconds, using the walk action's timing.
    /// These are a preview, not an activated motion revision.
    pub root_waypoints: Vec<[f32; 4]>,
    pub distance_m: f32,
    pub duration_sec: f32,
    pub session_id: String,
    pub generation_revision: u64,
    pub snapshot_pts_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum NavigationError {
    #[error("office scene is not available")]
    SceneUnavailable,
    #[error("avatar has no ready observation; supply explicit coordinates")]
    AvatarNotReady,
    #[error("unknown object ID: {0}")]
    UnknownTarget(String),
    #[error("coordinates must be finite world XYZ values")]
    InvalidPosition,
    #[error(transparent)]
    Path(#[from] pathfinder::PathfindError),
}
impl NavigationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::SceneUnavailable => "scene_unavailable",
            Self::AvatarNotReady => "avatar_not_ready",
            Self::UnknownTarget(_) => "unknown_target",
            Self::InvalidPosition => "invalid_position",
            Self::Path(pathfinder::PathfindError::StartBlocked(_)) => "start_blocked",
            Self::Path(pathfinder::PathfindError::GoalBlocked(_)) => "goal_blocked",
            Self::Path(pathfinder::PathfindError::NoPath) => "no_path",
        }
    }
}

impl SceneState {
    /// Generate a route without moving the avatar or editing its motion timeline.
    /// Object endpoints use transformed approach anchors. Held objects are omitted
    /// as obstacles, matching the action controller; destinations remain obstacles.
    pub fn generate_waypoint(
        &self,
        from: impl Into<NavigationTarget>,
        to: impl Into<NavigationTarget>,
    ) -> Result<WaypointPath, NavigationError> {
        let office = self
            .office
            .as_ref()
            .ok_or(NavigationError::SceneUnavailable)?;
        let from = from.into();
        let start = resolve_target(office, &from)?;
        let to = to.into();
        let goal = resolve_target(office, &to)?;
        let facing = match &to {
            NavigationTarget::Name(id) => office.objects.iter().find(|o| &o.id == id)
                .map(super::locomotion::target_direction),
            NavigationTarget::Position(_) => None,
        };
        let objects: Vec<_> = office
            .objects
            .iter()
            .filter(|o| !office.occupied_hands.iter().any(|(_, id)| id == &o.id))
            .cloned()
            .collect();
        let scene = scene_metadata(&objects);
        let mut path = super::locomotion::approach_path(&scene, start, goal, facing, super::locomotion::arrival_angle())?;
        path.dedup_by(|a, b| a.xz == b.xz);
        // An explicit origin does not specify an orientation. Assume it already
        // faces the first segment; only "avatar" includes an observed turn.
        let heading = if matches!(from, NavigationTarget::Name(ref name) if name == "avatar") {
            office.avatar.heading
        } else {
            let direction = path.get(1).map_or(Vec2::ZERO, |p| p.xz - start);
            direction.x.atan2(direction.y)
        };
        let root_waypoints = if std::env::var("OFFICE_NATURAL_NAVIGATION").as_deref() == Ok("0") {
            walking_waypoints(&path, 0., heading)
        } else { super::locomotion::walking(&scene, &path, 0.) };
        let distance_m = root_waypoints.windows(2).map(|p|
            Vec2::new(p[1][0] - p[0][0], p[1][2] - p[0][2]).length()).sum();
        Ok(WaypointPath {
            waypoints: path
                .iter()
                .map(|p| [p.xz.x, scene.floor_y, p.xz.y])
                .collect(),
            duration_sec: root_waypoints.last().map_or(0., |p| p[3]),
            root_waypoints,
            distance_m,
            session_id: office.motion.session_id.clone(),
            generation_revision: office.motion.generation_revision,
            snapshot_pts_ms: office.avatar.displayed_pts_ms,
        })
    }
}

fn resolve_target(
    office: &OfficeSnapshot,
    target: &NavigationTarget,
) -> Result<Vec2, NavigationError> {
    let position = match target {
        NavigationTarget::Position(p) => *p,
        NavigationTarget::Name(id) if id == "avatar" => {
            if !office.avatar.position_valid && !office.avatar.ready {
                return Err(NavigationError::AvatarNotReady);
            }
            office.avatar.pelvis
        }
        NavigationTarget::Name(id) => office
            .objects
            .iter()
            .find(|o| &o.id == id)
            .ok_or_else(|| NavigationError::UnknownTarget(id.clone()))?
            .approach_position(),
    };
    if !position.iter().all(|v| v.is_finite()) {
        return Err(NavigationError::InvalidPosition);
    }
    Ok(Vec2::new(position[0], position[2]))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WaypointQuery {
    GenerateWaypoint {
        from: NavigationTarget,
        to: NavigationTarget,
        #[serde(default)]
        request_id: Option<String>,
    },
}

/// The streaming query reply always preserves the request ID, including errors.
pub(crate) fn query_reply(state: &SceneState, value: &serde_json::Value) -> serde_json::Value {
    let request_id = value.get("request_id").cloned().unwrap_or_default();
    let result = serde_json::from_value::<WaypointQuery>(value.clone())
        .map_err(|error| ("invalid_request", error.to_string()))
        .and_then(
            |WaypointQuery::GenerateWaypoint {
                 from,
                 to,
                 request_id,
             }| {
                let _ = request_id; // Deserialized for schema validation.
                state
                    .generate_waypoint(from, to)
                    .map_err(|error| (error.code(), error.to_string()))
            },
        );
    match result {
        Ok(path) => serde_json::json!({"kind":"waypoint_path", "request_id":request_id,
            "ok":true, "path":path}),
        Err((code, message)) => serde_json::json!({"kind":"waypoint_path", "request_id":request_id,
            "ok":false, "error":{"code":code, "message":message}}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::controller::OfficeController;
    use crate::cc_render::renderer::Hand;

    fn scene() -> SceneState {
        let mut controller = OfficeController::default();
        controller.observed.ready = true;
        controller.observed.pelvis = [0., 0.95, 0.];
        SceneState {
            office: Some(controller.snapshot()),
            ..Default::default()
        }
    }

    #[test]
    fn query_reaches_every_furniture_approach_without_changing_state() {
        let state = scene();
        let before = serde_json::to_value(&state).unwrap();
        let office = state.office.as_ref().unwrap();
        let metadata = scene_metadata(&office.objects);
        let obstacles: Vec<_> = metadata
            .objects
            .iter()
            .map(|o| (o.id.clone(), o.footprint.inflated_polygon(0.35)))
            .collect();
        for object in office.objects.iter().filter(|o| !o.id.starts_with("wall_")) {
            let path = state
                .generate_waypoint("avatar", object.id.as_str())
                .unwrap();
            let goal = object.approach_position();
            assert_eq!(path.waypoints.last().unwrap(), &[goal[0], 0., goal[2]]);
            for segment in path.waypoints.windows(2) {
                assert!(pathfinder::visible(
                    Vec2::new(segment[0][0], segment[0][2]),
                    Vec2::new(segment[1][0], segment[1][2]),
                    &obstacles
                ));
            }
            assert!(path.root_waypoints.windows(2).all(|p| p[1][3] > p[0][3]));
            assert!(path.duration_sec >= path.distance_m / 0.8 - 1e-4);
        }
        assert_eq!(before, serde_json::to_value(&state).unwrap());
    }

    #[test]
    fn object_and_coordinate_destinations_share_endpoints_but_only_objects_specify_facing() {
        let state = scene();
        let objects = &state.office.as_ref().unwrap().objects;
        let chair = objects
            .iter()
            .find(|o| o.id == "chair")
            .unwrap()
            .approach_position();
        let bed = objects
            .iter()
            .find(|o| o.id == "bed")
            .unwrap()
            .approach_position();
        let by_id = state.generate_waypoint("chair", "bed").unwrap();
        let by_xyz = state.generate_waypoint(chair, bed).unwrap();
        assert_eq!(by_id.waypoints.first(), by_xyz.waypoints.first());
        assert_eq!(by_id.waypoints.last(), by_xyz.waypoints.last());
        let last = &by_id.root_waypoints[by_id.root_waypoints.len()-2..];
        let bed_object = objects.iter().find(|o| o.id == "bed").unwrap();
        assert!(super::super::locomotion::within_arrival_cone(
            Vec2::new(last[1][0]-last[0][0], last[1][2]-last[0][2]),
            super::super::locomotion::target_direction(bed_object), super::super::locomotion::arrival_angle() + 1e-5));
        assert!(
            by_id.waypoints.len() > 2,
            "must detour through bedroom door"
        );
        let identical = state.generate_waypoint([0., 5., 0.], [0., 0., 0.]).unwrap();
        assert_eq!(identical.waypoints, vec![[0., 0., 0.]]);
        assert_eq!(identical.distance_m, 0.);
        assert_eq!(identical.duration_sec, 0.);
    }

    #[test]
    fn unreachable_unknown_and_unready_queries_fail_explicitly() {
        let mut state = scene();
        assert_eq!(
            state
                .generate_waypoint("avatar", "missing")
                .unwrap_err()
                .code(),
            "unknown_target"
        );
        assert_eq!(
            state
                .generate_waypoint([0., 0., -1.8], "bed")
                .unwrap_err()
                .code(),
            "start_blocked"
        );
        assert_eq!(
            state
                .generate_waypoint("avatar", [0., 0., -1.8])
                .unwrap_err()
                .code(),
            "goal_blocked"
        );
        assert_eq!(
            state
                .generate_waypoint("avatar", [7., 0., 0.])
                .unwrap_err()
                .code(),
            "no_path"
        );
        assert_eq!(
            state
                .generate_waypoint([f32::NAN, 0., 0.], "bed")
                .unwrap_err()
                .code(),
            "invalid_position"
        );
        state.office.as_mut().unwrap().avatar.ready = false;
        assert_eq!(
            state.generate_waypoint("avatar", "bed").unwrap_err().code(),
            "avatar_not_ready"
        );
        assert!(state.generate_waypoint([0.; 3], "bed").is_ok());
        state.office.as_mut().unwrap().avatar.position_valid = true;
        assert!(
            state.generate_waypoint("avatar", "bed").is_ok(),
            "planning does not require a motion worker"
        );
        assert_eq!(
            SceneState::default()
                .generate_waypoint("chair", "desk")
                .unwrap_err()
                .code(),
            "scene_unavailable"
        );
    }

    #[test]
    fn snapshot_prop_transforms_and_occupied_hands_control_obstacles() {
        let mut state = scene();
        let office = state.office.as_mut().unwrap();
        let book = office.objects.iter_mut().find(|o| o.id == "book").unwrap();
        book.position = [0., 0.1, 1.];
        book.yaw_deg = 90.;
        let approach = book.approach_position();
        let path = state.generate_waypoint("avatar", "book").unwrap();
        assert_eq!(
            path.waypoints.last().unwrap(),
            &[approach[0], 0., approach[2]]
        );
        assert_eq!(
            state
                .generate_waypoint("avatar", [0., 0., 1.])
                .unwrap_err()
                .code(),
            "goal_blocked"
        );
        state
            .office
            .as_mut()
            .unwrap()
            .occupied_hands
            .push((Hand::Right, "book".into()));
        assert!(state.generate_waypoint("avatar", [0., 0., 1.]).is_ok());
    }

    #[test]
    fn malformed_queries_return_correlated_errors() {
        for value in [
            serde_json::json!({"kind":"generate_waypoint","from":"avatar","request_id":"bad"}),
            serde_json::json!({"kind":"generate_waypoint","from":[0,0],"to":"bed","request_id":"bad"}),
            serde_json::json!({"kind":"generate_waypoint","from":"avatar","to":"bed","move":true,"request_id":"bad"}),
        ] {
            let reply = query_reply(&scene(), &value);
            assert_eq!(reply["request_id"], "bad");
            assert_eq!(reply["ok"], false);
            assert_eq!(reply["error"]["code"], "invalid_request");
        }
    }
}
