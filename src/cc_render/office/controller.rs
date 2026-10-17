use super::super::renderer::{GrabCmd, GrabOrRelease, Hand, SceneState};
use super::{pathfinder, scene_metadata, starter_objects, OfficeObject};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
#[path = "surface_actions.rs"]
mod surface_actions;
#[path = "furniture_push.rs"]
mod furniture_push;
use furniture_push::PushPlan;
use surface_actions::SurfaceJump;

const FLOOR_GRASP_MAX_Y: f32 = 0.20;
/// Pelvis clearance from furniture footprints used for planning and goal checks.
const PLANNING_ROOT_RESERVE_M: f32 = 0.35;
/// Pelvis clearance that generated native motion must keep. Smaller than the
/// planning reserve on purpose: the office is game-like, so a model root that
/// drifts a few centimetres into the planning margin plays on. Only a pelvis
/// close enough for the body to pass into furniture rejects the candidate.
const NATIVE_ROOT_RESERVE_M: f32 = 0.20;
const KNEEL_TRANSITION_MS: u64 = 2000;
const KNEEL_SUPPORT_MS: u64 = 2400;
const FLOOR_CONTACT_MS: u64 = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Walk,
    JumpOnto,
    JumpDown,
    Push,
    Sit,
    StandUp,
    BendAndReturn,
    Wave,
    Grab,
    Release,
    Smile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRequest {
    pub action_id: String,
    pub verb: Verb,
    pub target: Option<String>,
    pub position: Option<[f32; 3]>,
    pub hand: Option<Hand>,
    #[serde(default = "duration")]
    pub duration_sec: f32,
    #[serde(default = "intensity")]
    pub intensity: f32,
}
fn duration() -> f32 {
    3.0
}
fn intensity() -> f32 {
    0.7
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OfficeCommand {
    /// Renderer-owned registration only; not deserializable from public commands.
    #[serde(skip)]
    InstallNativeSeatGeometry { geometry: std::sync::Arc<super::avatar_geometry::AvatarGeometry> },
    #[serde(skip)]
    AuthorizeNativeSeat { session_id: String, generation_revision: u64, target_id: String },
    /// Declared-interaction coupling from the scene lifecycle (renderer-owned).
    #[serde(skip)]
    Couple(super::interaction::CouplingCommand),
    NativeMotionRevision {
        generation_revision: u64,
    },
    /// Transport reservation of an existing native token, not authored motion.
    NativeSceneHandoff { generation_revision: u64, history_pts_ms: u64 },
    PerformAction {
        #[serde(flatten)]
        request: ActionRequest,
    },
    CancelAction {
        action_id: String,
    },
    EditMotionTimeline {
        edit_id: String,
        expected_revision: u64,
        layer: Option<super::motion_timeline::TimelineLayer>,
    },
    MotionError {
        error: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Accepted,
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Serialize)]
pub struct ActionEvent {
    pub sequence: u64,
    pub action_id: String,
    pub status: Status,
    pub detail: String,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Observation {
    pub ready: bool,
    /// A captured world-space pelvis observation, independent of worker readiness.
    pub position_valid: bool,
    pub pelvis: [f32; 3],
    pub wrists: [[f32; 3]; 2],
    pub heading: f32,
    pub displayed_pts_ms: u64,
    /// Identity of the pose actually applied, not the latest requested goal.
    pub native_stream: Option<super::timeline::StreamIdentity>,
    pub native_seat: Option<super::native_seat::SeatObservation>,
    /// Read-only native queue watermark; not a body target.
    pub buffered_until_pts_ms: Option<u64>,
    /// Transport acknowledgement: (revision, retained source-history timestamp).
    /// Set only after the timeline has applied the handoff; never a pose target.
    pub acknowledged_native_handoff: Option<(u64, u64)>,
    pub feet: [[f32; 3]; 2],
    pub torso_up: [f32; 3],
    /// Diagnostic canonical world deltas: chest, neck, head, observed after FK.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_rotation_deltas: Option<[[f32; 4]; 3]>,
    pub stale_frames: u64,
    pub underrun_frames: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JointTarget {
    pub joint: String,
    pub position: [f32; 3],
    pub rotation_xyzw: [f32; 4],
    pub root_relative: bool,
    pub rotation_only: bool,
    /// Rotate the target by the timeline heading's change from this reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_reference: Option<f32>,
    /// Absolute renderer-clock time at which contact begins and is maintained.
    pub contact_pts_ms: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct MotionRequest {
    pub handoff_pts_ms: Option<u64>,
    pub timeline: Option<super::motion_timeline::MotionTimeline>,
    pub kind: String,
    pub session_id: String,
    pub generation_revision: u64,
    pub pts_ms: u64,
    pub prompt: String,
    pub root_waypoints: Vec<[f32; 4]>,
    pub constrain_root_height: bool,
    pub ease_root: bool,
    pub joints: Vec<JointTarget>,
    pub observed_root: [f32; 3],
    pub posture: String,
    pub heading: Option<f32>,
}
#[derive(Clone, Debug)]
pub(crate) struct PushForceIntent {
    pub id: String,
    pub goal: Vec3,
    pub start: Vec3,
    pub avatar_start: Vec3,
    pub direction: Vec3,
    pub contact_offset: Vec3,
    pub intensity: f32,
}
#[derive(Clone, Debug, Serialize)]
pub struct OfficeSnapshot {
    pub native_seat_profile: Option<String>,
    pub motion_error: Option<String>,
    pub motion_rejection: Option<serde_json::Value>,
    pub timeline_edit_ids: Vec<String>,
    pub earliest_edit_pts_ms: u64,
    pub objects: Vec<OfficeObject>,
    /// Exact coarse-navigation membership, including observed held-prop exclusions.
    pub navigation_object_ids: Vec<String>,
    pub world_keypoints: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, super::InteractionKeypoint>,
    >,
    pub avatar: Observation,
    pub posture: String,
    pub standing_surface: Option<super::support::StandingSurface>,
    pub events: Vec<ActionEvent>,
    pub active: Vec<ActionRequest>,
    pub queued: Vec<ActionRequest>,
    pub motion: MotionRequest,
    pub occupied_hands: Vec<(Hand, String)>,
    pub couplings: Vec<Coupling>,
    /// What each holding hand is doing: seat error and finger closure (openspec add-conforming-hand-grasps).
    #[serde(default)]
    pub grasps: Vec<super::hand::GraspReport>,
    pub support_object: Option<String>,
    pub homes: std::collections::BTreeMap<String, [f32; 7]>,
}
#[derive(Clone, Debug)]
struct Active {
    jump: Option<SurfaceJump>,
    push: Option<PushPlan>,
    request: ActionRequest,
    started: f64,
    stage: Stage,
    origin_wrist: [f32; 3],
    origin_feet: [[f32; 3]; 2],
    execute_pts_ms: u64,
    execute_origin: [f32; 3],
    execute_heading: f32,
    stable_since_ms: Option<u64>,
    placement_prop: Option<String>,
    placement_sample: Option<(u64, [f32; 3])>,
    release_pts_ms: Option<u64>,
    moved: bool,
    sent: bool,
}
impl Active {
    fn approach_goal(&self, object: Option<&OfficeObject>) -> Option<[f32; 3]> {
        self.jump.as_ref().map(|j| j.from)
            .or_else(|| self.push.as_ref().and_then(|p| object.map(|o| (Vec3::from(o.position) + p.stage_offset).to_array())))
            .or(self.request.position)
            .or_else(|| object.map(OfficeObject::approach_position))
    }
    fn approach_direction(&self, object: Option<&OfficeObject>) -> Option<Vec2> {
        self.jump.as_ref().map(|jump| Vec2::new(jump.support.heading.sin(), jump.support.heading.cos()))
            .or_else(|| self.push.as_ref().map(|p| Vec2::new(p.direction.x, p.direction.z)))
            .or_else(|| object.map(super::locomotion::target_direction))
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Approach,
    Turn,
    Align,
    Contact,
    Execute,
    Settling,
    Standing,
    StandingSettling,
    Returning,
}
impl Stage {
    fn is_standing(&self) -> bool {
        matches!(self, Self::Standing | Self::StandingSettling)
    }
}
#[derive(Resource)]
pub struct OfficeController {
    pub motion_error: Option<String>,
    pub motion_rejection: Option<serde_json::Value>,
    pub native_motion_revision: Option<u64>,
    pub native_scene_handoff: Option<(u64, u64)>,
    native_seat: Option<super::native_seat::NativeSeatAdmission>,
    native_previous_pose: Option<super::super::renderer::SkeletalPose>,
    native_seat_candidate: Option<super::native_seat::SeatObservation>,
    pub future_root: Option<(u64, [f32; 3])>,
    pub future_handoffs: bool,
    /// Diagnostic comparison switch; natural navigation uses sparse facing guidance.
    pub natural_navigation: bool,
    pub natural_heading_interval_ms: u64,
    committed_motion: Option<MotionRequest>,
    timeline_layers: std::collections::BTreeMap<String, super::motion_timeline::TimelineLayer>,
    pub objects: Vec<OfficeObject>,
    pub observed: Observation,
    /// Parts with permanent physics joints remain obstacles during a hand grip.
    pub constrained_objects: std::collections::HashSet<String>,
    pub seated: Option<String>,
    /// Seat the native avatar was last measured on. Generated motion may step
    /// away from it (never deeper) until the pelvis clears its native reserve.
    pub leaving_seat: Option<String>,
    /// Objects attached to hands by declared interactions (posed each frame).
    pub couplings: Vec<Coupling>,
    /// What each holding hand is doing: seat error and finger closure (openspec add-conforming-hand-grasps).
    pub grasps: Vec<super::hand::GraspReport>,
    /// Object the avatar is authorized to stand on (support coupling).
    pub support_object: Option<String>,
    /// Authored (first observed) pose of every object: position + rotation xyzw.
    pub homes: std::collections::BTreeMap<String, [f32; 7]>,
    pub standing_surface: Option<super::support::StandingSurface>,
    surface_recovery_at_ms: Option<u64>,
    pub seat_supported: bool,
    seated_pose_at_ms: u64,
    pub motion_ready: bool,
    pub blocked: bool,
    pub last_replan: f64,
    active: Vec<Active>,
    pending: VecDeque<ActionRequest>,
    events: VecDeque<ActionEvent>,
    sequence: u64,
    pub revision: u64,
    session: String,
    pub now: f64,
    pub grab_commands: Vec<GrabOrRelease>,
    pub motion: MotionRequest,
    seen: std::collections::HashSet<String>,
    pub held: Vec<(Hand, String)>,
    held_since_ms: [Option<u64>; 2],
    /// Acquired world grip expressed relative to the body's heading at pickup.
    held_grip_rotations: [Option<Quat>; 2],
}
impl Default for OfficeController {
    fn default() -> Self {
        let session = format!(
            "office-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        Self {
            motion_error: None,
            motion_rejection: None,
            native_motion_revision: None,
            native_scene_handoff: None,
            native_seat: None,
            native_previous_pose: None,
            native_seat_candidate: None,
            future_root: None,
            future_handoffs: std::env::var("OFFICE_FUTURE_HANDOFFS").as_deref() == Ok("1"),
            natural_navigation: std::env::var("OFFICE_NATURAL_NAVIGATION").as_deref() != Ok("0"),
            natural_heading_interval_ms: std::env::var("OFFICE_NAVIGATION_HEADING_INTERVAL_MS")
                .ok().and_then(|value| value.parse().ok())
                .filter(|value| *value == 0 || (50..=10000).contains(value)).unwrap_or(800),
            committed_motion: None,
            timeline_layers: Default::default(),
            motion_ready: false,
            blocked: false,
            last_replan: -1.,
            objects: starter_objects(),
            constrained_objects: Default::default(),
            observed: Observation::default(),
            seated: None,
            leaving_seat: None,
            couplings: Vec::new(),
            grasps: Vec::new(),
            support_object: None,
            homes: home_poses(&starter_objects()),
            standing_surface: None,
            surface_recovery_at_ms: None,
            seat_supported: false,
            seated_pose_at_ms: 0,
            active: vec![],
            pending: VecDeque::new(),
            events: VecDeque::new(),
            sequence: 0,
            revision: 0,
            motion: MotionRequest {
                handoff_pts_ms: None,
                timeline: None,
                kind: "motion_request".into(),
                session_id: session.clone(),
                generation_revision: 0,
                pts_ms: 0,
                prompt: "An elegant woman stands still".into(),
                root_waypoints: vec![],
                constrain_root_height: false,
                ease_root: false,
                joints: vec![],
                observed_root: [0.; 3],
                posture: "standing".into(),
                heading: None,
            },
            held: vec![],
            held_since_ms: [None; 2],
            held_grip_rotations: [None; 2],
            session,
            now: 0.,
            grab_commands: vec![],
            seen: Default::default(),
        }
    }
}
fn action_heading(action: &Active, object: &super::OfficeObject) -> f32 {
    if let Some(push) = &action.push {
        return push.direction.x.atan2(push.direction.z);
    }
    if matches!(action.request.verb, Verb::BendAndReturn | Verb::Grab) {
        let target = object
            .grasp_transform()
            .map_or(Vec3::from(object.position), |t| t.translation);
        (target.x - action.execute_origin[0]).atan2(target.z - action.execute_origin[2])
    } else {
        object.yaw_deg.to_radians()
    }
}

/// Authored object poses, the targets of `@home/zone` references and plain releases.
pub fn home_poses(objects: &[super::OfficeObject]) -> std::collections::BTreeMap<String, [f32; 7]> {
    objects.iter().map(|o| {
        let t = o.transform();
        let (p, q) = (t.translation, t.rotation);
        (o.id.clone(), [p.x, p.y, p.z, q.x, q.y, q.z, q.w])
    }).collect()
}

fn hand_index(hand: Option<Hand>) -> usize {
    if hand == Some(Hand::Left) {
        0
    } else {
        1
    }
}

fn heading_turn_ms(from: f32, to: f32) -> u64 {
    let delta = to - from;
    (delta.sin().atan2(delta.cos()).abs() / 90_f32.to_radians() * 1000.)
        .ceil().max(1.) as u64
}

fn action_constrains_height(action: &Active) -> bool {
    match action.stage {
        Stage::Standing | Stage::StandingSettling => true,
        Stage::Turn | Stage::Align | Stage::Contact => action.request.verb != Verb::BendAndReturn,
        Stage::Execute | Stage::Settling => matches!(
            action.request.verb,
            Verb::Sit | Verb::Grab | Verb::Release | Verb::Push
        ),
        Stage::Returning => matches!(action.request.verb, Verb::BendAndReturn | Verb::Grab),
        Stage::Approach => false,
    }
}

fn standing_exit_settled(
    observed: &Observation,
    exit: Option<[f32; 3]>,
    execute_pts_ms: u64,
    stable_since_ms: &mut Option<u64>,
) -> bool {
    let settled = observed.displayed_pts_ms >= execute_pts_ms.saturating_add(2800)
        && observed.pelvis[1] > 0.85
        && observed.torso_up[1] > 0.95
        && exit.is_some_and(|p| {
            Vec2::new(p[0], p[2]).distance(Vec2::new(observed.pelvis[0], observed.pelvis[2])) < 0.08
        });
    if !settled {
        *stable_since_ms = None;
        return false;
    }
    observed
        .displayed_pts_ms
        .saturating_sub(*stable_since_ms.get_or_insert(observed.displayed_pts_ms))
        >= 400
}
fn resources(r: &ActionRequest) -> u8 {
    match r.verb {
        Verb::Smile => 8,
        Verb::Wave => 2 << hand_index(r.hand),
        Verb::Release => (if r.target.is_some() { 1 } else { 0 }) | (2 << hand_index(r.hand)),
        Verb::Grab => 1 | (2 << hand_index(r.hand)),
        Verb::BendAndReturn | Verb::JumpOnto | Verb::JumpDown | Verb::Push => 1 | 2 | 4,
        _ => 1,
    }
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    Vec3::from(a).distance(Vec3::from(b))
}

/// Check the released collider against the requested object's actual box tops,
/// not just its authored placement hint. The hint selects a nearby landing area;
/// collision geometry determines whether the prop is supported.
fn placement_prop(r: &ActionRequest, world: &SceneState) -> Result<Option<String>, &'static str> {
    if r.verb != Verb::Release || r.target.is_none() {
        return Ok(None);
    }
    world.grabs.iter().find(|g| g.hand == r.hand.unwrap_or(Hand::Right))
        .map(|g| Some(g.prop.clone())).ok_or("hand is not holding a prop")
}

fn placement_settled(
    surface: &OfficeObject, world: &SceneState, id: &str, observed_pts_ms: u64,
    stable_since_ms: &mut Option<u64>, sample: &mut Option<(u64, [f32; 3])>,
) -> bool {
    let Some(prop) = world.props.iter().find(|p| p.id == id && !p.loading) else { return false; };
    if !placement_supported(surface, prop) {
        *stable_since_ms = None;
        *sample = None;
        return false;
    }
    let center = (Vec3::from(prop.aabb_min) + Vec3::from(prop.aabb_max)) * 0.5;
    let steady = sample.is_none_or(|(pts, last)| {
        let dt = observed_pts_ms.saturating_sub(pts);
        // The office motion clock advances even without an ARKit face pose;
        // SceneState.pts_ms is the latter and can stay zero throughout playback.
        observed_pts_ms == pts || (observed_pts_ms > pts && dt <= 150
            && center.distance(Vec3::from(last)) <= 0.03 * dt as f32 / 1000. + 0.002)
    });
    if steady { stable_since_ms.get_or_insert(observed_pts_ms); }
    else { *stable_since_ms = None; }
    if sample.is_none_or(|(pts, _)| observed_pts_ms > pts) {
        *sample = Some((observed_pts_ms, center.to_array()));
    }
    stable_since_ms.is_some_and(|since| observed_pts_ms > since && observed_pts_ms - since >= 400)
}

fn placement_supported(surface: &OfficeObject, prop: &super::super::renderer::PropSnapshot) -> bool {
    let Some(anchor) = surface.placement_position() else { return false; };
    let min = Vec3::from(prop.aabb_min);
    let max = Vec3::from(prop.aabb_max);
    if !min.is_finite() || !max.is_finite() || !min.cmple(max).all() {
        return false;
    }
    let center = (min + max) * 0.5;
    if Vec2::new(center.x - anchor[0], center.z - anchor[2]).length() > 0.25 {
        return false;
    }
    let transform = surface.transform();
    // Authored office supports are upright boxes; tilted furniture needs a
    // different contact test rather than a false positive from a world AABB.
    if (transform.rotation * Vec3::Y).dot(Vec3::Y) < 0.99 {
        return false;
    }
    let local = transform.rotation.inverse() * (center - transform.translation);
    std::iter::once(([0.; 3], surface.half_extents))
        .chain(surface.parts.iter().map(|p| (p.position, p.half_extents)))
        .any(|(position, half)| {
            let top = surface.position[1] + position[1] + half[1];
            (min.y - top).abs() <= 0.06
                && local.x >= position[0] - half[0] + 0.02
                && local.x <= position[0] + half[0] - 0.02
                && local.z >= position[2] - half[2] + 0.02
                && local.z <= position[2] + half[2] - 0.02
        })
}
/// An object attached to one or more hands by a declared interaction. The
/// renderer poses it from the anchor wrists each frame (see systems::apply_couplings).
#[derive(Clone, Debug, Serialize)]
pub struct Coupling {
    pub object: String,
    /// Hand, object-local grip point and grip hint; the first anchor is primary.
    pub anchors: Vec<(Hand, [f32; 3], Option<super::interaction::Grip>)>,
    /// Sliding attach: keep the object on this world height (support plane).
    pub slide_height: Option<f32>,
    /// Pending release: `Some(None)` returns home, `Some(Some(zone))` places onto it.
    pub release: Option<Option<String>>,
    #[serde(skip)]
    pub latch: Option<CouplingLatch>,
}
#[derive(Clone, Copy, Debug)]
pub struct CouplingLatch {
    pub object_rotation: Quat,
    pub primary_rotation: Quat,
    pub previous_primary: Quat,
    pub object_height: f32,
    /// One-hand seated grip: (object in hand space at latch, seated pose, grip, blend
    /// progress 0..1). Progress only advances while seating is allowed.
    pub seat: Option<(Transform, Transform, super::interaction::Grip, f32)>,
    /// Two hands: held rigidly by the primary hand (hands too close to steer), and the
    /// frames left blending back to the line between the hands after that ends.
    pub rigid: bool,
    pub settle_frames: u8,
}

impl OfficeController {
    fn event(&mut self, id: &str, status: Status, detail: impl Into<String>) {
        self.sequence += 1;
        self.events.push_back(ActionEvent {
            sequence: self.sequence,
            action_id: id.into(),
            status,
            detail: detail.into(),
        });
        while self.events.len() > 128 {
            self.events.pop_front();
        }
    }
    pub fn command(&mut self, command: OfficeCommand, world: &SceneState) {
        match command {
            OfficeCommand::InstallNativeSeatGeometry { geometry } => {
                if self.native_seat.as_ref().is_some_and(|old| old.profile_identity() != geometry.profile_identity()) {
                    self.motion_error = Some("native seat calibration replacement rejected".into());
                } else if self.native_seat.is_none() {
                    self.native_seat = Some(super::native_seat::NativeSeatAdmission::new(geometry));
                }
            }
            OfficeCommand::Couple(command) => self.apply_coupling(command),
            OfficeCommand::AuthorizeNativeSeat { session_id, generation_revision, target_id } => {
                let result = self.native_seat.as_mut().ok_or_else(|| "native seat calibration unavailable".to_string())
                    .and_then(|admission| admission.authorize(super::native_seat::SeatScope { session_id, generation_revision, target_id }));
                if let Err(error) = result { self.motion_error = Some(error); }
            }
            OfficeCommand::NativeMotionRevision { generation_revision } => {
                self.native_motion_revision = Some(generation_revision);
                self.native_scene_handoff = None;
            }
            OfficeCommand::NativeSceneHandoff { generation_revision, history_pts_ms } => {
                self.native_scene_handoff = Some((generation_revision, history_pts_ms));
            }
            OfficeCommand::PerformAction { request } => {
                if let Some(error) = self.motion_error.clone() {
                    self.event(&request.action_id, Status::Failed, error);
                    return;
                }
                if self.seen.contains(&request.action_id) {
                    return;
                }
                if self.seen.len() >= 4096 {
                    self.event(
                        &request.action_id,
                        Status::Failed,
                        "session action limit reached",
                    );
                    return;
                }
                self.seen.insert(request.action_id.clone());
                if let Err(e) = self.validate(&request) {
                    self.event(&request.action_id, Status::Failed, e);
                    return;
                }
                self.event(&request.action_id, Status::Accepted, "");
                if self.pending.len() >= 32 {
                    self.event(&request.action_id, Status::Failed, "action queue full");
                    return;
                }
                self.event(&request.action_id, Status::Queued, "");
                self.pending.push_back(request);
                self.advance(world);
            }
            OfficeCommand::CancelAction { action_id } => {
                if self.defer_jump_cancel(&action_id) { return; }
                let old = self.active.len() + self.pending.len();
                if self.seat_supported {
                    if let Some(a) = self
                        .active
                        .iter()
                        .find(|a| a.request.action_id == action_id && a.request.verb == Verb::Sit)
                    {
                        self.seated = a.request.target.clone();
                    }
                }
                for a in &self.active {
                    if a.request.action_id == action_id && a.request.verb == Verb::Grab {
                        self.grab_commands.push(GrabOrRelease::CancelPursuit(
                            a.request.hand.unwrap_or(Hand::Right),
                        ));
                    }
                }
                self.active.retain(|a| a.request.action_id != action_id);
                self.pending.retain(|r| r.action_id != action_id);
                if old != self.active.len() + self.pending.len() {
                    self.future_root = None;
                    self.event(&action_id, Status::Cancelled, "");
                    self.rebuild(world);
                }
            }
            OfficeCommand::EditMotionTimeline {
                edit_id,
                expected_revision,
                layer,
            } => {
                if self.surface_owns_motion() {
                    self.event(&edit_id, Status::Failed, "surface traversal owns the motion timeline");
                    return;
                }
                if expected_revision != self.revision || edit_id.is_empty() || edit_id.len() > 128 {
                    self.event(
                        &edit_id,
                        Status::Failed,
                        "stale revision or invalid edit ID",
                    );
                    return;
                }
                if let Some(mut layer) = layer {
                    if let Err(error) = layer.validate(self.motion_start().0) {
                        self.event(&edit_id, Status::Failed, error);
                        return;
                    }
                    if self.timeline_layers.len() >= 16
                        && !self.timeline_layers.contains_key(&edit_id)
                    {
                        self.event(&edit_id, Status::Failed, "too many timeline layers");
                        return;
                    }
                    if self.timeline_layers.iter().any(|(id, existing)| {
                        id != &edit_id
                            && ((existing.root.is_some() && layer.root.is_some())
                                || (existing.prompts.is_some() && layer.prompts.is_some()))
                    }) {
                        self.event(
                            &edit_id,
                            Status::Failed,
                            "timeline channel already owned by another edit",
                        );
                        return;
                    }
                    if let Some(root) = &mut layer.root {
                        root.keyframes.insert(
                            0,
                            super::motion_timeline::RootKeyframe {
                                pts_ms: self.motion_start().0 as i64,
                                position: self.motion_start().1,
                            },
                        );
                    }
                    self.timeline_layers.insert(edit_id.clone(), layer);
                } else {
                    self.timeline_layers.remove(&edit_id);
                }
                self.rebuild(world);
                self.event(&edit_id, Status::Accepted, "future timeline updated");
            }
            OfficeCommand::MotionError { error } => {
                self.motion_error = Some(error.clone());
                self.motion_ready = false;
                self.observed.ready = false;
                let pending: Vec<_> = self.pending.drain(..).collect();
                for r in pending {
                    self.event(&r.action_id, Status::Failed, error.clone());
                }
                let actions = std::mem::take(&mut self.active);
                for a in actions {
                    if a.request.verb == Verb::Grab {
                        self.grab_commands.push(GrabOrRelease::CancelPursuit(
                            a.request.hand.unwrap_or(Hand::Right),
                        ));
                    }
                    self.event(&a.request.action_id, Status::Failed, error.clone());
                }
                self.rebuild(world);
            }
        }
    }
    fn validate(&self, r: &ActionRequest) -> Result<(), String> {
        if r.action_id.is_empty()
            || r.action_id.len() > 128
            || !r.duration_sec.is_finite()
            || !(0.1..=60.).contains(&r.duration_sec)
            || !r.intensity.is_finite()
            || !(0.0..=1.).contains(&r.intensity)
            || r.position.is_some_and(|p| !Vec3::from(p).is_finite())
        {
            return Err("invalid action options".into());
        }
        let obj = r
            .target
            .as_ref()
            .and_then(|id| self.objects.iter().find(|o| &o.id == id));
        match r.verb {
            Verb::Walk if r.position.is_none() && obj.is_none() => {
                Err("walk needs an object or position".into())
            }
            Verb::JumpOnto if obj.is_none() => Err("jump_onto needs a target".into()),
            Verb::Push => {
                let object = obj.ok_or("push needs a target")?;
                let goal = r.position.ok_or("push needs a destination")?;
                furniture_push::prepare(object, goal, &self.objects).map(|_| ())
            }
            Verb::Sit if obj.and_then(|o| o.seat).is_none() => Err("target has no seat".into()),
            Verb::Release if r.target.is_some() && obj.and_then(OfficeObject::placement_position).is_none() => {
                Err("target has no placement surface".into())
            }
            Verb::Grab if !obj.is_some_and(|o| o.dynamic && o.grasp_transform().is_some()) => {
                Err("target is not graspable".into())
            }
            _ => Ok(()),
        }
    }
    fn can_start(&self, r: &ActionRequest, world: &SceneState) -> bool {
        if !self.observed.ready {
            return false;
        }
        if self
            .active
            .iter()
            .any(|a| resources(&a.request) & resources(r) != 0)
        {
            return false;
        }
        let hand = r.hand.unwrap_or(Hand::Right);
        if matches!(r.verb, Verb::Wave | Verb::Grab) && world.grabs.iter().any(|g| g.hand == hand) {
            return false;
        }
        if r.verb == Verb::Grab
            && world
                .grabs
                .iter()
                .any(|g| Some(&g.prop) == r.target.as_ref())
        {
            return false;
        }
        true
    }
    fn start_action(&mut self, r: ActionRequest, world: &SceneState) -> bool {
        let placement_prop = match placement_prop(&r, world) {
            Ok(prop) => prop,
            Err(error) => {
                self.event(&r.action_id, Status::Failed, error);
                return false;
            }
        };
        if r.verb == Verb::Sit && self.seated.is_some() && self.seated == r.target {
            self.event(&r.action_id, Status::Completed, "already seated");
            return false;
        }
        let push = if r.verb == Verb::Push {
            match self.objects.iter().find(|o| Some(&o.id) == r.target.as_ref())
                .and_then(|o| r.position.map(|p| furniture_push::prepare(o, p, &self.objects))) {
                Some(Ok(plan)) => Some(plan),
                _ => { self.event(&r.action_id, Status::Failed, "push target or route changed"); return false; }
            }
        } else { None };
        let jump = match self.prepare_surface_action(&r) {
            Ok(jump) => jump,
            Err(error) => { self.event(&r.action_id, Status::Failed, error); return false; }
        };
        let stage = if r.verb == Verb::Push {
            Stage::Turn
        } else if r.verb == Verb::JumpOnto {
            Stage::Approach
        } else if self.seated.is_some()
            && matches!(
                r.verb,
                Verb::Walk
                    | Verb::Sit
                    | Verb::Grab
                    | Verb::StandUp
                    | Verb::Release
                    | Verb::BendAndReturn
            ) {
            Stage::Standing
        } else if matches!(r.verb, Verb::Walk | Verb::Sit | Verb::Grab | Verb::Push)
            || (r.verb == Verb::Release && r.target.is_some())
        {
            Stage::Approach
        } else if r.verb == Verb::BendAndReturn && r.target.is_some() {
            Stage::Align
        } else {
            Stage::Execute
        };
        self.event(&r.action_id, Status::Running, "");
        if r.verb == Verb::Release && r.target.is_none() {
            self.grab_commands
                .push(GrabOrRelease::Release(r.hand.unwrap_or(Hand::Right)));
        }
        self.active.push(Active {
            jump,
            push,
            origin_wrist: self.observed.wrists[hand_index(r.hand)],
            origin_feet: self.observed.feet,
            execute_pts_ms: self.motion_start().0,
            execute_origin: self.motion_start().1,
            execute_heading: self.observed.heading,
            stable_since_ms: None,
            placement_prop,
            placement_sample: None,
            release_pts_ms: None,
            request: r,
            started: self.now,
            stage,
            moved: false,
            sent: false,
        });
        true
    }
    pub fn advance(&mut self, world: &SceneState) {
        let held: Vec<_> = world
            .grabs
            .iter()
            .map(|g| (g.hand, g.prop.clone()))
            .collect();
        let mut changed = false;
        for (index, hand) in [Hand::Left, Hand::Right].into_iter().enumerate() {
            let previous = self.held.iter().find(|(h, _)| *h == hand);
            let current = held.iter().find(|(h, _)| *h == hand);
            if previous != current {
                self.held_since_ms[index] = current.map(|_| self.observed.displayed_pts_ms);
                self.held_grip_rotations[index] = current
                    .and_then(|(_, id)| self.objects.iter().find(|o| &o.id == id))
                    .and_then(|o| o.grasp_transform())
                    .map(|grip| Quat::from_rotation_y(-self.observed.heading) * grip.rotation);
                changed = true;
            }
        }
        self.held = held;
        for _ in 0..self.pending.len() {
            let r = self.pending.pop_front().unwrap();
            if !self.can_start(&r, world) {
                self.pending.push_back(r);
                continue;
            }
            changed |= self.start_action(r, world);
        }
        let mut active = std::mem::take(&mut self.active);
        for mut a in active.drain(..) {
            let r = a.request.clone();
            let elapsed = self
                .observed
                .displayed_pts_ms
                .saturating_sub(a.execute_pts_ms) as f64
                / 1000.;
            if self.now - a.started > 60. {
                self.event(&r.action_id, Status::Failed, "observed-state timeout");
                changed = true;
                continue;
            }
            if self.observed.displayed_pts_ms < a.execute_pts_ms {
                self.active.push(a);
                continue;
            }
            if a.jump.is_some() {
                changed |= self.advance_surface_action(a);
                continue;
            }
            let obj = r
                .target
                .as_ref()
                .and_then(|id| self.objects.iter().find(|o| &o.id == id));
            let goal = a.approach_goal(obj);
            let mut done = false;
            if a.stage.is_standing() {
                if a.stage == Stage::Standing && elapsed >= 2.8 {
                    a.stage = Stage::StandingSettling;
                    changed = true;
                }
                let exit = self
                    .seated
                    .as_ref()
                    .and_then(|id| self.objects.iter().find(|o| &o.id == id))
                    .map(|o| o.approach_position());
                if standing_exit_settled(
                    &self.observed,
                    exit,
                    a.execute_pts_ms,
                    &mut a.stable_since_ms,
                ) {
                    self.seated = None;
                    a.stable_since_ms = None;
                    a.stage = Stage::Approach;
                    changed = true;
                    done = r.verb == Verb::StandUp;
                }
            } else if a.stage == Stage::Approach {
                if let Some(goal) = goal {
                    let start = Vec2::new(self.observed.pelvis[0], self.observed.pelvis[2]);
                    let end = Vec2::new(goal[0], goal[2]);
                    if let Some(stage) = self.approach_stage(&r, goal) {
                        a.stage = if r.verb == Verb::Push { Stage::Contact } else { stage };
                        a.execute_pts_ms = self.motion_start().0;
                        a.execute_origin = self.motion_start().1;
                        a.execute_heading = self.observed.heading;
                        changed = true;
                        done = r.verb == Verb::Walk;
                    } else {
                        let mut scene = self.navigation_scene();
                        if let Some(plan) = a.push.as_ref() {
                            scene.objects.retain(|o| o.id != plan.id);
                        }
                        if let Err(e) = super::locomotion::approach_path(&scene, start, end,
                            a.approach_direction(obj), super::locomotion::arrival_angle())
                    {
                        self.event(&r.action_id, Status::Failed, e.to_string());
                        changed = true;
                        continue;
                        }
                    }
                }
            } else if a.stage == Stage::Turn && r.verb == Verb::Push {
                if obj.is_some_and(|o| (self.observed.heading - action_heading(&a, o)).cos() > 0.12_f32.cos())
                    && self.observed.pelvis[1] > 0.85
                    && self.observed.displayed_pts_ms >= a.execute_pts_ms + 1000 {
                    a.stage = Stage::Approach;
                    a.execute_pts_ms = self.motion_start().0;
                    a.execute_origin = self.motion_start().1;
                    changed = true;
                }
            } else if a.stage == Stage::Align {
                let aligned = obj.is_some_and(|o| {
                    (self.observed.heading - action_heading(&a, o)).cos() > 0.12_f32.cos()
                        && self.observed.pelvis[1] > 0.85
                        && (r.verb != Verb::Sit || {
                            let near = o.seat_staging_position();
                            let pelvis = self.observed.pelvis;
                            seat_contacts_aligned(&self.observed, o)
                                && self.observed.displayed_pts_ms >= a.execute_pts_ms
                                    + heading_turn_ms(a.execute_heading, o.yaw_deg.to_radians()) + 1200
                                && Vec2::new(pelvis[0] - near[0], pelvis[2] - near[2]).length() < 0.1
                        })
                });
                let ready = if r.verb == Verb::Grab {
                    if aligned
                        && self.observed.torso_up[1] > 0.97
                        && self.observed.displayed_pts_ms >= a.execute_pts_ms + 800
                    {
                        self.observed.displayed_pts_ms.saturating_sub(
                            *a.stable_since_ms
                                .get_or_insert(self.observed.displayed_pts_ms),
                        ) >= 400
                    } else {
                        a.stable_since_ms = None;
                        false
                    }
                } else {
                    aligned
                };
                if ready {
                    a.stable_since_ms = None;
                    a.stage = if r.verb == Verb::Push { Stage::Contact } else { Stage::Execute };
                    a.execute_pts_ms = self.motion_start().0;
                    a.execute_origin = self.motion_start().1;
                    a.origin_feet = self.observed.feet;
                    changed = true;
                }
            } else if a.stage == Stage::Contact && r.verb == Verb::Push {
                if furniture_push::contact_ready(&mut a, obj, &self.observed) {
                    a.stage = Stage::Execute;
                    a.execute_pts_ms = self.motion_start().0;
                    a.execute_origin = self.motion_start().1;
                    a.stable_since_ms = None;
                    changed = true;
                } else {
                    if elapsed > 4.0 {
                        self.event(&r.action_id, Status::Failed, "hands did not establish stable furniture contact");
                        changed = true;
                        continue;
                    }
                }
            } else if a.stage == Stage::Execute
                && r.verb == Verb::Sit
                && self.observed.displayed_pts_ms >= a.execute_pts_ms + 2400
            {
                a.stage = Stage::Settling;
                changed = true;
            } else {
                done = match r.verb {
                    Verb::Sit => {
                        let settled = self.observed.displayed_pts_ms >= a.execute_pts_ms + 2400
                            && obj
                                .and_then(|o| o.seat_position())
                                .is_some_and(|p| distance(p, self.observed.pelvis) < 0.07)
                            && obj.is_some_and(|o| {
                                (self.observed.heading - o.yaw_deg.to_radians()).cos()
                                    > 0.15_f32.cos()
                                    && seat_contacts_aligned(&self.observed, o)
                            });
                        if settled {
                            let since = a
                                .stable_since_ms
                                .get_or_insert(self.observed.displayed_pts_ms);
                            self.observed.displayed_pts_ms.saturating_sub(*since) >= 400
                        } else {
                            a.stable_since_ms = None;
                            false
                        }
                    }
                    Verb::StandUp => self.observed.pelvis[1] > 0.85,
                    Verb::BendAndReturn => {
                        let phase_ms = self
                            .observed
                            .displayed_pts_ms
                            .saturating_sub(a.execute_pts_ms);
                        if a.stage != Stage::Returning
                            && phase_ms >= 1400 + (r.duration_sec * 1000.) as u64
                        {
                            a.stage = Stage::Returning;
                            a.execute_pts_ms = self.motion_start().0;
                            a.execute_origin = self.motion_start().1;
                            changed = true;
                            false
                        } else {
                            a.stage == Stage::Returning
                                && phase_ms >= 1400
                                && self.observed.pelvis[1] > 0.9
                                && self.observed.torso_up[1] > 0.94
                        }
                    }
                    Verb::Grab => {
                        if !a.sent
                            && self.observed.displayed_pts_ms >= self.motion.joints.iter()
                                .find(|j| j.joint == if hand_index(r.hand) == 0 { "LeftHand" } else { "RightHand" })
                                .map_or(a.execute_pts_ms + 800, |j| j.contact_pts_ms)
                            && obj
                                .and_then(|o| o.grasp_transform().map(|p| p.translation.to_array()))
                                .is_some_and(|p| {
                                    distance(p, self.observed.wrists[hand_index(r.hand)]) < 0.03
                                })
                        {
                            self.grab_commands.push(GrabOrRelease::Grab(GrabCmd {
                                hand: r.hand.unwrap_or(Hand::Right),
                                target: r.target.clone().unwrap(),
                                palm_offset_override: Some([0.; 3]),
                            }));
                            a.sent = true;
                        }
                        let held = world.grabs.iter().any(|g| {
                            g.hand == r.hand.unwrap_or(Hand::Right)
                                && Some(&g.prop) == r.target.as_ref()
                        });
                        if held && a.stage != Stage::Returning {
                            a.stage = Stage::Returning;
                            a.execute_pts_ms = self.motion_start().0;
                            a.execute_origin = self.motion_start().1;
                            a.execute_heading = self.observed.heading;
                            a.stable_since_ms = None;
                            changed = true;
                        }
                        let wrist = Quat::from_rotation_y(-a.execute_heading)
                            * (Vec3::from(self.observed.wrists[hand_index(r.hand)]) - Vec3::from(self.observed.pelvis));
                        let recovered = held && a.stage == Stage::Returning
                            && self.observed.displayed_pts_ms >= a.execute_pts_ms + 1200
                            && self.observed.pelvis[1] > 0.9 && self.observed.torso_up[1] > 0.95
                            && wrist.y > -0.15 && wrist.z > 0.05;
                        if recovered {
                            self.observed.displayed_pts_ms.saturating_sub(
                                *a.stable_since_ms.get_or_insert(self.observed.displayed_pts_ms)) >= 400
                        } else {
                            a.stable_since_ms = None;
                            false
                        }
                    }
                    Verb::Release => {
                        let held = world.grabs.iter().any(|g| {
                            g.hand == r.hand.unwrap_or(Hand::Right)
                                && a.placement_prop.as_ref().is_none_or(|id| &g.prop == id)
                        });
                        if held && !a.sent
                            && obj.and_then(OfficeObject::placement_position).is_some_and(|p| {
                                distance(p, self.observed.wrists[hand_index(r.hand)]) < 0.1
                            })
                        {
                            self.grab_commands.push(GrabOrRelease::Release(r.hand.unwrap_or(Hand::Right)));
                            a.sent = true;
                            a.release_pts_ms = Some(self.observed.displayed_pts_ms);
                        }
                        match (a.placement_prop.as_deref(), obj) {
                            (Some(id), Some(surface)) if a.sent && !held => placement_settled(
                                surface, world, id, self.observed.displayed_pts_ms,
                                &mut a.stable_since_ms, &mut a.placement_sample,
                            ),
                            (Some(_), _) => false,
                            (None, _) => !held,
                        }
                    }
                    Verb::Smile => elapsed >= r.duration_sec as f64,
                    Verb::Wave => {
                        a.moved |=
                            distance(a.origin_wrist, self.observed.wrists[hand_index(r.hand)])
                                > 0.15;
                        if a.stage != Stage::Returning && elapsed >= r.duration_sec as f64 {
                            a.stage = Stage::Returning;
                            a.execute_pts_ms = self.motion_start().0;
                            a.stable_since_ms = None;
                            changed = true;
                        }
                        let returned = a.moved
                            && a.stage == Stage::Returning
                            && wave_hand_restored(&a, &self.observed, self.seated.is_some());
                        if self.seated.is_some() {
                            let supported = self.objects.iter().find(|o| Some(&o.id) == self.seated.as_ref())
                                .is_some_and(|o| seat_contacts_aligned(&self.observed, o)
                                    && o.seat_position().is_some_and(|p| distance(p, self.observed.pelvis) < 0.07)
                                    && (self.observed.heading - o.yaw_deg.to_radians()).cos() > 0.15_f32.cos());
                            if returned && supported && self.observed.displayed_pts_ms >= a.execute_pts_ms + 800 {
                                self.observed.displayed_pts_ms.saturating_sub(
                                    *a.stable_since_ms.get_or_insert(self.observed.displayed_pts_ms)) >= 400
                            } else {
                                a.stable_since_ms = None;
                                false
                            }
                        } else {
                            returned
                        }
                    }
                    Verb::Walk => true,
                    Verb::Push if a.stage == Stage::Returning =>
                        furniture_push::recovered(&mut a, &self.observed, elapsed),
                    Verb::Push => {
                        if furniture_push::settled(&mut a, &self.observed, world) {
                            a.stage = Stage::Returning;
                            a.execute_pts_ms = self.motion_start().0;
                            a.execute_origin = self.motion_start().1;
                            a.stable_since_ms = None;
                            changed = true;
                        }
                        false
                    }
                    Verb::JumpOnto | Verb::JumpDown => unreachable!("handled by surface traversal"),
                };
                if r.verb == Verb::Push && elapsed > r.duration_sec as f64 + 4. {
                    self.event(&r.action_id, Status::Failed, "furniture did not reach and settle at the goal");
                    changed = true;
                    continue;
                }
                if r.verb == Verb::Release && a.placement_prop.is_some() && !a.sent
                    && !world.grabs.iter().any(|g| g.hand == r.hand.unwrap_or(Hand::Right)
                        && Some(&g.prop) == a.placement_prop.as_ref())
                {
                    self.event(&r.action_id, Status::Failed, "grip lost before placement");
                    changed = true;
                    continue;
                }
                if r.verb == Verb::Release && a.placement_prop.is_some()
                    && a.release_pts_ms.is_some_and(|pts|
                        self.observed.displayed_pts_ms.saturating_sub(pts) > 5000)
                {
                    self.event(&r.action_id, Status::Failed, "prop did not settle on requested surface");
                    changed = true;
                    continue;
                }
                if r.verb == Verb::Grab
                    && world.failed_grabs.iter().any(|g| {
                        g.hand == r.hand.unwrap_or(Hand::Right)
                            && Some(&g.target) == r.target.as_ref()
                    })
                {
                    self.event(&r.action_id, Status::Failed, "grab proximity timeout");
                    changed = true;
                    continue;
                }
            }
            if done {
                if r.verb == Verb::Sit {
                    self.seated = r.target.clone();
                }
                if self.seated.is_some() && matches!(r.verb, Verb::Sit | Verb::Wave) {
                    self.seated_pose_at_ms = self.observed.displayed_pts_ms;
                }
                self.event(&r.action_id, Status::Completed, "");
                changed = true;
            } else {
                self.active.push(a);
            }
        }
        if changed {
            self.rebuild(world);
        }
    }
    fn approach_stage(&self, request: &ActionRequest, goal: [f32; 3]) -> Option<Stage> {
        let delta = Vec2::new(self.observed.pelvis[0] - goal[0], self.observed.pelvis[2] - goal[2]);
        if self.natural_navigation && request.verb == Verb::Grab {
            let facing = self.objects.iter().find(|o| Some(&o.id) == request.target.as_ref())
                .map(super::locomotion::target_direction).unwrap_or(Vec2::ZERO);
            return (delta.length() < 0.65 && self.permits_root(Vec3::from(goal))
                && super::locomotion::within_arrival_cone(-delta, facing, super::locomotion::arrival_angle()))
                .then_some(Stage::Execute);
        }
        let tolerance = if matches!(request.verb, Verb::Grab | Verb::Release | Verb::Push) { 0.04 } else { 0.12 };
        (delta.length() < tolerance).then_some(if matches!(request.verb, Verb::Sit | Verb::Grab | Verb::Push) {
            Stage::Align
        } else { Stage::Execute })
    }

    fn grab_reach_path(&self, action: &Active, object: Option<&OfficeObject>) -> (Vec<[f32; 4]>, Option<f32>, u64) {
        if self.natural_navigation {
            let goal = object.map_or(action.execute_origin, OfficeObject::approach_position);
            let elapsed = (self.observed.displayed_pts_ms as f64 - action.execute_pts_ms as f64) as f32 / 1000.;
            let duration = super::locomotion::reach_duration(action.execute_origin, goal);
            (super::locomotion::reaching(action.execute_origin, goal, elapsed), None, (duration * 1000.).round() as u64)
        } else {
            (vec![[self.observed.pelvis[0], 0.78, self.observed.pelvis[2], 0.8]],
                object.map(|o| action_heading(action, o)), 800)
        }
    }

    fn motion_start(&self) -> (u64, [f32; 3]) {
        self.future_root
            .filter(|(pts, _)| self.future_handoffs && *pts >= self.observed.displayed_pts_ms + 150)
            .unwrap_or((self.observed.displayed_pts_ms, self.observed.pelvis))
    }
    pub(crate) fn push_force_intent(&self) -> Option<PushForceIntent> {
        if self.motion_error.is_some() || !self.observed.ready { return None; }
        let action = self.active.iter().find(|a| a.request.verb == Verb::Push && a.stage == Stage::Execute)?;
        let plan = action.push.as_ref()?;
        Some(PushForceIntent {
            id: plan.id.clone(), goal: plan.goal, start: plan.start,
            avatar_start: Vec3::from(action.execute_origin), direction: plan.direction,
            contact_offset: plan.contact_offset, intensity: action.request.intensity,
        })
    }
    pub fn display_motion(&self) -> &MotionRequest {
        if self
            .motion
            .handoff_pts_ms
            .is_some_and(|at| self.observed.displayed_pts_ms < at)
        {
            self.committed_motion.as_ref().unwrap_or(&self.motion)
        } else {
            &self.motion
        }
    }
    pub fn rebuild(&mut self, world: &SceneState) {
        let committed = self.display_motion().clone();
        self.revision += 1;
        let changing_posture = self.active.iter().any(|a| {
            (a.stage == Stage::Approach || a.stage.is_standing())
                || matches!(
                    a.request.verb,
                    Verb::Sit | Verb::Grab | Verb::Release | Verb::BendAndReturn | Verb::Push
                )
        });
        let mut prompt = vec![if changing_posture {
            "An elegant woman".to_string()
        } else if self.seated.is_some() {
            if self.active.iter().all(|a| a.request.verb == Verb::Smile) {
                if self.held.is_empty() {
                    "An elegant woman sits comfortably upright on the chair with both feet planted and a relaxed, graceful posture and hands resting naturally in her lap".to_string()
                } else {
                    "An elegant woman remains seated upright and still with both feet planted"
                        .to_string()
                }
            } else {
                "An elegant woman remains seated".to_string()
            }
        } else {
            "An elegant woman stands upright".to_string()
        }];
        let mut roots = vec![];
        let mut joints = vec![];
        let mut heading = None;
        for a in &self.active {
            let r = &a.request;
            let object = r
                .target
                .as_ref()
                .and_then(|id| self.objects.iter().find(|o| &o.id == id));
            if a.stage == Stage::Contact && r.verb == Verb::Push {
                let plan = a.push.as_ref().unwrap();
                prompt.push("stands at the furniture and reaches both hands toward the nearest broad surface, bracing the palms on it while keeping the feet planted".into());
                heading = Some(plan.direction.x.atan2(plan.direction.z));
                roots.push([a.execute_origin[0], a.execute_origin[1], a.execute_origin[2], 0.]);
                continue;
            }
            if matches!(a.stage, Stage::Turn | Stage::Align) {
                prompt.push(
                    if r.verb == Verb::Sit {
                        "turns smoothly to face away from the chair, takes small steps backward, then stops upright clear of the chair front with both feet planted on the floor; she has not sat down yet"
                    } else {
                        "stands gracefully upright with shoulders relaxed and arms resting at her sides, turns calmly in place and comes to a complete stop"
                    }.into(),
                );
                heading = object.map(|o| action_heading(a, o));
                if r.verb == Verb::Sit {
                    if let Some(o) = object {
                        let mut near = o.seat_staging_position();
                        near[1] = 0.95;
                        let elapsed = (self.observed.displayed_pts_ms as f64
                            - a.execute_pts_ms as f64) as f32
                            / 1000.;
                        let turn = heading_turn_ms(a.execute_heading, o.yaw_deg.to_radians()) as f32 / 1000.;
                        roots.extend([
                            [
                                a.execute_origin[0],
                                a.execute_origin[1],
                                a.execute_origin[2],
                                -elapsed,
                            ],
                            [a.execute_origin[0], a.execute_origin[1], a.execute_origin[2], turn - elapsed],
                            [near[0], near[1], near[2], turn + 1.2 - elapsed],
                        ]);
                    }
                }
                continue;
            }
            if a.stage.is_standing() {
                prompt.push(if a.stage == Stage::StandingSettling {
                    "stands still in a relaxed upright posture after stepping clear of the chair"
                        .into()
                } else {
                    "stands up and steps forward from the seat".into()
                });
                if let Some(o) = self
                    .seated
                    .as_ref()
                    .and_then(|id| self.objects.iter().find(|o| &o.id == id))
                {
                    let p = o.approach_position();
                    let seat = o.anchor(o.seat.unwrap());
                    let forward = o.seat_staging_position();
                    let elapsed = (self.observed.displayed_pts_ms as f64 - a.execute_pts_ms as f64)
                        as f32
                        / 1000.;
                    roots.extend([
                        [seat[0], seat[1], seat[2], -elapsed],
                        [forward[0], 0.75, forward[2], 0.9 - elapsed],
                        [forward[0], 0.95, forward[2], 1.8 - elapsed],
                        [p[0], 0.95, p[2], 2.8 - elapsed],
                    ]);
                }
                continue;
            }
            if a.stage == Stage::Approach {
                let goal = a.approach_goal(object);
                if let Some(goal) = goal {
                    if let Ok(path) = super::locomotion::approach_path(
                        &self.navigation_scene(),
                        Vec2::new(self.motion_start().1[0], self.motion_start().1[2]),
                        Vec2::new(goal[0], goal[2]),
                        a.approach_direction(object),
                        super::locomotion::arrival_angle(),
                    ) {
                        let t = self
                            .motion_start()
                            .0
                            .saturating_sub(self.observed.displayed_pts_ms)
                            as f32
                            / 1000.;
                        roots = if self.natural_navigation {
                            super::locomotion::walking(&self.navigation_scene(), &path, t)
                        } else {
                            walking_waypoints(&path, t, self.observed.heading)
                        };
                    }
                }
                prompt.push("turns toward the path and walks forward naturally, facing the direction of travel and turning smoothly around corners".into());
            } else {
                match r.verb {
                    Verb::Wave => prompt.push(format!(
                        "{} the {} hand{}",
                        if a.stage == Stage::Returning {
                            "lowers"
                        } else {
                            "raises"
                        },
                        if hand_index(r.hand) == 0 {
                            "left"
                        } else {
                            "right"
                        },
                        if a.stage == Stage::Returning {
                            if self.seated.is_some() {
                                " back to the lap and relaxes upright on the chair"
                            } else {
                                " back down to the side and relaxes the arm"
                            }
                        } else {
                            " to shoulder height and waves it side to side in greeting"
                        }
                    )),
                    Verb::Push => {
                        let plan = a.push.as_ref().expect("push plan validated at start");
                        if a.stage == Stage::Returning {
                            prompt.push("releases the furniture gently, straightens her torso and settles into a balanced upright stance with both feet on the floor".into());
                            heading = Some(plan.direction.x.atan2(plan.direction.z));
                            let p = a.execute_origin;
                            roots.push([p[0], p[1], p[2], 0.]);
                            continue;
                        }
                        let seconds = r.duration_sec;
                        let elapsed = (self.observed.displayed_pts_ms as f64 - a.execute_pts_ms as f64) as f32 / 1000.;
                        let displacement = plan.goal - plan.start;
                        let origin = Vec3::from(a.execute_origin);
                        let end = origin + displacement;
                        let waypoint_gait = std::env::var("OFFICE_PUSH_WAYPOINT_GAIT").as_deref() == Ok("1");
                        prompt.push(if waypoint_gait {
                            "walks forward step by step while pushing the chair with both palms against its moving surface; her feet alternately lift and plant, her hips travel with the chair, and her hands keep contact"
                        } else {
                            "plants her feet, leans naturally into the furniture with both hands against its near face and walks steadily forward to push it across the floor without lifting it"
                        }.into());
                        heading = Some(plan.direction.x.atan2(plan.direction.z));
                        roots.extend([
                            [origin.x, origin.y, origin.z, -elapsed],
                            [origin.x, origin.y, origin.z, 0.6 - elapsed],
                        ]);
                        if waypoint_gait {
                            let path = [
                                pathfinder::Waypoint { xz: Vec2::new(origin.x, origin.z) },
                                pathfinder::Waypoint { xz: Vec2::new(end.x, end.z) },
                            ];
                            let walk = super::locomotion::walking(&self.navigation_scene(), &path, 0.);
                            let travel = walk.last().map_or(0., |p| p[3]).max(0.001);
                            roots.extend(walk.into_iter().skip(1).map(|p| [
                                p[0], origin.y, p[2], 0.6 + p[3] * (seconds - 0.6) / travel - elapsed,
                            ]));
                        } else {
                            roots.push([end.x, origin.y, end.z, seconds - elapsed]);
                        }
                    }
                    Verb::Sit => {
                        prompt.push(if a.stage == Stage::Settling {
                            "sits gracefully upright on the chair, shoulders relaxed, both feet planted, with her hands resting naturally in her lap".into()
                        } else {
                            "keeps both feet planted and smoothly lowers her hips back onto the chair seat".into()
                        });
                        heading = object.map(|o| o.yaw_deg.to_radians());
                        if let Some(p) = object.and_then(|o| o.seat_position()) {
                            let o = object.unwrap();
                            let elapsed = (self.observed.displayed_pts_ms as f64
                                - a.execute_pts_ms as f64)
                                as f32
                                / 1000.;
                            let origin = a.execute_origin;
                            roots.extend([
                                [origin[0], origin[1], origin[2], -elapsed],
                                // Arrival has already been observed. Hold that
                                // committed position instead of correcting back
                                // to the ideal waypoint before lowering.
                                [origin[0], origin[1], origin[2], 1.0 - elapsed],
                                [p[0], p[1], p[2], 2.4 - elapsed],
                            ]);
                        }
                    }
                    Verb::BendAndReturn => {
                        let returning = a.stage == Stage::Returning;
                        prompt.push(if returning {
                            "straightens back up to a relaxed upright standing pose".into()
                        } else {
                            "bends forward at the waist with slightly bent knees and reaches down as if picking something up".into()
                        });
                        let yaw = object.map_or(self.observed.heading, |o| {
                            (o.position[0] - a.execute_origin[0])
                                .atan2(o.position[2] - a.execute_origin[2])
                        });
                        heading = Some(yaw);
                        let elapsed = (self.observed.displayed_pts_ms as f64
                            - a.execute_pts_ms as f64) as f32
                            / 1000.;
                        let p = a.execute_origin;
                        roots.extend([
                            [p[0], p[1], p[2], -elapsed],
                            [
                                p[0],
                                if returning { 0.95 } else { p[1] },
                                p[2],
                                1.4 - elapsed,
                            ],
                        ]);
                    }
                    Verb::Grab => {
                        if a.stage == Stage::Returning {
                            heading = Some(a.execute_heading);
                            let p = a.execute_origin;
                            let elapsed = (self.observed.displayed_pts_ms as f64 - a.execute_pts_ms as f64) as f32 / 1000.;
                            roots.extend([[p[0], p[1], p[2], -elapsed], [p[0], 0.95, p[2], 1.2 - elapsed]]);
                            prompt.push("straightens upright and brings the held object close in front of the body with a comfortably bent elbow".into());
                            continue;
                        }
                        // Blend the natural approach into reaching; legacy mode
                        // retains its crouch. Wrist proximity and the grab FSM
                        // still verify ownership changes.
                        let floor_grasp = self.timeline_layers.is_empty()
                            && object.and_then(OfficeObject::grasp_transform)
                                .is_some_and(|grasp| grasp.translation.y <= FLOOR_GRASP_MAX_Y);
                        let (path, facing, contact_delay) = self.grab_reach_path(a, object);
                        if floor_grasp {
                            roots = vec![[a.execute_origin[0], a.execute_origin[1], a.execute_origin[2], 0.]];
                            heading = None; // preserve the aligned facing in generated history
                            prompt.push("A woman kneels down onto both knees on the floor.".into());
                        } else {
                            roots = path;
                            heading = facing;
                            prompt.push(format!(
                                "leans forward and reaches with the {} hand to grasp the object",
                                if hand_index(r.hand) == 0 { "left" } else { "right" }
                            ));
                        }
                        if let Some(p) = object
                            .and_then(|o| o.grasp_transform().map(|p| p.translation.to_array()))
                        {
                            joints.push(JointTarget {
                                joint: if hand_index(r.hand) == 0 {
                                    "LeftHand"
                                } else {
                                    "RightHand"
                                }
                                .into(),
                                position: p,
                                rotation_xyzw: object
                                    .and_then(|o| o.grasp_transform())
                                    .map_or(Quat::IDENTITY, |t| t.rotation)
                                    .to_array(),
                                root_relative: false,
                                rotation_only: false,
                                heading_reference: None,
                                contact_pts_ms: a.execute_pts_ms + if floor_grasp { FLOOR_CONTACT_MS } else { contact_delay },
                            });
                        }
                    }
                    Verb::Release => {
                        if let Some(p) = object.and_then(|o| o.placement_position()) {
                            prompt.push("places the carried object on the surface".into());
                            joints.push(JointTarget {
                                joint: if hand_index(r.hand) == 0 {
                                    "LeftHand"
                                } else {
                                    "RightHand"
                                }
                                .into(),
                                position: p,
                                rotation_xyzw: [0., 0., 0., 1.],
                                root_relative: false,
                                rotation_only: false,
                                heading_reference: None,
                                contact_pts_ms: a.execute_pts_ms + 800,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        for g in &world.grabs {
            prompt.push(format!(
                "carries {} upright in the {:?} hand with the elbow bent naturally",
                g.prop, g.hand
            ));
            let joint = match g.hand {
                Hand::Left => "LeftHand",
                Hand::Right => "RightHand",
            };
            if joints.iter().any(|target| target.joint == joint) {
                continue; // An explicit placement/reach owns this hand's target.
            }
            // Continue the acquired world grip, turning only by the body's
            // heading change since pickup. Object-local grip rotation is not
            // body-local: multiplying it by absolute heading twists the wrist
            // as soon as a positional reach becomes a carrying contact.
            if let Some(object) = self.objects.iter().find(|o| o.id == g.prop) {
                let grip = self.held_grip_rotations[hand_index(Some(g.hand))]
                    .unwrap_or_else(|| Quat::from_rotation_y(-self.observed.heading)
                        * object.grasp_transform().map_or(Quat::IDENTITY, |t| t.rotation));
                joints.push(JointTarget {
                    joint: joint.into(),
                    position: [0.; 3],
                    rotation_xyzw: (Quat::from_rotation_y(
                        heading.unwrap_or(self.observed.heading),
                    ) * grip)
                        .to_array(),
                    root_relative: false,
                    rotation_only: true,
                    heading_reference: Some(heading.unwrap_or(self.observed.heading)),
                    // The grip persists across action revisions. Restarting
                    // this blend exposes uncorrected wrist rotations again.
                    contact_pts_ms: self.held_since_ms[hand_index(Some(g.hand))]
                        .unwrap_or(self.observed.displayed_pts_ms),
                });
            }
        }
        if let Some(id) = &self.seated {
            if let Some(o) = self.objects.iter().find(|o| &o.id == id) {
                heading = Some(o.yaw_deg.to_radians());
                if let Some(p) = o.seat_position() {
                    if roots.is_empty() {
                        roots.push([p[0], p[1], p[2], 0.]);
                    }
                }
            }
        }
        let contact_seat = self.seated.as_deref().or_else(|| {
            self.active
                .iter()
                .find(|a| {
                    a.request.verb == Verb::Sit
                        && matches!(a.stage, Stage::Align | Stage::Execute | Stage::Settling)
                })
                .and_then(|a| a.request.target.as_deref())
        });
        if !self.active.iter().any(|a| a.stage.is_standing()) {
            if let Some(o) = contact_seat.and_then(|id| self.objects.iter().find(|o| o.id == id)) {
                for (joint, x) in [("LeftFoot", 0.12), ("RightFoot", -0.12)] {
                    let mut p = o.anchor([x, 0., 0.45]);
                    p[1] = 0.08;
                    let anchor = o.world_keypoint(if joint == "LeftFoot" {
                        "left_foot"
                    } else {
                        "right_foot"
                    });
                    if let Some(anchor) = anchor {
                        p = anchor.translation.to_array();
                    }
                    p = o.seat_floor_foot(p);
                    joints.push(JointTarget {
                        joint: joint.into(),
                        position: p,
                        rotation_xyzw: anchor
                            .map_or(Quat::from_rotation_y(o.yaw_deg.to_radians()), |t| {
                                t.rotation
                            })
                            .to_array(),
                        root_relative: false,
                        rotation_only: false,
                        heading_reference: None,
                        contact_pts_ms: self
                            .active
                            .iter()
                            .find(|a| {
                                a.request.verb == Verb::Sit
                                    && matches!(
                                        a.stage,
                                        Stage::Align | Stage::Execute | Stage::Settling
                                    )
                            })
                            .map_or(0, |a| {
                                if a.stage == Stage::Align {
                                    a.execute_pts_ms + heading_turn_ms(a.execute_heading, o.yaw_deg.to_radians()) + 1200
                                } else {
                                    a.execute_pts_ms
                                }
                            }),
                    });
                }
            }
        }
        self.motion = MotionRequest {
            handoff_pts_ms: None,
            timeline: None,
            kind: "motion_request".into(),
            session_id: self.session.clone(),
            generation_revision: self.revision,
            pts_ms: self.observed.displayed_pts_ms,
            prompt: prompt
                .join(" and ")
                .replacen("An elegant woman and ", "An elegant woman ", 1),
            constrain_root_height: self.seated.is_some()
                || self.active.iter().any(|a| action_constrains_height(a)
                    && !(self.natural_navigation && a.request.verb == Verb::Grab && a.stage == Stage::Execute)),
            ease_root: self.active.iter().any(|a| {
                a.request.verb == Verb::BendAndReturn
                    || (a.request.verb == Verb::Grab && a.stage == Stage::Returning)
                    || a.stage.is_standing()
                    || (a.request.verb == Verb::Sit
                        && matches!(a.stage, Stage::Execute | Stage::Settling))
            }),
            root_waypoints: roots,
            joints,
            observed_root: self.observed.pelvis,
            posture: if self.seated.is_some() {
                "seated"
            } else if self.standing_surface.is_some() {
                "standing_on_surface"
            } else {
                "standing"
            }
            .into(),
            heading,
        };
        self.motion.handoff_pts_ms = self
            .future_root
            .filter(|(pts, _)| self.future_handoffs && *pts >= self.observed.displayed_pts_ms + 150)
            .map(|(pts, _)| pts);
        let mut timeline = super::motion_timeline::MotionTimeline::from_motion(&self.motion);
        if let Some(align) = self.active.iter().find(|a| a.stage == Stage::Align) {
            if let Some(target) = timeline.root.heading {
                timeline.root.heading_transition = Some(super::motion_timeline::HeadingTransition {
                    start_ms: align.execute_pts_ms,
                    duration_ms: heading_turn_ms(align.execute_heading, target),
                    from: align.execute_heading,
                });
            }
        }
        timeline.root.face_path = self.active.iter().any(|a| a.stage == Stage::Approach);
        if std::env::var("OFFICE_PUSH_WAYPOINT_GAIT").as_deref() == Ok("1")
            && self.active.iter().any(|a| a.request.verb == Verb::Push && a.stage == Stage::Execute) {
            timeline.root.heading = None;
            timeline.root.face_path = true;
        }
        self.guide_natural_heading(&mut timeline);
        if timeline.root.face_path && timeline.root.keyframes.len() >= 2 {
            let first = &timeline.root.keyframes[0];
            let second = &timeline.root.keyframes[1];
            if first.position[0] == second.position[0] && first.position[2] == second.position[2] {
                timeline.root.heading_transition = Some(super::motion_timeline::HeadingTransition {
                    start_ms: first.pts_ms as u64,
                    duration_ms: (second.pts_ms - first.pts_ms) as u64,
                    from: self.observed.heading,
                });
            }
        }
        if !self.natural_navigation && timeline.root.face_path && self.timeline_layers.is_empty() {
            if let Some(end) = timeline.root.keyframes.last() {
                if let Some(previous) = timeline.root.keyframes.iter().rev().find(|key| {
                    Vec2::new(key.position[0] - end.position[0], key.position[2] - end.position[2]).length() > 0.001
                }) {
                    let heading = (end.position[0] - previous.position[0])
                        .atan2(end.position[2] - previous.position[2]);
                    // The route ends at rest. A root-only stop admits raised
                    // arms and folded torsos, especially on a short approach.
                    timeline.pose_keyframes.push(serde_json::json!({
                        "kind":"model_pose", "posture":"standing", "hold":true, "pts_ms":end.pts_ms,
                        "visible_from_ms":timeline.root.keyframes[0].pts_ms,
                        "root_position":end.position, "heading":heading,
                        "carry_hands":self.held.iter().map(|(hand, _)| match hand {
                            Hand::Left => "Left", Hand::Right => "Right"
                        }).collect::<Vec<_>>(),
                        "free_hands":self.active.iter().filter(|a| a.request.verb == Verb::Wave)
                            .map(|a| if hand_index(a.request.hand) == 0 { "Left" } else { "Right" })
                            .collect::<Vec<_>>()
                    }));
                }
            }
        }
        // Use the native hand constraint as in ARDY's GUI. Its finger endpoint
        // positions make wrist position/orientation a coherent contact frame.
        for target in self
            .motion
            .joints
            .iter()
            .filter(|j| j.joint.ends_with("Hand") && !j.root_relative && !j.rotation_only)
        {
            timeline.pose_keyframes.push(serde_json::json!({
                "kind":"end_effectors", "hold":true, "pts_ms":target.contact_pts_ms,
                "heading":self.motion.heading.unwrap_or(self.observed.heading), "keypoints":[{
                    "joint":target.joint,"position":target.position,"rotation_xyzw":target.rotation_xyzw
                }]
            }));
        }
        timeline
            .contacts
            .retain(|j| !j.joint.ends_with("Hand") || j.root_relative || j.rotation_only);
        self.schedule_floor_grab(&mut timeline);
        for recovery in self.active.iter().filter(|a| a.request.verb == Verb::Grab && a.stage == Stage::Returning && self.timeline_layers.is_empty()) {
            let mut root = recovery.execute_origin;
            root[1] = 0.95;
            timeline.pose_keyframes.push(serde_json::json!({
                "kind":"model_pose", "posture":"standing", "hold":true,
                "pts_ms":recovery.execute_pts_ms + 1200,
                "visible_from_ms":recovery.execute_pts_ms,
                "heading":recovery.execute_heading, "root_position":root,
                "carry_hands":self.held.iter().map(|(hand, _)| match hand {
                    Hand::Left => "Left", Hand::Right => "Right"
                }).collect::<Vec<_>>()
            }));
        }
        for standing in self.active.iter().filter(|a| a.stage.is_standing() && self.timeline_layers.is_empty()) {
            if let Some(seat) = self.seated.as_ref()
                .and_then(|id| self.objects.iter().find(|o| &o.id == id))
            {
                // A pelvis path alone admits folded torsos and trailing arms.
                // Native coherent endpoints guide the generated rise and exit;
                // these are point targets, not enforced or held body poses.
                for (offset, mut root) in [
                    (1800, seat.seat_staging_position()),
                    (2800, seat.approach_position()),
                ] {
                    root[1] = 0.95;
                    timeline.pose_keyframes.push(serde_json::json!({
                        "kind":"model_pose", "posture":"standing", "hold":true,
                        "pts_ms":standing.execute_pts_ms + offset,
                        "visible_from_ms":standing.execute_pts_ms,
                        "heading":seat.yaw_deg.to_radians(), "root_position":root,
                        "carry_hands":self.held.iter().map(|(hand, _)| match hand {
                            Hand::Left => "Left", Hand::Right => "Right"
                        }).collect::<Vec<_>>()
                    }));
                }
            }
        }
        for align in self
            .active
            .iter()
            .filter(|a| a.request.verb == Verb::Grab && a.stage == Stage::Align)
        {
            if self.timeline_layers.is_empty()
                && self
                    .active
                    .iter()
                    .all(|a| matches!(a.request.verb, Verb::Grab | Verb::Smile))
            {
                if let Some(object) = self
                    .objects
                    .iter()
                    .find(|o| Some(&o.id) == align.request.target.as_ref())
                {
                    let mut root = object.approach_position();
                    root[1] = 0.95;
                    timeline.pose_keyframes.push(serde_json::json!({
                        "kind":"model_pose", "posture":"standing", "hold":true, "pts_ms":align.execute_pts_ms + 800,
                        "visible_from_ms":align.execute_pts_ms,
                        "heading":self.motion.heading.unwrap_or(self.observed.heading),
                        "root_position":root,
                        "carry_hands":self.held.iter().map(|(hand, _)| match hand {
                            Hand::Left => "Left", Hand::Right => "Right"
                        }).collect::<Vec<_>>()
                    }));
                }
            }
        }
        for action in self.active.iter().filter(|a| a.request.verb == Verb::Push
            && a.stage == Stage::Returning) {
            let plan = action.push.as_ref().unwrap();
            let mut root = action.execute_origin;
            root[1] = 0.95;
            timeline.pose_keyframes.push(serde_json::json!({
                "kind":"model_pose", "posture":"standing", "pts_ms":action.execute_pts_ms + 1600,
                "visible_from_ms":action.execute_pts_ms, "root_position":root,
                "heading":plan.direction.x.atan2(plan.direction.z)
            }));
        }
        for action in self.active.iter().filter(|a| a.request.verb == Verb::Push
            && matches!(a.stage, Stage::Contact | Stage::Execute)) {
            let plan = action.push.as_ref().unwrap();
            let contact = plan.start + plan.contact_offset;
            let total_ms = if action.stage == Stage::Contact { 1800 }
                else { (action.request.duration_sec * 1000.) as u64 };
            let continuous = action.stage == Stage::Execute && furniture_push::long_probe_enabled();
            for offset_ms in furniture_push::hand_constraint_times(total_ms, continuous) {
                let fraction = if action.stage == Stage::Contact { 0. }
                    else { ((offset_ms as f32 - 600.) / (total_ms as f32 - 600.)).clamp(0., 1.) };
                let at = contact + (plan.goal - plan.start) * fraction;
                let pts = action.execute_pts_ms + offset_ms;
                let hold = action.stage == Stage::Contact && offset_ms == total_ms;
                timeline.pose_keyframes.push(serde_json::json!({
                    "kind":"end_effectors", "pts_ms":pts, "hold":hold,
                    "visible_from_ms":action.execute_pts_ms,
                    "heading":plan.direction.x.atan2(plan.direction.z),
                    "keypoints":[
                        {"joint":"LeftHand", "position":(at + Vec3::new(plan.direction.z, 0., -plan.direction.x) * 0.18).to_array()},
                        {"joint":"RightHand", "position":(at - Vec3::new(plan.direction.z, 0., -plan.direction.x) * 0.18).to_array()}
                    ]
                }));
            }
        }
        // Native end-effector masks constrain only scene contacts. They do not
        // carry an authored seated/standing skeleton or fix intermediate joints.
        if let Some(o) = contact_seat.and_then(|id| self.objects.iter().find(|o| o.id == id)) {
            if !self.active.iter().any(|a| a.stage.is_standing()) {
                for target in &self.motion.joints {
                    if target.joint.ends_with("Foot") {
                        timeline.pose_keyframes.push(serde_json::json!({
                            "kind":"end_effectors", "hold":true, "pts_ms":target.contact_pts_ms,
                            "heading":o.yaw_deg.to_radians(), "keypoints":[{
                                "joint":target.joint,"position":target.position,"rotation_xyzw":target.rotation_xyzw
                            }]
                        }));
                    }
                }
                let contact = self.active.iter().find(|a| a.request.verb == Verb::Sit);
                if let Some(align) = contact.filter(|a| a.stage == Stage::Align) {
                    let mut root = o.seat_staging_position();
                    root[1] = 0.95;
                    timeline.pose_keyframes.push(serde_json::json!({
                        "kind":"model_pose", "posture":"standing", "hold":true,
                        "pts_ms":align.execute_pts_ms + heading_turn_ms(align.execute_heading, o.yaw_deg.to_radians()) + 1200,
                        "visible_from_ms":align.execute_pts_ms,
                        "root_position":root, "heading":o.yaw_deg.to_radians()
                    }));
                    // Use an ARDY-generated upright destination rather than a
                    // lone hip contact that lets the model pre-emptively sit and
                    // drift behind the staged root before Execute begins.
                }
                if contact.is_none_or(|a| a.stage != Stage::Align) {
                    if let Some(root) = o.seat_position() {
                        timeline.pose_keyframes.push(serde_json::json!({
                            "kind":"end_effectors", "hold":true,
                            "pts_ms":contact.map_or(0, |a| a.execute_pts_ms + 2400),
                            "heading":o.yaw_deg.to_radians(), "keypoints":[{
                                "joint":"Hips","position":root,
                                "rotation_xyzw":o.world_keypoint("seat_pelvis")
                                    .map_or(Quat::from_rotation_y(o.yaw_deg.to_radians()), |t| t.rotation).to_array()
                            }]
                        }));
                        let settling = contact.filter(|a| a.stage == Stage::Settling);
                        if let Some(sitting) = contact.filter(|a| a.stage == Stage::Execute && self.timeline_layers.is_empty()) {
                            timeline.pose_keyframes.push(serde_json::json!({
                                "kind":"model_pose", "posture":"seated", "hold":true,
                                "pts_ms":sitting.execute_pts_ms + 2400,
                                "visible_from_ms":sitting.execute_pts_ms,
                                "heading":o.yaw_deg.to_radians(), "root_position":root,
                                "carry_hands":self.held.iter().map(|(hand, _)| match hand {
                                    Hand::Left => "Left", Hand::Right => "Right"
                                }).collect::<Vec<_>>()
                            }));
                        }
                        // Gestures continue from motion history with only the
                        // scene contacts above, never a frozen seated body.
                        if (self.seated.is_some() || settling.is_some())
                            && self.active.iter().all(|a| {
                                a.request.verb == Verb::Smile
                                    || (a.request.verb == Verb::Sit && a.stage == Stage::Settling)
                            })
                            && self.timeline_layers.is_empty()
                        {
                            let start = settling
                                .map_or(self.seated_pose_at_ms, |a| a.execute_pts_ms + 2400);
                            timeline.pose_keyframes.push(serde_json::json!({
                                "kind":"model_pose", "posture":"seated", "hold":true,
                                "pts_ms":start + 800, "visible_from_ms":start,
                                "root_position":root, "heading":o.yaw_deg.to_radians(),
                                "carry_hands":self.held.iter().map(|(hand, _)| match hand {
                                    Hand::Left => "Left", Hand::Right => "Right"
                                }).collect::<Vec<_>>()
                            }));
                        }
                    }
                }
                // Retain renderer contact metadata, but avoid duplicate model masks.
                timeline.contacts.retain(|j| !j.joint.ends_with("Foot"));
            }
        }
        for a in self.active.iter().filter(|a| {
            a.request.verb == Verb::BendAndReturn
                && matches!(a.stage, Stage::Execute | Stage::Returning)
                && self.timeline_layers.is_empty()
        }) {
            let heading = self.motion.heading.unwrap_or(self.observed.heading);
            let returning = a.stage == Stage::Returning;
            let mut root = a.execute_origin;
            root[1] = if returning { 0.95 } else { a.execute_origin[1] };
            // Guide the whole pose at the root trajectory's deadline. Feet plus
            // an independent wrist target left the shoulders free to flail;
            // returning previously constrained only the feet. Native guidance
            // keeps the transition generated rather than enforcing a static pose.
            timeline.pose_keyframes.push(serde_json::json!({
                "kind":"model_pose", "posture":if returning { "standing" } else { "bend" },
                "hold":true, "pts_ms":a.execute_pts_ms + 1400,
                "visible_from_ms":a.execute_pts_ms, "heading":heading, "root_position":root,
                "carry_hands":self.held.iter().map(|(hand, _)| match hand {
                    Hand::Left => "Left", Hand::Right => "Right"
                }).collect::<Vec<_>>()
            }));
        }
        self.constrain_standing_idle(&mut timeline);
        self.apply_surface_motion(&mut timeline);
        for layer in self.timeline_layers.values() {
            layer.apply(&mut timeline);
        }
        self.motion.timeline = Some(timeline);
        self.committed_motion = self.motion.handoff_pts_ms.map(|_| committed);
    }

    /// Stage a floor grasp with a model-sampled destination, then support
    /// sampled from ARDY's committed history. Only the final hand target is
    /// specified by the object; no task-specific body rotations are authored.
    fn schedule_floor_grab(&self, timeline: &mut super::motion_timeline::MotionTimeline) {
        let Some(action) = self.active.iter().find(|a| {
            a.stage == Stage::Execute && a.request.verb == Verb::Grab
                && self.timeline_layers.is_empty()
                && a.request.target.as_ref().and_then(|id| self.objects.iter().find(|o| &o.id == id))
                    .and_then(OfficeObject::grasp_transform)
                    .is_some_and(|grasp| grasp.translation.y <= FLOOR_GRASP_MAX_Y)
        }) else { return };
        let kneel = action.execute_pts_ms + KNEEL_TRANSITION_MS;
        let support = action.execute_pts_ms + KNEEL_SUPPORT_MS;
        let contact = action.execute_pts_ms + FLOOR_CONTACT_MS;
        let start = self.motion.pts_ms;
        let hand = if hand_index(action.request.hand) == 0 { "LeftHand" } else { "RightHand" };
        timeline.root.keyframes = vec![
            super::motion_timeline::RootKeyframe { pts_ms: start as i64, position: self.motion.observed_root },
            super::motion_timeline::RootKeyframe { pts_ms: kneel as i64, position: action.execute_origin },
        ];
        timeline.root.position_mode = "keyframes".into();
        timeline.root.constrain_height = false;
        timeline.root.heading = None;
        timeline.root.heading_transition = None;
        timeline.root.face_path = false;
        timeline.root.heading_interval_ms = 0;
        timeline.prompts = vec![
            super::motion_timeline::PromptSpan { start_ms: start, end_ms: Some(support),
                text: "A woman kneels down onto both knees on the floor.".into() },
            super::motion_timeline::PromptSpan { start_ms: support, end_ms: None,
                text: "A woman is kneeling on the floor.".into() },
        ];
        timeline.pose_keyframes.retain(|k| k["kind"] != "end_effectors"
            || !k["keypoints"].as_array().is_some_and(|points|
                points.iter().any(|p| p["joint"] == hand)));
        timeline.pose_keyframes.push(serde_json::json!({
            "kind":"generated_posture", "posture":"kneeling", "pts_ms":kneel,
            "visible_from_ms":start, "heading":action.execute_heading
        }));
        timeline.pose_keyframes.push(serde_json::json!({
            "kind":"observed_support", "pts_ms":support
        }));
        if let Some(target) = self.motion.joints.iter().find(|j| j.joint == hand) {
            timeline.pose_keyframes.push(serde_json::json!({
                "kind":"end_effectors", "pts_ms":contact - 400, "end_ms":contact + 800,
                "visible_from_ms":support, "keypoints":[{
                    "joint":hand, "position":target.position,
                    "rotation_xyzw":target.rotation_xyzw
                }]
            }));
        }
    }

    fn guide_natural_heading(&self, timeline: &mut super::motion_timeline::MotionTimeline) {
        if !self.natural_navigation { return; }
        let interval = self.natural_heading_interval_ms;
        if interval == 0 { timeline.root.face_path = false; return; }
        let (start, duration, origin) = if timeline.root.face_path {
            (self.motion_start().0, 800, self.observed.heading)
        } else if let Some(action) = self.active.iter().find(|a| a.request.verb == Verb::Grab && a.stage == Stage::Execute) {
            let Some(object) = self.objects.iter().find(|o| Some(&o.id) == action.request.target.as_ref()) else { return; };
            timeline.root.heading = Some(action_heading(action, object));
            (action.execute_pts_ms,
                (super::locomotion::reach_duration(action.execute_origin, object.approach_position()) * 1000.) as u64,
                action.execute_heading)
        } else { return; };
        timeline.root.heading_interval_ms = interval;
        timeline.root.heading_transition = Some(super::motion_timeline::HeadingTransition {
            start_ms: start, duration_ms: duration, from: origin,
        });
    }

    /// Condition the generator on a stable stance while waiting for a task.
    /// This is a native pose constraint, never a post-generation pose override.
    /// Body gestures release it entirely; the model continues from history.
    fn constrain_standing_idle(&self, timeline: &mut super::motion_timeline::MotionTimeline) {
        if self.seated.is_some() || !self.timeline_layers.is_empty()
            || self.active.iter().any(|a| a.request.verb != Verb::Smile)
        {
            return;
        }
        let (start, mut root) = self.motion_start();
        root[1] = self.standing_surface.as_ref().map_or(0.95, |s| s.position[1] + super::support::ROOT_HEIGHT);
        timeline.root.keyframes = vec![super::motion_timeline::RootKeyframe {
            pts_ms: start as i64, position: root,
        }];
        timeline.root.heading = Some(self.observed.heading);
        timeline.root.constrain_height = true;
        if self.standing_surface.is_some()
            || self.surface_recovery_at_ms.is_some_and(|pts| start.saturating_sub(pts) <= 1500)
        {
            // Feet alone keep the actor on the cushion but leave the torso and
            // head free to fold over time. Give ARDY an upright full-body
            // destination, placed at the discovered support height; thereafter
            // support is sampled from actual committed feet, as for kneeling.
            timeline.root.position_mode = "keyframes".into();
            timeline.root.constrain_height = false;
            timeline.prompts = vec![super::motion_timeline::PromptSpan {
                start_ms: start, end_ms: None,
                text: if self.standing_surface.is_some() {
                    "A woman stands upright on the support with both feet planted, shoulders relaxed and head comfortably above her shoulders."
                } else {
                    "A woman stands upright on the floor with both feet planted, shoulders relaxed and head comfortably above her shoulders."
                }.into(),
            }];
            timeline.pose_keyframes.push(serde_json::json!({
                "kind":"generated_posture", "posture":"standing",
                "pts_ms":start + 800, "visible_from_ms":start,
                "heading":self.observed.heading
            }));
            timeline.pose_keyframes.push(serde_json::json!({
                "kind":"observed_support", "pts_ms":start + 1200
            }));
            return;
        }
        timeline.pose_keyframes.push(serde_json::json!({
            "kind":"model_pose", "posture":"standing", "hold":true,
            "pts_ms":start + 1000, "root_position":root, "heading":self.observed.heading,
            "carry_hands":self.held.iter().map(|(hand, _)| if *hand == Hand::Left { "Left" } else { "Right" }).collect::<Vec<_>>()
        }));
    }
    /// Acquire support only within 15 cm, then preserve contact until standing.
    /// Subsequent model drift cannot repeatedly detach and drop onto the seat.
    #[cfg(test)]
    pub fn supported_root(&mut self, proposed: Vec3) -> Vec3 {
        if self
            .active
            .iter()
            .any(|a| a.stage.is_standing() && self.observed.displayed_pts_ms >= a.execute_pts_ms)
        {
            self.seat_supported = false;
            return proposed;
        }
        let seat = self.seated.as_deref().or_else(|| {
            self.active
                .iter()
                .find(|a| a.request.verb == Verb::Sit && a.stage == Stage::Settling)
                .and_then(|a| a.request.target.as_deref())
        });
        let anchor = seat
            .and_then(|id| self.objects.iter().find(|o| o.id == id))
            .and_then(|o| o.seat.map(|p| Vec3::from(o.anchor(p))));
        let Some(anchor) = anchor else {
            self.seat_supported = false;
            return proposed;
        };
        self.seat_supported |= proposed.distance(anchor) <= 0.15;
        if !self.seat_supported {
            return proposed;
        }
        let observed = Vec3::from(self.observed.pelvis);
        observed + (anchor - observed).clamp_length_max(0.025)
    }
    /// Accept an unchanged model root or reject the entire candidate frame.
    /// Validation must never manufacture a collision-safe or seat-supported pose.
    pub fn resolve_root(&mut self, proposed: Vec3) -> Option<Vec3> {
        self.permits_native_root(proposed).then_some(proposed)
    }

    pub(crate) fn resolve_native_pose(&mut self, pose: &super::super::renderer::SkeletalPose) -> Option<Vec3> {
        self.native_seat_candidate = None;
        let root = Vec3::from(pose.root_pos);
        let measurement = self.native_seat.as_ref().and_then(|admission|
            admission.measure(pose, self.native_previous_pose.as_ref(), &self.objects));
        match measurement {
            None => self.resolve_root(root),
            Some(Err(error)) => { self.motion_error = Some(error); None }
            Some(Ok(evidence)) => {
                if !self.permits_root_with_validated_seat(root, Some(&evidence.target_id), NATIVE_ROOT_RESERVE_M) {
                    return None;
                }
                self.native_seat_candidate = Some(evidence);
                Some(root)
            }
        }
    }

    /// Remember each object's first observed (authored) pose as its home.
    pub fn remember_homes(&mut self) {
        for (id, pose) in home_poses(&self.objects) {
            self.homes.entry(id).or_insert(pose);
        }
    }

    fn apply_coupling(&mut self, command: super::interaction::CouplingCommand) {
        use super::interaction::CouplingCommand as C;
        match command {
            C::Attach { object, anchors, slide_on } => {
                if self.constrained_objects.contains(&object) {
                    self.motion_error = Some(format!("{object}: articulated parts require a physical contact grip"));
                    return;
                }
                if self.couplings.iter().any(|c| c.object == object
                    || c.anchors.iter().any(|(h, ..)| anchors.iter().any(|(a, ..)| a == h))) {
                    self.motion_error = Some(format!("cannot attach {object}: object or hand already coupled"));
                    return;
                }
                let slide_height = slide_on.and_then(|reference| self.objects.iter().find(|o| o.id == object)
                    .and_then(|owner| super::interaction::resolve_zone(owner, &reference, &self.objects, &self.homes))
                    .map(|(zone, t)| (t * Vec3::from(zone.center.unwrap_or_default())).y));
                self.couplings.push(Coupling { object, anchors, slide_height, release: None, latch: None });
            }
            C::Release { object, onto } => match self.couplings.iter_mut().find(|c| c.object == object) {
                Some(coupling) => coupling.release = Some(onto),
                // A physically gripped object let go onto a zone: settle it there too.
                None if onto.is_some() && self.objects.iter().any(|o| o.id == object) =>
                    self.couplings.push(Coupling { object, anchors: Vec::new(), slide_height: None,
                                                   release: Some(onto), latch: None }),
                None => self.motion_error = Some(format!("cannot release {object}: not coupled")),
            },
            C::SupportOn { object } => self.support_object = Some(object),
            C::SupportOff { object } => {
                if self.support_object.as_deref() == Some(object.as_str()) {
                    self.support_object = None;
                    // Stepping off: leave the support the same way as a seat.
                    self.leaving_seat = Some(object);
                }
            }
        }
    }

    /// Hands currently holding a coupled object, with the grip hint used.
    pub fn coupled_hands(&self) -> Vec<(Hand, Option<super::interaction::Grip>)> {
        self.couplings.iter().filter(|c| c.release.is_none())
            .flat_map(|c| c.anchors.iter().map(|(h, _, g)| (*h, *g))).collect()
    }

    pub(crate) fn record_native_pose(&mut self, pose: &super::super::renderer::SkeletalPose) {
        self.observed.native_seat = self.native_seat_candidate.clone();
        if let Some(evidence) = &self.native_seat_candidate {
            self.leaving_seat = Some(evidence.target_id.clone());
        } else if let Some(id) = self.leaving_seat.clone() {
            let pelvis = Vec2::new(pose.root_pos[0], pose.root_pos[2]);
            let clear = self.navigation_scene().objects.iter().find(|o| o.id == id)
                .is_none_or(|o| o.footprint.distance(pelvis) >= NATIVE_ROOT_RESERVE_M);
            if clear { self.leaving_seat = None; }
        }
        self.native_previous_pose = Some(pose.clone());
    }

    pub fn permits_root(&self, position: Vec3) -> bool {
        self.permits_root_with_validated_seat(position, None, PLANNING_ROOT_RESERVE_M)
    }

    /// Root check for generated native motion (see [`NATIVE_ROOT_RESERVE_M`]).
    pub fn permits_native_root(&self, position: Vec3) -> bool {
        self.permits_root_with_validated_seat(position, None, NATIVE_ROOT_RESERVE_M)
    }

    fn permits_root_with_validated_seat(&self, position: Vec3, validated_seat: Option<&str>, reserve: f32) -> bool {
        if let Some(permitted) = self.permits_surface_root(position) { return permitted; }
        let start = Vec2::new(self.observed.pelvis[0], self.observed.pelvis[2]);
        let end = Vec2::new(position.x, position.z);
        let push = self.active.iter().find_map(|a| a.push.as_ref());
        if let Some(plan) = push {
            let Some(object) = self.objects.iter().find(|o| o.id == plan.id) else { return false; };
            let contact = Vec3::from(object.position) + plan.contact_offset;
            let separation = (contact - position).dot(plan.direction);
            let lateral = (contact - position - plan.direction * separation).length();
            if separation < 0.22 || lateral > 0.65 { return false; }
        }
        let scene = self.navigation_scene();
        let checked: Vec<_> = scene
            .objects
            .iter()
            .filter(|o| validated_seat != Some(o.id.as_str()) && self.seat_transition() != Some(o.id.as_str())
                && push.is_none_or(|plan| plan.id != o.id)
                && !(reserve == NATIVE_ROOT_RESERVE_M && self.support_object.as_deref() == Some(o.id.as_str()))
                && !self.couplings.iter().any(|c| c.object == o.id))
            .collect();
        // Generated motion standing up from the seat it was measured on (a bed it sat
        // on, say) may move away from that seat but never deeper into it. Any other
        // object, and planning, keep the strict visibility rule.
        let leaving = |o: &&super::metadata::SceneObject| reserve == NATIVE_ROOT_RESERVE_M
            && self.leaving_seat.as_deref() == Some(o.id.as_str()) && o.footprint.distance(start) < reserve;
        let polygons: Vec<_> = checked
            .iter()
            .filter(|o| !leaving(o))
            .map(|o| (o.id.clone(), o.footprint.inflated_polygon(reserve)))
            .collect();
        checked.iter().filter(|o| leaving(o))
            .all(|o| o.footprint.distance(end) >= o.footprint.distance(start) - 1e-4)
            && super::pathfinder::visible(start, end, &polygons)
    }
    pub fn navigation_scene(&self) -> super::metadata::SceneMetadata {
        scene_metadata(
            &self
                .objects
                .iter()
                .filter(|o| self.constrained_objects.contains(&o.id)
                    || (!self.held.iter().any(|(_, id)| id == &o.id)
                    && !self.couplings.iter().any(|c| c.object == o.id)
                    && !self.active.iter().any(|a| a.push.as_ref().is_some_and(|p| p.id == o.id))))
                .cloned()
                .collect::<Vec<_>>(),
        )
    }
    pub fn seat_transition(&self) -> Option<&str> {
        self.active
            .iter()
            .find_map(|a| {
                if a.stage.is_standing() {
                    self.seated.as_deref()
                } else if a.request.verb == Verb::Sit
                    && matches!(a.stage, Stage::Align | Stage::Execute | Stage::Settling)
                {
                    a.request.target.as_deref()
                } else {
                    None
                }
            })
            .or(self.seated.as_deref())
    }
    pub fn seated_idle(&self) -> bool {
        let before_handoff = self
            .motion
            .handoff_pts_ms
            .is_some_and(|at| self.observed.displayed_pts_ms < at);
        self.seated.is_some()
            && self.seat_supported
            && (self.timeline_layers.is_empty() || before_handoff)
            && self.active.iter().all(|a| {
                a.request.verb == Verb::Smile || self.observed.displayed_pts_ms < a.execute_pts_ms
            })
    }
    pub fn smile(&self) -> f32 {
        self.active
            .iter()
            .filter(|a| a.request.verb == Verb::Smile)
            .map(|a| {
                let t = (self.now - a.started) as f32;
                a.request.intensity
                    * (t / 0.25).min(1.)
                    * ((a.request.duration_sec - t) / 0.25).clamp(0., 1.)
            })
            .fold(0., f32::max)
    }
    pub fn snapshot(&self) -> OfficeSnapshot {
        OfficeSnapshot {
            native_seat_profile: self.native_seat.as_ref().map(|g| g.profile_identity().to_owned()),
            navigation_object_ids: self.navigation_scene().objects.into_iter().map(|o| o.id).collect(),
            motion_error: self.motion_error.clone(),
            motion_rejection: self.motion_rejection.clone(),
            timeline_edit_ids: self.timeline_layers.keys().cloned().collect(),
            earliest_edit_pts_ms: self.motion_start().0,
            occupied_hands: self.held.clone(),
            objects: self.objects.clone(),
            world_keypoints: self
                .objects
                .iter()
                .map(|o| (o.id.clone(), o.resolved_world_keypoints()))
                .collect(),
            avatar: self.observed.clone(),
            posture: if self.seated.is_some() {
                "seated"
            } else if self.standing_surface.is_some() {
                "standing_on_surface"
            } else {
                "standing"
            }
            .into(),
            standing_surface: self.standing_surface.clone(),
            events: self.events.iter().cloned().collect(),
            active: self.active.iter().map(|a| a.request.clone()).collect(),
            queued: self.pending.iter().cloned().collect(),
            motion: self.motion.clone(),
            couplings: self.couplings.clone(),
            grasps: self.grasps.clone(),
            support_object: self.support_object.clone(),
            homes: self.homes.clone(),
        }
    }
}

fn wave_hand_restored(wave: &Active, observed: &Observation, seated: bool) -> bool {
    let current =
        Vec3::from(observed.wrists[hand_index(wave.request.hand)]) - Vec3::from(observed.pelvis);
    let resting = Vec3::from(wave.origin_wrist) - Vec3::from(wave.execute_origin);
    if seated {
        let other = Quat::from_rotation_y(-wave.execute_heading)
            * (Vec3::from(observed.wrists[1 - hand_index(wave.request.hand)]) - Vec3::from(observed.pelvis));
        resting.y < 0.4 && current.distance(resting) < 0.13
            && other.y < 0.45 && other.z > 0.02
    } else {
        current.y < 0.1
    }
}

fn seat_contacts_aligned(observed: &Observation, seat: &OfficeObject) -> bool {
    let forward = Quat::from_rotation_y(seat.yaw_deg.to_radians()) * Vec3::Z;
    let origin = Vec3::from(seat.position);
    observed.torso_up[1] > 0.94
        && observed.feet.iter().all(|foot| {
            let p = Vec3::from(*foot);
            // Ankle centers must be on the floor in front of the seat, never behind
            // its backrest. Height accounts for the avatar's ankle-to-sole distance.
            p.y >= -0.02 && p.y <= 0.18 && (p - origin).dot(forward) >= 0.25
        })
}

/// Reserve time to turn before translating. Heading is generated by ARDY from
/// the same route; this never rotates a rendered pose or rewrites its history.
pub(super) fn walking_waypoints(path: &[pathfinder::Waypoint], mut time: f32, heading: f32) -> Vec<[f32; 4]> {
    let Some(first) = path.first() else {
        return Vec::new();
    };
    let mut roots = vec![[first.xz.x, 0.95, first.xz.y, time]];
    if let Some(next) = path.iter().find(|p| p.xz.distance(first.xz) > 0.001) {
        let direction = next.xz - first.xz;
        let delta = direction.x.atan2(direction.y) - heading;
        let turn = delta.sin().atan2(delta.cos()).abs();
        if turn > 15_f32.to_radians() {
            time += turn / 120_f32.to_radians();
            roots.push([first.xz.x, 0.95, first.xz.y, time]);
        }
    }
    for pair in path.windows(2) {
        let distance = pair[0].xz.distance(pair[1].xz);
        if distance <= 0.001 {
            continue;
        }
        time += distance / 0.8;
        roots.push([pair[1].xz.x, 0.95, pair[1].xz.y, time]);
    }
    roots
}

#[cfg(test)]
#[path = "constrained_navigation_tests.rs"]
mod constrained_navigation_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn action(id: &str, verb: Verb, hand: Option<Hand>, target: Option<&str>) -> ActionRequest {
        ActionRequest {
            action_id: id.into(),
            verb,
            hand,
            target: target.map(str::to_string),
            position: None,
            duration_sec: 1.,
            intensity: 0.7,
        }
    }
    fn ready() -> OfficeController {
        let mut c = OfficeController::default();
        // Retain legacy choreography tests for the explicit A/B mode.
        // Natural navigation has separate trajectory and handoff regressions.
        c.natural_navigation = false;
        c.observed.ready = true;
        c.observed.pelvis = [0., 0.97, 0.2];
        c
    }
    fn submit(c: &mut OfficeController, r: ActionRequest, w: &SceneState) {
        c.command(OfficeCommand::PerformAction { request: r }, w);
    }

    #[test]
    fn push_waits_for_hand_contact_and_observed_furniture_follow_through() {
        use super::super::super::renderer::PropSnapshot;
        let mut c = ready();
        c.objects.retain(|o| o.id == "chair");
        let chair = &mut c.objects[0];
        chair.position[2] = 0.8;
        chair.dynamic = true;
        chair.pushable = true;
        c.observed.heading = 0.;
        c.observed.pelvis = [0., 0.95, 0.];
        let mut r = action("push", Verb::Push, None, Some("chair"));
        r.position = Some([0., 0.23, 1.02]);
        r.duration_sec = 3.;
        let mut world = SceneState::default();
        submit(&mut c, r, &world);
        assert_eq!(c.active[0].stage, Stage::Turn);
        assert!(c.push_force_intent().is_none());
        c.active[0].stage = Stage::Contact;
        c.active[0].execute_pts_ms = 1000;
        c.observed.displayed_pts_ms = 2900;
        let contact = Vec3::from(c.objects[0].position) + c.active[0].push.as_ref().unwrap().contact_offset;
        c.observed.wrists = [(contact + Vec3::X * 0.18).to_array(),
                             (contact - Vec3::X * 0.18).to_array()];
        c.observed.torso_up = [0., 1., 0.];
        c.advance(&world);
        assert!(c.push_force_intent().is_none(), "one contact sample cannot start force");
        c.observed.displayed_pts_ms = 3200;
        c.advance(&world);
        assert_eq!(c.active[0].stage, Stage::Execute);
        assert!(c.push_force_intent().is_some());
        world.props.push(PropSnapshot { id: "chair".into(), transform: Mat4::from_translation(Vec3::new(0., 0.23, 0.8)).to_cols_array_2d(),
            aabb_min: [0.; 3], aabb_max: [0.; 3], grasp_world: None, articulation: None, loading: false });
        c.observed.displayed_pts_ms = 4800;
        c.advance(&world);
        assert_eq!(c.active[0].stage, Stage::Execute, "animated feet alone cannot move furniture");
        world.props[0].transform[3][2] = 0.99;
        for pts in (4850..=5300).step_by(50) {
            c.observed.displayed_pts_ms = pts;
            c.advance(&world);
        }
        assert_eq!(c.active[0].stage, Stage::Execute, "the avatar cannot be left behind");
        c.observed.pelvis[2] += 0.18;
        c.observed.wrists = c.observed.wrists.map(|w| [w[0], w[1], w[2] + 0.19]);
        for pts in (5350..=5800).step_by(50) {
            c.observed.displayed_pts_ms = pts;
            c.advance(&world);
            if c.active[0].stage == Stage::Returning { break; }
        }
        assert_eq!(c.active[0].stage, Stage::Returning);
        assert!(!c.events.iter().any(|e| e.status == Status::Completed), "recovery precedes completion");
        for pts in (7500..=8100).step_by(50) {
            c.observed.displayed_pts_ms = pts;
            c.advance(&world);
        }
        assert!(c.events.iter().any(|e| e.status == Status::Completed));
    }

    #[test]
    fn placement_requires_a_grip_and_observed_stable_support() {
        use super::super::super::renderer::{GrabSnapshot, PropSnapshot};
        let mut c = ready();
        let desk = c.objects.iter().find(|o| o.id == "desk").unwrap().clone();
        let anchor = desk.placement_position().unwrap();
        let request = action("place", Verb::Release, Some(Hand::Right), Some("desk"));
        let mut world = SceneState::default();
        submit(&mut c, request.clone(), &world);
        assert!(c.events.iter().any(|e| e.status == Status::Failed && e.detail == "hand is not holding a prop"));

        let mut c = ready();
        c.observed.wrists[1] = anchor;
        c.observed.displayed_pts_ms = 1000;
        world.grabs.push(GrabSnapshot { hand: Hand::Right, prop: "mug".into() });
        submit(&mut c, request, &world);
        c.active[0].stage = Stage::Execute;
        c.active[0].execute_pts_ms = 0;
        c.advance(&world);
        assert!(c.grab_commands.iter().any(|cmd| matches!(cmd, GrabOrRelease::Release(Hand::Right))));
        assert!(!c.events.iter().any(|e| e.status == Status::Completed));
        world.grabs.clear();
        world.props.push(PropSnapshot {
            id: "mug".into(), transform: [[0.; 4]; 4],
            aabb_min: [anchor[0] - 0.04, 0.70, anchor[2] - 0.04],
            aabb_max: [anchor[0] + 0.04, 0.78, anchor[2] + 0.04],
            grasp_world: None, articulation: None, loading: false,
        });
        for pts in (1050..=1500).step_by(50) {
            // Face pose PTS can remain zero throughout body-motion playback.
            world.pts_ms = 0;
            c.observed.displayed_pts_ms = pts;
            c.advance(&world);
        }
        assert!(!c.events.iter().any(|e| e.status == Status::Completed), "unsupported height cannot complete");
        world.props[0].aabb_min[1] = 0.76;
        world.props[0].aabb_max[1] = 0.84;
        for pts in (1550..=2000).step_by(50) {
            world.pts_ms = 0;
            c.observed.displayed_pts_ms = pts;
            c.advance(&world);
        }
        assert!(c.events.iter().any(|e| e.status == Status::Completed));
    }

    #[test]
    fn placement_outside_surface_fails_after_release_timeout() {
        use super::super::super::renderer::GrabSnapshot;
        let mut c = ready();
        let anchor = c.objects.iter().find(|o| o.id == "desk").unwrap().placement_position().unwrap();
        c.observed.wrists[1] = anchor;
        c.observed.displayed_pts_ms = 1000;
        let mut world = SceneState::default();
        world.grabs.push(GrabSnapshot { hand: Hand::Right, prop: "mug".into() });
        submit(&mut c, action("place", Verb::Release, Some(Hand::Right), Some("desk")), &world);
        c.active[0].stage = Stage::Execute;
        c.active[0].execute_pts_ms = 0;
        c.advance(&world);
        world.grabs.clear();
        c.observed.displayed_pts_ms = 6101;
        world.pts_ms = 6101;
        c.advance(&world);
        assert!(c.events.iter().any(|e| e.status == Status::Failed && e.detail.contains("did not settle")));
        assert!(!c.events.iter().any(|e| e.status == Status::Completed));
    }
    #[test]
    fn walking_faces_the_route_and_preserves_that_policy_during_a_gesture() {
        let mut c = ready();
        let mut w = SceneState::default();
        let mut request = action("walk", Verb::Walk, None, None);
        request.position = Some([1.5, 0., 0.2]);
        submit(&mut c, request, &w);
        let root = &c.motion.timeline.as_ref().unwrap().root;
        assert!(root.face_path);
        assert!(root.heading.is_none());
        // +X requires a quarter-turn from the observed +Z heading. The root
        // stays still during that turn instead of demanding an immediate strafe.
        assert_eq!(root.keyframes[0].position, root.keyframes[1].position);
        assert_eq!(root.keyframes[1].pts_ms - root.keyframes[0].pts_ms, 750);
        let turn = root.heading_transition.as_ref().unwrap();
        assert_eq!(turn.from, c.observed.heading);
        assert_eq!(turn.duration_ms, 750);
        let goal = &c.motion.timeline.as_ref().unwrap().pose_keyframes[0];
        assert_eq!(goal["posture"], "standing");
        assert_eq!(goal["pts_ms"], root.keyframes.last().unwrap().pts_ms);
        assert_eq!(goal["root_position"], serde_json::json!(root.keyframes.last().unwrap().position));
        let keys = &root.keyframes;
        let end = keys.last().unwrap().position;
        let previous = keys[keys.len() - 2].position;
        let arrival_heading = (end[0] - previous[0]).atan2(end[2] - previous[2]);
        assert!((goal["heading"].as_f64().unwrap() - arrival_heading as f64).abs() < 1e-6);
        assert_eq!(goal["hold"], true);
        assert_ne!(goal["enforce"], true);
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Left), None),
            &w,
        );
        assert!(c.motion.timeline.as_ref().unwrap().root.face_path);
        w.grabs.push(super::super::super::renderer::GrabSnapshot { hand: Hand::Right, prop: "mug".into() });
        c.advance(&w);
        c.rebuild(&w);
        let goal = &c.motion.timeline.as_ref().unwrap().pose_keyframes[0];
        assert_eq!(goal["free_hands"], serde_json::json!(["Left"]));
        assert_eq!(goal["carry_hands"], serde_json::json!(["Right"]));
        c.command(
            OfficeCommand::CancelAction {
                action_id: "walk".into(),
            },
            &w,
        );
        assert!(!c.motion.timeline.as_ref().unwrap().root.face_path);
    }

    #[test]
    fn route_timing_preserves_handoff_and_uses_shortest_initial_turn() {
        let path =
            [Vec2::ZERO, Vec2::ZERO, Vec2::new(0.1, -1.)].map(|xz| pathfinder::Waypoint { xz });
        let roots = walking_waypoints(&path, 0.5, -175_f32.to_radians());
        assert_eq!(
            roots.len(),
            2,
            "a small turn across pi needs no turning pause"
        );
        assert_eq!(roots[0][3], 0.5);
        assert!((roots[1][3] - 0.5 - path[2].xz.length() / 0.8).abs() < 1e-5);
        let roots = walking_waypoints(&path, 0.5, 0.);
        assert_eq!(roots.len(), 3);
        assert_eq!(roots[0][..3], roots[1][..3]);
        assert!(roots[1][3] > 1.9);
        assert!(roots.windows(2).all(|p| p[0][3] < p[1][3]));
    }

    #[test]
    fn standing_idle_is_generated_with_a_native_target_and_yields_to_actions() {
        let mut c = ready();
        let w = SceneState::default();
        c.rebuild(&w);
        let idle = &c.motion.timeline.as_ref().unwrap().pose_keyframes[0];
        assert_eq!(idle["posture"], "standing");
        assert_eq!(idle["hold"], true);
        assert!(idle.get("enforce").is_none());
        submit(&mut c, action("wave", Verb::Wave, Some(Hand::Left), None), &w);
        assert!(c.motion.timeline.as_ref().unwrap().pose_keyframes.is_empty(),
            "standing wave must use current motion history without pose or wrist targets");
        let mut walk = action("walk", Verb::Walk, None, None);
        walk.position = Some([1.5, 0., 0.2]);
        submit(&mut c, walk, &w);
        assert!(c.motion.timeline.as_ref().unwrap().pose_keyframes.iter()
            .any(|key| key["posture"] == "standing" && key["hold"] == true),
            "a generated standing destination stabilizes the route endpoint");
    }

    #[test]
    fn seated_idle_contacts_remain_committed_until_handoff() {
        let mut c = ready();
        let w = SceneState::default();
        c.seated = Some("chair".into());
        c.seat_supported = true;
        c.observed.pelvis = [0., 0.6, -0.8];
        c.rebuild(&w);
        let supported = |m: &MotionRequest| {
            m.timeline
                .as_ref()
                .unwrap()
                .pose_keyframes
                .iter()
                .any(|k| k["kind"] == "model_pose" && k["posture"] == "seated" && k["hold"] == true)
        };
        assert!(supported(&c.motion));
        let old_prompt = c.motion.prompt.clone();
        c.observed.displayed_pts_ms = 500;
        c.future_handoffs = true;
        c.future_root = Some((1000, [0., 0.6, -0.8]));
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Right), None),
            &w,
        );
        assert!(c.seated_idle());
        assert_eq!(c.display_motion().prompt, old_prompt);
        assert!(supported(c.display_motion()));
        assert!(!supported(&c.motion), "wave must release the full-body hold");
        let keys = &c.motion.timeline.as_ref().unwrap().pose_keyframes;
        assert!(keys.iter().all(|key| key["kind"] == "end_effectors"));
        let contacts: Vec<_> = keys.iter().flat_map(|key| key["keypoints"].as_array().unwrap())
            .map(|point| point["joint"].as_str().unwrap()).collect();
        assert_eq!(contacts, ["LeftFoot", "RightFoot", "Hips"],
            "retain seat and floor contacts, without authored wave wrist targets");
        assert!(c.motion.prompt.contains("waves"));
        c.observed.displayed_pts_ms = 1000;
        assert!(!c.seated_idle());
        assert!(c.display_motion().prompt.contains("waves"));
    }
    #[test]
    fn carrying_mug_keeps_seated_pelvis_support() {
        let mut c = ready();
        c.seated = Some("chair".into());
        c.held = vec![(Hand::Right, "mug".into())];
        c.rebuild(&SceneState::default());
        let key = c
            .motion
            .timeline
            .as_ref()
            .unwrap()
            .pose_keyframes
            .iter()
            .find(|k| k["kind"] == "model_pose" && k["posture"] == "seated")
            .expect("carrying must not disable seat support");
        assert_eq!(key["carry_hands"], serde_json::json!(["Right"]));
        assert_eq!(key["heading"], 0.0);
    }
    #[test]
    fn disabled_future_handoff_uses_displayed_pose_and_clock() {
        let mut c = ready();
        c.future_handoffs = false;
        c.observed.displayed_pts_ms = 500;
        c.observed.pelvis = [0., 0.95, 0.];
        c.future_root = Some((1000, [0.4, 0.7, 0.3]));
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Left), None),
            &SceneState::default(),
        );
        assert_eq!(c.active[0].execute_pts_ms, 500);
        assert_eq!(c.active[0].execute_origin, [0., 0.95, 0.]);
        assert!(c.motion.handoff_pts_ms.is_none());
        assert!(c.committed_motion.is_none());
    }
    #[test]
    fn new_action_starts_from_buffered_future_root_and_clock() {
        let mut c = ready();
        c.future_handoffs = true;
        c.observed.displayed_pts_ms = 500;
        c.future_root = Some((1000, [0.2, 0.97, 0.2]));
        let w = SceneState::default();
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Left), None),
            &w,
        );
        assert_eq!(c.active[0].execute_pts_ms, 1000);
        assert_eq!(c.active[0].execute_origin, [0.2, 0.97, 0.2]);
        assert_eq!(c.motion.handoff_pts_ms, Some(1000));
        c.now = 4.; // Wall-clock delays must not expire unplayed motion.
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Execute);
        c.observed.displayed_pts_ms = 2000;
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Returning);
    }
    #[test]
    fn seated_wave_requires_restored_body_and_settled_return() {
        let mut c = ready();
        c.seated = Some("chair".into());
        c.observed.pelvis = [0., 0.6, -0.8];
        c.observed.wrists = [[0.3, 0.73, -0.62], [-0.3, 0.78, -0.7]];
        c.observed.feet = [[0.12, 0.08, -0.35], [-0.12, 0.08, -0.35]];
        c.observed.torso_up = [0., 1., 0.];
        let w = SceneState::default();
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Right), None),
            &w,
        );
        c.observed.wrists[1][1] = 1.2;
        c.now = 0.5;
        c.observed.displayed_pts_ms = 500;
        c.advance(&w);
        c.now = 2.;
        c.observed.displayed_pts_ms = 2000;
        c.advance(&w);
        assert_eq!(c.active.len(), 1);
        c.observed.wrists[1] = [-0.3, 0.78, -0.7];
        c.advance(&w);
        assert_eq!(c.active.len(), 1, "a low wrist cannot bypass the return blend");
        c.observed.displayed_pts_ms = 2800;
        c.observed.torso_up = [0., 0.5, 0.86];
        c.advance(&w);
        assert!(c.active[0].stable_since_ms.is_none(), "twisted body is not settled");
        c.observed.torso_up = [0., 1., 0.];
        c.observed.wrists[0][2] = -1.1;
        c.advance(&w);
        assert!(c.active[0].stable_since_ms.is_none(), "other hand must clear the chair");
        c.observed.wrists[0][2] = -0.62;
        c.advance(&w);
        c.observed.displayed_pts_ms = 3100;
        c.advance(&w);
        assert_eq!(c.active.len(), 1, "return needs a sustained settled interval");
        c.observed.displayed_pts_ms = 3200;
        c.advance(&w);
        assert!(c.active.is_empty());
        assert_eq!(c.events.back().unwrap().status, Status::Completed);
    }
    #[test]
    fn seated_completion_rejects_feet_behind_chair_and_tipped_torso() {
        let c = ready();
        let chair = c.objects.iter().find(|o| o.id == "chair").unwrap();
        let mut observed = c.observed;
        observed.torso_up = [0., 1., 0.];
        observed.feet = [[0.1, 0.08, -1.3], [-0.1, 0.08, -1.3]];
        assert!(!seat_contacts_aligned(&observed, chair));
        observed.feet = [[0.1, 0.08, -0.35], [-0.1, 0.08, -0.35]];
        assert!(seat_contacts_aligned(&observed, chair));
        observed.torso_up = [0., 0.8, 0.6];
        assert!(!seat_contacts_aligned(&observed, chair));
    }
    #[test]
    fn live_timeline_edits_preserve_world_and_reject_stale_writers() {
        use super::super::motion_timeline::{PromptSpan, TimelineLayer};
        let mut c = ready();
        let w = SceneState::default();
        c.seated = Some("chair".into());
        c.seat_supported = true;
        assert!(c.seated_idle());
        c.held.push((Hand::Left, "mug".into()));
        c.observed.displayed_pts_ms = 3000;
        let position = c.observed.pelvis;
        let revision = c.revision;
        let layer = TimelineLayer {
            prompts: Some(vec![PromptSpan {
                start_ms: 3000,
                end_ms: None,
                text: "waves the free right hand while seated".into(),
            }]),
            ..Default::default()
        };
        c.command(
            OfficeCommand::EditMotionTimeline {
                edit_id: "live".into(),
                expected_revision: revision,
                layer: Some(layer.clone()),
            },
            &w,
        );
        assert_eq!(c.revision, revision + 1);
        assert_eq!(c.observed.pelvis, position);
        assert_eq!(c.observed.displayed_pts_ms, 3000);
        assert_eq!(c.seated.as_deref(), Some("chair"));
        assert_eq!(c.held, vec![(Hand::Left, "mug".into())]);
        assert_eq!(c.snapshot().timeline_edit_ids, vec!["live"]);
        assert!(!c.seated_idle());
        assert!(c.motion.timeline.as_ref().unwrap().prompts[0]
            .text
            .contains("right hand"));
        c.command(
            OfficeCommand::EditMotionTimeline {
                edit_id: "live".into(),
                expected_revision: revision,
                layer: None,
            },
            &w,
        );
        assert_eq!(c.revision, revision + 1);
        assert_eq!(c.events.back().unwrap().status, Status::Failed);
        c.command(
            OfficeCommand::EditMotionTimeline {
                edit_id: "other".into(),
                expected_revision: c.revision,
                layer: Some(layer),
            },
            &w,
        );
        assert_eq!(c.events.back().unwrap().status, Status::Failed);
        c.command(
            OfficeCommand::EditMotionTimeline {
                edit_id: "live".into(),
                expected_revision: c.revision,
                layer: None,
            },
            &w,
        );
        assert_eq!(c.revision, revision + 2);
        assert!(c.timeline_layers.is_empty());
        assert!(c.seated_idle());
        assert_eq!(c.seated.as_deref(), Some("chair"));
        assert_eq!(c.held.len(), 1);
    }
    #[test]
    fn live_model_led_kneel_edit_compiles_without_authored_body_pose() {
        use super::super::motion_timeline::TimelineLayer;
        let mut c = ready();
        c.observed.displayed_pts_ms = 1000;
        let layer: TimelineLayer = serde_json::from_value(serde_json::json!({
            "root": {"keyframes": [{"pts_ms": 1050, "position": [0., 0.95, 0.]},
                                   {"pts_ms": 2600, "position": [0., 0.35, 0.]}],
                     "interpolation":"smooth", "position_mode":"keyframes",
                     "constrain_height":false, "heading":null},
            "prompts": [{"start_ms":1000, "end_ms":2200,
                         "text":"A woman kneels down onto both knees on the floor."},
                        {"start_ms":2200, "end_ms":null,
                         "text":"A woman is kneeling on the floor."}],
            "pose_keyframes": [
                {"kind":"generated_posture", "posture":"kneeling", "pts_ms":2200,
                 "visible_from_ms":1000, "heading":0.0},
                {"kind":"observed_support", "pts_ms":2700}
            ]
        })).unwrap();
        c.command(OfficeCommand::EditMotionTimeline {
            edit_id: "low-posture".into(), expected_revision: c.revision,
            layer: Some(layer),
        }, &SceneState::default());
        assert_eq!(c.events.back().unwrap().status, Status::Accepted);
        let timeline = c.motion.timeline.as_ref().unwrap();
        assert!(timeline.pose_keyframes.iter().any(|k| k["kind"] == "generated_posture"));
        assert!(timeline.pose_keyframes.iter().any(|k| k["kind"] == "observed_support"));
        assert!(!timeline.pose_keyframes.iter().any(|k| k["kind"] == "full_body"));
    }

    #[test]
    fn live_root_edit_starts_at_displayed_position_and_invalid_pose_is_atomic() {
        use super::super::motion_timeline::TimelineLayer;
        let mut c = ready();
        let w = SceneState::default();
        c.observed.displayed_pts_ms = 1250;
        let layer: TimelineLayer = serde_json::from_value(serde_json::json!({
            "root": {"keyframes": [{"pts_ms": 3000, "position": [1., 0.97, 0.2]}],
                "interpolation": "linear", "constrain_height": false}
        }))
        .unwrap();
        c.command(
            OfficeCommand::EditMotionTimeline {
                edit_id: "route".into(),
                expected_revision: c.revision,
                layer: Some(layer),
            },
            &w,
        );
        let track = &c.motion.timeline.as_ref().unwrap().root;
        assert_eq!(track.keyframes[0].pts_ms, 1250);
        assert_eq!(track.keyframes[0].position, c.observed.pelvis);
        let revision = c.revision;
        let invalid = TimelineLayer {
            pose_keyframes: vec![serde_json::json!({
                "kind": "full_body", "pts_ms": 4000, "positions": []
            })],
            ..Default::default()
        };
        c.command(
            OfficeCommand::EditMotionTimeline {
                edit_id: "bad".into(),
                expected_revision: revision,
                layer: Some(invalid),
            },
            &w,
        );
        assert_eq!(c.revision, revision);
        assert_eq!(c.events.back().unwrap().status, Status::Failed);
    }
    #[test]
    fn kitchen_plate_grab_uses_live_rim_pose_without_attaching_early() {
        let mut c = ready();
        let world = SceneState::default();
        let plate = c.objects.iter().find(|o| o.id == "plate").unwrap().clone();
        submit(&mut c, action("fetch-plate", Verb::Grab, Some(Hand::Right), Some("plate")), &world);
        assert_eq!(c.active[0].stage, Stage::Approach);
        assert!(c.motion.joints.is_empty(), "do not reach while walking across the home");
        c.observed.pelvis = plate.approach_position();
        c.observed.pelvis[1] = 0.95;
        c.active[0].stage = Stage::Execute;
        c.active[0].execute_origin = c.observed.pelvis;
        c.active[0].execute_pts_ms = 1000;
        c.rebuild(&world);
        let target = c.motion.joints.iter().find(|p| p.joint == "RightHand").unwrap();
        let grasp = plate.grasp_transform().unwrap();
        assert_eq!(target.position, grasp.translation.to_array());
        assert!(Quat::from_array(target.rotation_xyzw).angle_between(grasp.rotation).abs() < 1e-5);
        assert!(c.grab_commands.is_empty());
        assert!(!c.events.iter().any(|e| e.status == Status::Completed));
    }

    #[test]
    fn natural_grab_does_not_shortcut_a_sideways_final_approach() {
        let mut c = ready();
        c.natural_navigation = true;
        let plate = c.objects.iter().find(|o| o.id == "plate").unwrap();
        let goal = plate.approach_position();
        let facing = super::super::locomotion::target_direction(plate);
        let side = Vec2::new(-facing.y, facing.x);
        let request = action("plate", Verb::Grab, Some(Hand::Right), Some("plate"));
        c.observed.pelvis = [goal[0] - side.x * 0.4, 0.95, goal[2] - side.y * 0.4];
        assert!(c.approach_stage(&request, goal).is_none());
        c.observed.pelvis = [goal[0] - facing.x * 0.4, 0.95, goal[2] - facing.y * 0.4];
        assert_eq!(c.approach_stage(&request, goal), Some(Stage::Execute));
    }

    #[test]
    fn natural_grab_blends_final_approach_without_a_pose_or_facing_gate() {
        let mut c = ready();
        c.natural_navigation = true;
        let w = SceneState::default();
        submit(&mut c, action("plate", Verb::Grab, Some(Hand::Right), Some("plate")), &w);
        let walk = c.motion.timeline.as_ref().unwrap();
        assert!(walk.root.face_path);
        assert_eq!(walk.root.heading_interval_ms, 800);
        assert!(walk.root.heading.is_none());
        assert!(walk.pose_keyframes.is_empty());
        assert!(walk.root.keyframes.windows(2).all(|pair| pair[0].position != pair[1].position));
        let goal = c.objects.iter().find(|o| o.id == "plate").unwrap().approach_position();
        c.observed.pelvis = [goal[0], 0.95, goal[2] + 0.5];
        c.observed.heading = 0.8; // Not aligned to the counter.
        c.observed.displayed_pts_ms = 10000;
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Execute);
        let reach = c.motion.timeline.as_ref().unwrap();
        assert_eq!(reach.root.heading_interval_ms, 800);
        assert!(reach.root.heading_transition.is_some());
        assert!(!reach.root.constrain_height);
        assert!(reach.pose_keyframes.iter().all(|k| k["kind"] == "end_effectors"));
        assert!(reach.root.keyframes[1].position[2] < reach.root.keyframes[0].position[2]);
        assert!(c.motion.joints[0].contact_pts_ms > c.observed.displayed_pts_ms + 800);
        c.observed.wrists[1] = c.motion.joints[0].position;
        c.advance(&w);
        assert!(c.grab_commands.is_empty(), "a nearby wrist cannot attach before the planned contact");
    }

    #[test]
    fn floor_grab_schedules_model_kneel_and_history_supported_reach() {
        let mut c = ready();
        let w = SceneState::default();
        submit(&mut c, action("floor", Verb::Grab, Some(Hand::Right), Some("mug")), &w);
        let floor = c.objects.iter_mut().find(|o| o.id == "mug").unwrap();
        floor.position = [0., 0.08, 0.28];
        floor.approach = [0., -0.08, -0.28];
        floor.keypoints.retain(|_, point| point.role != "approach");
        let floor = floor.clone();
        c.active[0].stage = Stage::Execute;
        c.active[0].execute_pts_ms = 1200;
        c.active[0].execute_origin = [0., 0.95, 0.];
        c.active[0].execute_heading = 0.;
        c.observed.displayed_pts_ms = 1200;
        c.rebuild(&w);
        let timeline = c.motion.timeline.as_ref().unwrap();
        assert_eq!(timeline.root.position_mode, "keyframes");
        assert!(!timeline.root.constrain_height);
        assert!(timeline.pose_keyframes.iter().any(|k|
            k["kind"] == "generated_posture" && k["pts_ms"] == 3200));
        assert!(timeline.pose_keyframes.iter().any(|k|
            k["kind"] == "observed_support" && k["pts_ms"] == 3600));
        let reach = timeline.pose_keyframes.iter().find(|k|
            k["kind"] == "end_effectors" && k["keypoints"][0]["joint"] == "RightHand").unwrap();
        assert_eq!(reach["pts_ms"], 4800);
        assert_eq!(reach["end_ms"], 6000);
        assert_eq!(reach["keypoints"][0]["position"], serde_json::json!(
            floor.grasp_transform().unwrap().translation.to_array()));
        assert_eq!(c.motion.joints[0].contact_pts_ms, 5200);
        assert!(timeline.contacts.is_empty());
    }

    #[test]
    fn grab_pursuit_waits_for_displayed_contact_even_when_hand_is_nearby() {
        let mut c = ready();
        let w = SceneState::default();
        submit(
            &mut c,
            action("grab", Verb::Grab, Some(Hand::Right), Some("mug")),
            &w,
        );
        c.active[0].stage = Stage::Execute;
        c.active[0].execute_pts_ms = 1600;
        c.observed.wrists[1] = c
            .objects
            .iter()
            .find(|o| o.id == "mug")
            .unwrap()
            .grasp_transform()
            .unwrap()
            .translation
            .to_array();
        for pts in [1000, 1600, 2399] {
            c.observed.displayed_pts_ms = pts;
            c.advance(&w);
            assert!(
                c.grab_commands.is_empty(),
                "physical timeout started before contact at {pts}"
            );
        }
        c.observed.displayed_pts_ms = 2400;
        c.observed.wrists[1][0] += 0.06;
        c.advance(&w);
        assert!(c.grab_commands.is_empty(), "do not attach while contact correction is still approaching the handle");
        c.observed.wrists[1][0] -= 0.06;
        c.observed.displayed_pts_ms = 2467;
        c.advance(&w);
        assert!(matches!(
            c.grab_commands.as_slice(),
            [GrabOrRelease::Grab(_)]
        ));
    }
    #[test]
    fn contact_deadline_survives_concurrent_action_revision() {
        let mut c = ready();
        let w = SceneState::default();
        let mug = c.objects.iter().find(|o| o.id == "mug").unwrap();
        c.observed.pelvis = mug.anchor(mug.approach);
        c.observed.pelvis[1] = 0.95;
        c.observed.displayed_pts_ms = 1000;
        submit(
            &mut c,
            action("grab", Verb::Grab, Some(Hand::Right), Some("mug")),
            &w,
        );
        assert_eq!(c.active[0].stage, Stage::Align);
        assert!(
            c.motion.joints.is_empty(),
            "do not reach until facing the object"
        );
        c.observed.heading = -std::f32::consts::FRAC_PI_2;
        c.observed.pelvis[1] = 0.95;
        c.observed.torso_up = [0., 1., 0.];
        c.advance(&w);
        assert_eq!(
            c.active[0].stage,
            Stage::Align,
            "instantaneous facing is not settled alignment"
        );
        for pts in [1800, 2200] {
            c.observed.displayed_pts_ms = pts;
            c.advance(&w);
        }
        let deadline = c.motion.joints[0].contact_pts_ms;
        assert_eq!(deadline, 3000);
        let timeline = c.motion.timeline.as_ref().unwrap();
        assert!(
            timeline.contacts.is_empty(),
            "native hand keys replace the legacy mask"
        );
        assert!(timeline
            .pose_keyframes
            .iter()
            .any(|key| key["keypoints"][0]["joint"] == "RightHand"));
        c.observed.displayed_pts_ms = 2400;
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Left), None),
            &w,
        );
        assert_eq!(c.motion.joints[0].contact_pts_ms, deadline);
    }
    #[test]
    fn wave_waits_for_observed_arm_return() {
        let mut c = ready();
        let w = SceneState::default();
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Left), None),
            &w,
        );
        c.observed.wrists[0] = [0.3, 1.4, 0.];
        c.now = 2.;
        c.observed.displayed_pts_ms = 2000;
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Returning);
        assert!(c.motion.prompt.contains("lowers the left hand"));
        assert!(!c.motion.constrain_root_height);
        c.observed.wrists[0][1] = 0.8;
        c.advance(&w);
        assert!(c.active.is_empty());
        assert_eq!(c.events.back().unwrap().status, Status::Completed);
    }
    #[test]
    fn bend_uses_coherent_pose_targets_after_aligning_to_object() {
        let mut c = ready();
        let w = SceneState::default();
        submit(
            &mut c,
            action("bend", Verb::BendAndReturn, None, Some("desk")),
            &w,
        );
        assert_eq!(c.active[0].stage, Stage::Align);
        assert!(!c.motion.prompt.contains("bends forward"));
        let turn = c.motion.timeline.as_ref().unwrap().root.heading_transition.as_ref()
            .expect("desk alignment must not request an instantaneous heading jump");
        assert_eq!(turn.from, c.observed.heading);
        assert_eq!(turn.start_ms, c.observed.displayed_pts_ms);
        assert_eq!(turn.duration_ms, heading_turn_ms(c.observed.heading, c.motion.heading.unwrap()));
        // Alignment must allow standing up and stepping out of the previous
        // gait. Freezing a low walking pelvis made the >0.85 m gate unreachable.
        assert!(!c.motion.constrain_root_height);
        assert!(c
            .motion
            .timeline
            .as_ref()
            .unwrap()
            .pose_keyframes
            .is_empty());
        c.observed.heading = c.motion.heading.unwrap();
        c.observed.torso_up = [0., 1., 0.];
        c.observed.feet = [[0.12, 0.08, 0.], [-0.12, 0.08, 0.]];
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Execute);
        assert_eq!(c.active[0].origin_feet, c.observed.feet);
        assert!(c.motion.joints.iter().all(|j| j.joint != "Spine3"));
        let timeline = c.motion.timeline.as_ref().unwrap();
        let goal = &timeline.pose_keyframes[0];
        assert_eq!(goal["kind"], "model_pose", "a generated destination guides the body");
        assert_eq!(goal["posture"], "bend");
        assert_eq!(goal["pts_ms"], timeline.root.keyframes.last().unwrap().pts_ms);
        assert_eq!(goal["root_position"], serde_json::json!(timeline.root.keyframes.last().unwrap().position));
        assert_eq!(goal["hold"], true);
        assert_ne!(goal["enforce"], true, "guide ARDY without overwriting its generated transition");
        assert_eq!(timeline.pose_keyframes.len(), 1, "do not compete with an unrelated wrist target");
        c.observed.displayed_pts_ms = 10_000;
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Returning);
        let timeline = c.motion.timeline.as_ref().unwrap();
        let goal = &timeline.pose_keyframes[0];
        assert_eq!(goal["posture"], "standing");
        assert_eq!(goal["pts_ms"], 11_400);
        assert_eq!(goal["root_position"], serde_json::json!(timeline.root.keyframes.last().unwrap().position));
        assert_eq!(goal["hold"], true);
        assert_ne!(goal["enforce"], true);
    }
    #[test]
    fn sit_aligns_then_settles_and_keeps_timing_across_revisions() {
        let mut c = ready();
        let w = SceneState::default();
        let chair = c.objects.iter().find(|o| o.id == "chair").unwrap().clone();
        c.observed.pelvis = chair.anchor(chair.approach);
        c.observed.pelvis[1] = 0.95;
        c.observed.heading = std::f32::consts::PI;
        c.observed.displayed_pts_ms = 1000;
        submit(&mut c, action("sit", Verb::Sit, None, Some("chair")), &w);
        let turn = c.motion.timeline.as_ref().unwrap().root.heading_transition.as_ref().unwrap();
        assert_eq!(turn.start_ms, 1000);
        assert_eq!(turn.duration_ms, 2000);
        assert_eq!(turn.from, std::f32::consts::PI);
        let root = &c.motion.timeline.as_ref().unwrap().root;
        assert_eq!(root.keyframes[1].pts_ms, 3000);
        assert_eq!(root.keyframes[0].position, root.keyframes[1].position);
        assert_eq!(root.keyframes.last().unwrap().pts_ms, 4200);
        let goals: Vec<_> = c.motion.timeline.as_ref().unwrap().pose_keyframes.iter()
            .filter(|k| k["kind"] == "model_pose").collect();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0]["posture"], "standing");
        assert_eq!(goals[0]["pts_ms"], 4200);
        assert!((goals[0]["root_position"][2].as_f64().unwrap() as f32
            - chair.seat_staging_position()[2]).abs() < 0.001);
        assert!(c.motion.joints.iter().all(|j| j.contact_pts_ms == 4200));
        assert_eq!(c.active[0].stage, Stage::Align);
        assert!(c.motion.constrain_root_height);
        assert_eq!(c.motion.joints.len(), 2);
        assert!(c.motion.joints.iter().all(|j| j.joint.ends_with("Foot")));
        c.observed.heading = 0.;
        c.advance(&w);
        assert_eq!(
            c.active[0].stage,
            Stage::Align,
            "heading alone cannot start sitting"
        );
        c.observed.torso_up = [0., 1., 0.];
        c.observed.feet = [chair.seat_floor_foot([0.12, 0.08, -0.35]),
                           chair.seat_floor_foot([-0.12, 0.08, -0.35])];
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Align, "turning is not completing the backward step");
        c.observed.displayed_pts_ms = 4200;
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Align, "deadline alone cannot stand in for arrival");
        c.observed.pelvis = chair.seat_staging_position();
        c.observed.pelvis[1] = 0.95;
        c.observed.pelvis[2] -= 0.06; // Arrived within tolerance, not exactly at the planned waypoint.
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Execute);
        assert!(c.motion.ease_root);
        assert_eq!(c.motion.root_waypoints.len(), 3);
        assert_eq!(c.motion.root_waypoints[0][..3], c.motion.root_waypoints[1][..3],
            "descent must not slide forward to an idealized alignment waypoint");
        assert!(c.motion.joints.iter().all(|j| j.joint != "Spine3"));
        let goal = c.motion.timeline.as_ref().unwrap().pose_keyframes.iter()
            .find(|k| k["kind"] == "model_pose").expect("descent needs a generated seated endpoint");
        assert_eq!(goal["posture"], "seated");
        assert_eq!(goal["pts_ms"], 6600);
        assert_eq!(goal["root_position"], serde_json::json!(chair.seat_position().unwrap()));
        assert_ne!(goal["enforce"], true);
        assert_eq!(goal["hold"], true);
        c.observed.displayed_pts_ms = 4700;
        c.rebuild(&w);
        let end = c.motion.root_waypoints.last().unwrap()[3] * 1000. + c.motion.pts_ms as f32;
        assert!((end - 6600.).abs() < 1.);
        c.observed.pelvis = chair.anchor(chair.seat.unwrap());
        c.observed.torso_up = [0., 1., 0.];
        c.observed.feet = [[0.1, 0.08, -0.35], [-0.1, 0.08, -0.35]];
        c.observed.displayed_pts_ms = 6600;
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::Settling);
        assert!(c.motion.prompt.contains("sits gracefully upright"));
        c.advance(&w);
        assert!(c.seated.is_none());
        c.observed.displayed_pts_ms = 7000;
        c.advance(&w);
        assert_eq!(c.seated.as_deref(), Some("chair"));
        assert!(c
            .motion
            .timeline
            .as_ref()
            .unwrap()
            .pose_keyframes
            .iter()
            .any(|k| k["pts_ms"] == 0 && k["keypoints"][0]["joint"] == "Hips"));
    }
    #[test]
    fn future_sit_root_and_contact_key_share_the_same_handoff_clock() {
        let mut c = ready();
        c.future_handoffs = true;
        let w = SceneState::default();
        c.observed.pelvis = [0., 0.95, 0.1];
        c.observed.displayed_pts_ms = 1000;
        submit(&mut c, action("sit", Verb::Sit, None, Some("chair")), &w);
        c.active[0].stage = Stage::Execute;
        c.active[0].execute_pts_ms = 1600;
        c.active[0].execute_origin = [0., 0.95, -0.45];
        c.future_root = Some((1600, [0., 0.95, -0.45]));
        for now in [1000, 1300, 1800] {
            c.observed.displayed_pts_ms = now;
            c.rebuild(&w);
            let timeline = c.motion.timeline.as_ref().unwrap();
            assert_eq!(timeline.root.keyframes.last().unwrap().pts_ms, 4000);
            let pelvis = timeline
                .pose_keyframes
                .iter()
                .find(|k| k["keypoints"][0]["joint"] == "Hips")
                .unwrap();
            assert_eq!(pelvis["pts_ms"], 4000);
        }
    }
    #[test]
    fn seat_support_is_bounded_and_released_for_standing() {
        let mut c = ready();
        let chair = c.objects.iter().find(|o| o.id == "chair").unwrap();
        let seat = Vec3::from(chair.anchor(chair.seat.unwrap()));
        c.seated = Some("chair".into());
        c.observed.pelvis = (seat + Vec3::Y * 0.05).to_array();
        let corrected = c.supported_root(seat + Vec3::Y * 0.08);
        assert!((corrected.y - seat.y - 0.025).abs() < 1e-5);
        let distant = seat + Vec3::Y;
        // Once support is acquired, a bad model proposal cannot detach it.
        assert_eq!(c.supported_root(distant), corrected);
        submit(
            &mut c,
            action("stand", Verb::StandUp, None, None),
            &SceneState::default(),
        );
        let proposed = seat + Vec3::Y * 0.08;
        assert_eq!(c.supported_root(proposed), proposed);
    }
    #[test]
    fn wire_envelope_decodes() {
        let c: OfficeCommand = serde_json::from_str(
            r#"{"kind":"perform_action","action_id":"a","verb":"wave","hand":"left"}"#,
        )
        .unwrap();
        assert!(matches!(c, OfficeCommand::PerformAction { .. }));
    }
    #[test]
    fn concurrent_walk_wave_smile_and_conflicting_hand() {
        let mut c = ready();
        let w = SceneState::default();
        submit(&mut c, action("walk", Verb::Walk, None, Some("desk")), &w);
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Left), None),
            &w,
        );
        submit(&mut c, action("smile", Verb::Smile, None, None), &w);
        submit(
            &mut c,
            action("wave2", Verb::Wave, Some(Hand::Left), None),
            &w,
        );
        assert_eq!(c.active.len(), 3);
        assert_eq!(c.pending.len(), 1);
        assert!(c.motion.prompt.contains("walks"));
        assert!(c.motion.prompt.contains("left hand"));
    }
    #[test]
    fn attached_mug_does_not_complete_pickup_until_carry_posture_settles() {
        let mut c = ready();
        let mut world = SceneState::default();
        submit(&mut c, action("grab", Verb::Grab, Some(Hand::Right), Some("mug")), &world);
        c.active[0].stage = Stage::Execute;
        c.active[0].sent = true;
        c.observed.pelvis = [1.4, 0.78, 0.1];
        c.observed.torso_up = [0., 0.7, 0.7];
        c.observed.displayed_pts_ms = 2000;
        world.grabs.push(super::super::super::renderer::GrabSnapshot {
            hand: Hand::Right, prop: "mug".into(),
        });
        c.advance(&world);
        assert_eq!(c.active.len(), 1, "physical attachment is not upright pickup completion");
        assert_eq!(c.active[0].stage, Stage::Returning);
        assert!(!c.motion.prompt.contains("reaches"));
        assert!(c.motion.joints.iter().all(|target| target.rotation_only));
        let goal = c.motion.timeline.as_ref().unwrap().pose_keyframes.iter()
            .find(|key| key["posture"] == "standing").unwrap();
        assert_eq!(goal["pts_ms"], 3200);
        assert_eq!(goal["carry_hands"], serde_json::json!(["Right"]));
        assert_ne!(goal["enforce"], true);
        c.observed.displayed_pts_ms = 3200;
        c.advance(&world);
        assert!(c.active[0].stable_since_ms.is_none());
        c.observed.pelvis[1] = 0.95;
        c.observed.torso_up = [0., 1., 0.];
        c.advance(&world);
        assert!(c.active[0].stable_since_ms.is_none(), "dangling wrist is not a carrying pose");
        c.observed.wrists[1] = [1.15, 1.02, 0.3];
        c.advance(&world);
        c.observed.displayed_pts_ms = 3500;
        c.advance(&world);
        assert_eq!(c.active.len(), 1);
        c.observed.displayed_pts_ms = 3600;
        c.advance(&world);
        assert!(c.active.is_empty());
        assert_eq!(c.events.back().unwrap().status, Status::Completed);
        assert!(c.grab_commands.is_empty(), "recovery must not re-grab or release the mug");
    }
    #[test]
    fn carry_orientation_blends_once_per_physical_attachment() {
        let mut c = ready();
        let mut world = SceneState::default();
        c.observed.displayed_pts_ms = 1000;
        world.grabs.push(super::super::super::renderer::GrabSnapshot {
            hand: Hand::Right, prop: "mug".into(),
        });
        c.advance(&world);
        c.rebuild(&world);
        let contact = |c: &OfficeController| c.motion.joints.iter()
            .find(|j| j.joint == "RightHand" && j.rotation_only).unwrap().contact_pts_ms;
        assert_eq!(contact(&c), 1000);
        c.observed.displayed_pts_ms = 3000;
        c.observed.heading = 1.2;
        c.advance(&world);
        c.rebuild(&world);
        assert_eq!(contact(&c), 1000, "an action revision must not release an already settled grip");
        let revision = c.revision;
        world.grabs.clear();
        c.advance(&world);
        assert!(c.revision > revision, "physical release must remove the carry target");
        assert!(!c.motion.joints.iter().any(|j| j.rotation_only));
        world.grabs.push(super::super::super::renderer::GrabSnapshot {
            hand: Hand::Right, prop: "mug".into(),
        });
        c.observed.displayed_pts_ms = 5000;
        c.advance(&world);
        assert_eq!(contact(&c), 5000, "reattachment starts a new blend");
    }

    #[test]
    fn mug_grasp_points_right_hand_forward_with_thumb_up() {
        let mut c = ready();
        let object = c.objects.iter_mut().find(|o| o.id == "mug").unwrap();
        for yaw in [0., 0.8, -1.4] {
            object.rotation_xyzw = Some(Quat::from_rotation_y(yaw).to_array());
            let grasp = object.grasp_transform().unwrap();
            let reach = grasp.translation - Vec3::from(object.approach_position());
            let forward = Vec3::new(reach.x, 0., reach.z).normalize();
            // Core right-hand neutral fingers extend along -X; its thumb
            // extends toward +Z. The handle constraint must fit the approach.
            assert!((grasp.rotation * -Vec3::X).dot(forward) > 0.99);
            assert!((grasp.rotation * Vec3::Z).dot(Vec3::Y) > 0.99);
        }
    }

    #[test]
    fn carry_keeps_acquired_grip_frame_across_turns_and_revisions() {
        for hand in [Hand::Left, Hand::Right] {
            let mut c = ready();
            c.observed.heading = -1.2;
            let object = c.objects.iter_mut().find(|o| o.id == "mug").unwrap();
            object.rotation_xyzw = Some(Quat::from_rotation_y(0.4).to_array());
            let acquired = object.grasp_transform().unwrap().rotation;
            let mut world = SceneState::default();
            world.grabs.push(super::super::super::renderer::GrabSnapshot {
                hand, prop: "mug".into(),
            });
            c.advance(&world);
            let joint = if hand == Hand::Left { "LeftHand" } else { "RightHand" };
            for heading in [-1.2, 0.3, 2.0] {
                c.observed.heading = heading;
                c.rebuild(&world);
                let target = c.motion.joints.iter().find(|j| j.joint == joint).unwrap();
                let expected = Quat::from_rotation_y(heading + 1.2) * acquired;
                assert!(Quat::from_array(target.rotation_xyzw).angle_between(expected) < 0.001,
                    "carry must continue the acquired frame, applying only the subsequent body turn");
                // Updating a carried object's world pose must not apply its
                // rotation a second time on the next controller revision.
                c.objects.iter_mut().find(|o| o.id == "mug").unwrap().rotation_xyzw =
                    Some(Quat::from_rotation_y(heading + 1.6).to_array());
                c.advance(&world);
            }
            world.grabs.clear();
            c.advance(&world);
            let acquired_again = c.objects.iter().find(|o| o.id == "mug").unwrap()
                .grasp_transform().unwrap().rotation;
            world.grabs.push(super::super::super::renderer::GrabSnapshot {
                hand, prop: "mug".into(),
            });
            c.advance(&world);
            let target = c.motion.joints.iter().find(|j| j.joint == joint).unwrap();
            assert!(Quat::from_array(target.rotation_xyzw).angle_between(acquired_again) < 0.001,
                "reattachment must acquire a new frame");
        }
    }

    #[test]
    fn occupied_hand_queues_wave_and_cancel_keeps_prop() {
        let mut c = ready();
        let mut w = SceneState::default();
        w.grabs.push(super::super::super::renderer::GrabSnapshot {
            hand: Hand::Right,
            prop: "mug".into(),
        });
        submit(
            &mut c,
            action("wave", Verb::Wave, Some(Hand::Right), None),
            &w,
        );
        assert_eq!(c.pending.len(), 1);
        c.rebuild(&w);
        assert!(
            !c.motion
                .joints
                .iter()
                .any(|j| j.joint == "RightHand" && !j.rotation_only),
            "carrying must not retain the pickup wrist target"
        );
        assert!(c.motion.prompt.contains("carries mug upright"));
        let grip = c
            .motion
            .timeline
            .as_ref()
            .unwrap()
            .contacts
            .iter()
            .find(|j| j.joint == "RightHand")
            .expect("carrying needs orientation guidance");
        assert!(grip.rotation_only);
        let orientation = Quat::from_array(grip.rotation_xyzw);
        let local_grip = c.objects.iter().find(|o| o.id == "mug").unwrap()
            .local_keypoint("grasp").unwrap().rotation;
        assert!((orientation * local_grip.inverse() * Vec3::Y).distance(Vec3::Y) < 1e-5,
            "the mug must remain upright; its wrist frame need not have world-up Y");
        c.command(
            OfficeCommand::CancelAction {
                action_id: "wave".into(),
            },
            &w,
        );
        assert!(c.pending.is_empty());
        assert!(c.grab_commands.is_empty());
        assert_eq!(w.grabs.len(), 1);
    }
    #[test]
    fn seated_idle_keeps_its_support_contact_without_exempting_other_furniture() {
        let mut c = ready();
        let chair = c.objects.iter().find(|o| o.id == "chair").unwrap();
        c.observed.pelvis = chair.seat_position().unwrap();
        c.seated = Some("chair".into());
        assert!(c.permits_root(Vec3::from(c.observed.pelvis)));
        assert!(c.permits_root(Vec3::from(c.observed.pelvis) + Vec3::X * 0.01));
        assert!(!c.permits_root(Vec3::new(0., 0.6, -1.8)));
        c.seated = None;
        assert!(!c.permits_root(Vec3::from(c.observed.pelvis)));
    }
    #[test]
    fn movement_completes_only_at_observed_destination() {
        let mut c = ready();
        let w = SceneState::default();
        submit(&mut c, action("walk", Verb::Walk, None, Some("desk")), &w);
        c.now = 10.;
        c.advance(&w);
        assert_eq!(c.active.len(), 1);
        let goal = c.objects.iter().find(|o| o.id == "desk").unwrap();
        let p = goal.anchor(goal.approach);
        c.observed.pelvis = [p[0], 0.97, p[2]];
        c.advance(&w);
        assert!(c.active.is_empty());
        assert_eq!(c.events.back().unwrap().status, Status::Completed);
    }
    #[test]
    fn standing_requires_settled_upright_exit_before_releasing_chair() {
        let mut c = ready();
        let w = SceneState::default();
        c.seated = Some("chair".into());
        c.observed.pelvis = [0., 0.6, -0.8];
        submit(&mut c, action("stand", Verb::StandUp, None, None), &w);
        let root = &c.motion.timeline.as_ref().unwrap().root;
        assert!((root.keyframes.last().unwrap().position[2] - 0.1).abs() < 1e-5);
        let endpoints = &c.motion.timeline.as_ref().unwrap().pose_keyframes;
        assert_eq!(endpoints.len(), 2);
        for (key, waypoint) in endpoints.iter().zip(&root.keyframes[2..]) {
            assert_eq!(key["pts_ms"], waypoint.pts_ms);
            assert_eq!(key["root_position"], serde_json::json!(waypoint.position));
            assert_eq!(key["kind"], "model_pose");
            assert_eq!(key["hold"], true);
            assert_ne!(key["enforce"], true);
        }

        // The live failure crossed the old arrival tolerance while still tipped
        // and stepping. It must keep the chair transition and generation track.
        c.observed.displayed_pts_ms = 3000;
        c.observed.pelvis = [-0.034, 0.954, -0.020];
        c.observed.torso_up = [0., 0.885, 0.465];
        c.advance(&w);
        assert_eq!(c.active[0].stage, Stage::StandingSettling);
        assert!(c.motion.prompt.contains("stands still"));
        assert!(!c.motion.prompt.contains("stands up"));
        assert_eq!(c.seated.as_deref(), Some("chair"));

        c.observed.pelvis = [0., 0.95, 0.1];
        c.observed.torso_up = [0., 1., 0.];
        for pts in [3100, 3400] {
            c.observed.displayed_pts_ms = pts;
            c.advance(&w);
            assert_eq!(c.seated.as_deref(), Some("chair"));
        }
        c.observed.displayed_pts_ms = 3500;
        c.advance(&w);
        assert!(c.seated.is_none());
        assert!(c.active.is_empty());
        assert!(c.permits_root(Vec3::from(c.observed.pelvis)));
    }

    #[test]
    fn walk_from_seat_stands_before_navigation() {
        let mut c = ready();
        c.seated = Some("chair".into());
        c.observed.pelvis = [0., 0.6, -0.8];
        let w = SceneState::default();
        submit(&mut c, action("walk", Verb::Walk, None, Some("desk")), &w);
        assert_eq!(c.active[0].stage, Stage::Standing);
        assert!(c.motion.prompt.contains("stands up"));
        c.command(
            OfficeCommand::CancelAction {
                action_id: "walk".into(),
            },
            &w,
        );
        assert_eq!(c.seated.as_deref(), Some("chair"));
    }
    #[test]
    fn root_validation_rejects_collisions_instead_of_repairing_them() {
        let mut c = ready();
        c.observed.pelvis = [1.2504, 0.79, -1.81];
        // A 5 cm drift into the chair's planning margin is minor: planning still
        // refuses it, but generated native motion plays on unchanged.
        let minor = Vec3::new(1.20, 0.83, -1.80);
        assert!(!c.permits_root(minor));
        assert_eq!(c.resolve_root(minor), Some(minor));
        // Walking into the chair itself is still rejected, never repaired.
        let chair = c.objects.iter().find(|o| o.id == "chair").unwrap().position;
        let proposed = Vec3::new(chair[0], 0.83, chair[2]);
        assert!(!c.permits_native_root(proposed));
        // Starting inside the chair's native reserve, leaving is allowed only for the
        // seat the avatar was measured on; never deeper; planning keeps the strict rule.
        let saved = c.observed.pelvis;
        c.observed.pelvis = [chair[0] + 0.45, 0.83, chair[2]];
        assert!(!c.permits_native_root(Vec3::new(chair[0] + 0.52, 0.83, chair[2])), "only a measured seat");
        c.leaving_seat = Some("chair".into());
        assert!(c.permits_native_root(Vec3::new(chair[0] + 0.52, 0.83, chair[2])));
        assert!(!c.permits_native_root(Vec3::new(chair[0] + 0.40, 0.83, chair[2])));
        assert!(!c.permits_root(Vec3::new(chair[0] + 0.52, 0.83, chair[2])));
        c.leaving_seat = None;
        c.observed.pelvis = saved;
        assert!(c.resolve_root(proposed).is_none());
        assert!(c.resolve_root(Vec3::new(0.8, 0.83, -1.8)).is_none());
        let safe = Vec3::from(c.observed.pelvis);
        assert_eq!(c.resolve_root(safe), Some(safe));
    }
    #[test]
    fn native_seat_admission_is_revision_scoped_and_preserves_other_obstacles() {
        use crate::cc_render::renderer::SkeletalPose;
        let mut c = ready();
        let world = SceneState::default();
        c.command(OfficeCommand::InstallNativeSeatGeometry { geometry: std::sync::Arc::new(
            super::super::avatar_geometry::AvatarGeometry::test_fixture()) }, &world);
        let mut previous = SkeletalPose::rest();
        previous.stream = Some(super::super::timeline::StreamIdentity { session_id:"test".into(),
            generation_revision:1,skeleton_id:"smpl22".into(),root_mode:"world".into() });
        previous.root_pos = [0.,1.,0.1];
        c.observed.pelvis = previous.root_pos;
        c.record_native_pose(&previous);
        let mut candidate = previous.clone(); candidate.pts_ms = 50;
        candidate.stream.as_mut().unwrap().generation_revision = 2;
        candidate.root_pos = [0.,1.,-0.8];
        assert!(c.resolve_native_pose(&candidate).is_none());
        c.command(OfficeCommand::AuthorizeNativeSeat { session_id:"test".into(),generation_revision:2,target_id:"chair".into() }, &world);
        assert_eq!(c.resolve_native_pose(&candidate), Some(Vec3::from(candidate.root_pos)));
        candidate.stream.as_mut().unwrap().generation_revision = 3;
        assert!(c.resolve_native_pose(&candidate).is_none());
        candidate.stream.as_mut().unwrap().generation_revision = 2;
        candidate.root_pos[2] = -1.8;
        assert!(c.resolve_native_pose(&candidate).is_none()); // Desk is never exempted.
        candidate.root_pos = [-1.,0.42,-0.8];
        // Owner-approved default: target-seat overlaps warn, never repair or stop the pose.
        assert_eq!(c.resolve_native_pose(&candidate), Some(Vec3::from(candidate.root_pos)));
        assert_eq!(candidate.root_pos, [-1.,0.42,-0.8]);
        assert!(serde_json::from_value::<OfficeCommand>(serde_json::json!({"kind":"authorize_native_seat",
            "session_id":"test","generation_revision":3,"target_id":"chair"})).is_err());
    }
    #[test]
    fn blocked_destination_and_invalid_input_fail() {
        let mut c = ready();
        let w = SceneState::default();
        let mut a = action("bad", Verb::Walk, None, None);
        a.position = Some([0., 0., -1.8]);
        submit(&mut c, a, &w);
        assert_eq!(c.events.back().unwrap().status, Status::Failed);
        let mut a = action("nan", Verb::Wave, None, None);
        a.intensity = f32::NAN;
        submit(&mut c, a, &w);
        assert_eq!(c.events.back().unwrap().status, Status::Failed);
    }
}
