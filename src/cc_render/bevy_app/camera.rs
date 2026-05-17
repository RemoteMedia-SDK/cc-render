//! Camera + lighting + scene setup.
//!
//! Camera + 3-light portrait setup ported verbatim from the
//! prototype's tuned values (see `examples/avatar-render-proto`).

use bevy::{
    core_pipeline::{bloom::Bloom, tonemapping::Tonemapping},
    pbr::{light_consts::lux, prelude::EnvironmentMapLight, CascadeShadowConfigBuilder},
    prelude::*,
    render::{
        camera::RenderTarget,
        render_asset::RenderAssetUsages,
        render_resource::{
            Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
        },
    },
    ui::IsDefaultUiCamera,
};

use super::capture::CaptureTarget;
use super::pose::{CC5BindRotations, PoseAnimation};

/// Tuned camera defaults (sitting CC5 avatar, head world ≈ (0, 1.10, 0.887)).
/// Values match the prototype's final framing.
pub(crate) const CAMERA_POS: Vec3 = Vec3::new(0.0, 1.10, -0.8);
pub(crate) const CAMERA_LOOK_AT: Vec3 = Vec3::new(0.0, 1.10, 0.887);
pub(crate) const CAMERA_FOV_DEG: f32 = 65.0;

/// Parse `"x,y,z"` from an env var into a `Vec3`, falling back to
/// `default` when the var is absent or malformed. Used to expose
/// `CAMERA_POS` / `CAMERA_LOOK_AT` / `CAMERA_FOV_DEG` as
/// per-invocation tunables without recompiling.
fn env_vec3(key: &str, default: Vec3) -> Vec3 {
    let Ok(s) = std::env::var(key) else {
        return default;
    };
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() != 3 {
        tracing::warn!(
            target: "cc_render",
            "env {key}: expected 'x,y,z', got {s:?} — using default {default:?}"
        );
        return default;
    }
    let parse = |i: usize| parts[i].parse::<f32>().ok();
    match (parse(0), parse(1), parse(2)) {
        (Some(x), Some(y), Some(z)) => Vec3::new(x, y, z),
        _ => {
            tracing::warn!(
                target: "cc_render",
                "env {key}: failed to parse {s:?} as 3 floats — using default {default:?}"
            );
            default
        }
    }
}

fn env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Like `env_vec3` but returns `None` when the env var is unset rather
/// than falling back to a default. Used by `--fit-motion` so the
/// motion-envelope expansion is opt-in (absent = no inflation).
fn env_vec3_opt(key: &str) -> Option<Vec3> {
    let s = std::env::var(key).ok()?;
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() != 3 {
        tracing::warn!(target: "cc_render", "env {key}: expected 'x,y,z', got {s:?}");
        return None;
    }
    let x = parts[0].parse().ok()?;
    let y = parts[1].parse().ok()?;
    let z = parts[2].parse().ok()?;
    Some(Vec3::new(x, y, z))
}

/// Effective camera position — `CC_AVATAR_CAMERA_POS=x,y,z` overrides
/// the const default. Read by both `setup_scene` and
/// `auto_orient_avatar` so they stay in sync.
pub(crate) fn camera_pos() -> Vec3 {
    env_vec3("CC_AVATAR_CAMERA_POS", CAMERA_POS)
}
pub(crate) fn camera_look_at() -> Vec3 {
    env_vec3("CC_AVATAR_CAMERA_LOOK_AT", CAMERA_LOOK_AT)
}
pub(crate) fn camera_fov_deg() -> f32 {
    env_f32("CC_AVATAR_CAMERA_FOV_DEG", CAMERA_FOV_DEG)
}

/// Resolve `CC_AVATAR_FOCUS` (case-insensitive) into a list of SMPL-22
/// joint indices defining the body part to frame on, plus a small
/// padding (meters) added to the resulting AABB so the part isn't
/// jammed against the frustum edges.
///
/// Presets cover the natural cinematic targets: face/head close-ups,
/// torso/upper-body, single-arm/leg, and per-hand. Returns `None` if
/// the env var is unset, or the preset name is unrecognized (with a
/// warning).
///
/// SMPL-22 indices, per `pose::SMPL22_TO_CC5`:
///   0 Pelvis  1 L_Hip  2 R_Hip  3 Spine1  4 L_Knee  5 R_Knee
///   6 Spine2  7 L_Ankle  8 R_Ankle  9 Spine3 10 L_Foot 11 R_Foot
///  12 Neck   13 L_Collar 14 R_Collar 15 Head 16 L_Shoulder 17 R_Shoulder
///  18 L_Elbow 19 R_Elbow 20 L_Wrist 21 R_Wrist
/// Per-preset focus tuning: which joint(s) to anchor on, and how much
/// to pad the resulting joint-AABB along each axis. Padding is
/// asymmetric (pad_below_y vs pad_above_y) because most body-part
/// joints don't sit at the centroid of their visible mesh — the Head
/// joint, for instance, lives at the neck-base, but the actual face
/// extends ~0.20m UP and only ~0.05m DOWN from there. Asymmetric
/// padding shifts the AABB center to match the visible part.
struct FocusPreset {
    joints: &'static [usize],
    pad_below_y: f32,
    pad_above_y: f32,
    pad_horiz: f32,
    /// When true, position the camera on the avatar's BACK side (i.e.
    /// opposite of the auto-oriented "front"). Used for `butt` so the
    /// view actually shows the rear instead of the front of the body.
    view_from_back: bool,
}

/// Sentinel "front-view" template used with struct update syntax
/// (`FocusPreset { joints: ..., ..FRONT_DEFAULT }`) so existing
/// front-facing presets don't have to spell `view_from_back: false`.
const FRONT_DEFAULT: FocusPreset = FocusPreset {
    joints: &[],
    pad_below_y: 0.0,
    pad_above_y: 0.0,
    pad_horiz: 0.0,
    view_from_back: false,
};

fn focus_spec() -> Option<FocusPreset> {
    let raw = std::env::var("CC_AVATAR_FOCUS").ok()?;
    let key = raw.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    let preset: FocusPreset = match key.as_str() {
        // Head joint = base of skull; the actual face/skull mesh
        // extends ~0.30m UP and only ~0.05m down. Asymmetric pad lifts
        // AABB center to eye level and pulls camera back enough to
        // frame the whole face + hair.
        "head" => FocusPreset {
            joints: &[12, 15],
            pad_below_y: 0.05,
            pad_above_y: 0.32,
            pad_horiz: 0.20,
            ..FRONT_DEFAULT
        },
        "face" => FocusPreset {
            joints: &[15],
            pad_below_y: 0.05,
            pad_above_y: 0.28,
            pad_horiz: 0.16,
            ..FRONT_DEFAULT
        },
        "neck" => FocusPreset {
            joints: &[12, 15],
            pad_below_y: 0.10,
            pad_above_y: 0.18,
            pad_horiz: 0.18,
            ..FRONT_DEFAULT
        },
        "torso" => FocusPreset {
            joints: &[0, 3, 6, 9, 12],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.18,
            ..FRONT_DEFAULT
        },
        "upper" | "upper_body" | "upper-body" => FocusPreset {
            joints: &[6, 9, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21],
            pad_below_y: 0.08,
            pad_above_y: 0.18,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "arms" => FocusPreset {
            joints: &[16, 17, 18, 19, 20, 21],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "l_arm" | "left_arm" | "larm" => FocusPreset {
            joints: &[16, 18, 20],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "r_arm" | "right_arm" | "rarm" => FocusPreset {
            joints: &[17, 19, 21],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "legs" => FocusPreset {
            joints: &[1, 2, 4, 5, 7, 8, 10, 11],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.12,
            ..FRONT_DEFAULT
        },
        "l_leg" | "left_leg" | "lleg" => FocusPreset {
            joints: &[1, 4, 7, 10],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "r_leg" | "right_leg" | "rleg" => FocusPreset {
            joints: &[2, 5, 8, 11],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "hands" => FocusPreset {
            joints: &[20, 21],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "l_hand" | "left_hand" | "lhand" => FocusPreset {
            joints: &[20],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "r_hand" | "right_hand" | "rhand" => FocusPreset {
            joints: &[21],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "pelvis" | "hip" | "hips" => FocusPreset {
            joints: &[0],
            pad_below_y: 0.20,
            pad_above_y: 0.20,
            pad_horiz: 0.20,
            ..FRONT_DEFAULT
        },
        "feet" => FocusPreset {
            joints: &[10, 11],
            pad_below_y: 0.10,
            pad_above_y: 0.10,
            pad_horiz: 0.10,
            ..FRONT_DEFAULT
        },
        "shoulders" => FocusPreset {
            joints: &[13, 14, 16, 17],
            pad_below_y: 0.12,
            pad_above_y: 0.12,
            pad_horiz: 0.12,
            ..FRONT_DEFAULT
        },
        // Upper chest area. Spine3 sits at the breastbone; the
        // collarbones (13, 14) widen the frame to include both
        // breasts. Padding pulls the AABB DOWN to mid-ribcage and
        // sideways to capture the full bust width — a plain
        // collar+spine AABB would crop the breast mass that hangs
        // below the joints.
        "chest" | "breasts" | "bust" | "torso_upper" | "torso-upper" => FocusPreset {
            joints: &[9, 13, 14],
            pad_below_y: 0.18,
            pad_above_y: 0.05,
            pad_horiz: 0.22,
            ..FRONT_DEFAULT
        },
        // Buttocks / glutes. Anchored on Pelvis + the two hip joints;
        // pad DOWN to the upper thigh so the glute mass that hangs
        // below the hip joint is included, pad UP enough to catch
        // the lower-back transition. Horizontal pad is generous to
        // capture both cheeks plus the hip silhouette.
        // `view_from_back: true` flips the camera to look at the
        // rear of the avatar instead of the auto-oriented front.
        "butt" | "buttocks" | "glutes" | "rear" => FocusPreset {
            joints: &[0, 1, 2],
            pad_below_y: 0.22,
            pad_above_y: 0.10,
            pad_horiz: 0.20,
            view_from_back: true,
        },
        _ => {
            tracing::warn!(
                target: "cc_render",
                "CC_AVATAR_FOCUS={raw:?} not recognized — supported: \
                 head, face, neck, torso, upper_body, arms, l_arm, \
                 r_arm, legs, l_leg, r_leg, hands, l_hand, r_hand, \
                 pelvis, feet, shoulders, chest, butt. Ignoring.",
            );
            return None;
        }
    };
    Some(preset)
}

/// World-space AABB enclosing the given SMPL joint world positions
/// (each cached on `bind.entities[i]` after capture). Returns
/// `(min, max, count)`; an empty count means no joint entities were
/// resolvable yet (caller must wait + retry).
fn focus_aabb_from_joints(
    preset: &FocusPreset,
    bind: &CC5BindRotations,
    transforms: &Query<&GlobalTransform>,
) -> (Vec3, Vec3, usize) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut count = 0;
    for &i in preset.joints {
        if i >= 22 {
            continue;
        }
        let Some(e) = bind.entities[i] else { continue };
        let Ok(gt) = transforms.get(e) else { continue };
        let p = gt.translation();
        if !p.is_finite() {
            continue;
        }
        min = min.min(p);
        max = max.max(p);
        count += 1;
    }
    if count > 0 {
        // Asymmetric inflation so the AABB centroid lands on the
        // visible mesh, not the often off-center bone pivot. For
        // head/face: `pad_above_y` >> `pad_below_y` shifts the
        // center upward to match the actual face mesh.
        min.x -= preset.pad_horiz;
        min.z -= preset.pad_horiz;
        min.y -= preset.pad_below_y;
        max.x += preset.pad_horiz;
        max.z += preset.pad_horiz;
        max.y += preset.pad_above_y;
    }
    (min, max, count)
}

/// One-shot camera frame fit: scan all skinned-mesh + scene AABBs once
/// the GLB has loaded, then push the camera back along its current
/// look direction so the union AABB fits within the frustum (with a
/// small margin). Re-aims the camera at the AABB center while preserving
/// its current yaw/pitch around that center.
///
/// Gated by `CC_AVATAR_FIT_FRAME=1`. The fit runs once after the scene
/// settles (a few frames after bind capture so SceneSpawner has
/// populated `Aabb`s). Idempotent.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fit_camera_to_avatar(
    bind: Res<CC5BindRotations>,
    aabbs: Query<(Entity, &GlobalTransform, &bevy::render::primitives::Aabb)>,
    transforms: Query<&GlobalTransform>,
    parent_q: Query<&Parent>,
    avatar_q: Query<Entity, With<AvatarRoot>>,
    mut cam_q: Query<(&mut Transform, &Projection), (With<Camera3d>, Without<DirectionalLight>)>,
    mut done: Local<bool>,
    mut settle_ticks: Local<u32>,
) {
    if *done {
        return;
    }
    if std::env::var("CC_AVATAR_FIT_FRAME").ok().as_deref() != Some("1") {
        *done = true;
        return;
    }
    if !bind.captured {
        return;
    }
    // Wait a handful of ticks after bind capture so SceneSpawner has
    // had time to instantiate skinned meshes + their `Aabb` components.
    *settle_ticks += 1;
    if *settle_ticks < 6 {
        return;
    }

    // Compute the AABB to fit. With `CC_AVATAR_FOCUS=<part>`, replace
    // the whole-scene mesh AABB with a tight box around the named
    // SMPL joints (head/face close-up, hand framing, etc.). Otherwise
    // union every renderable Aabb in the scene.
    let focus = focus_spec();
    let (mut min, mut max, count): (Vec3, Vec3, usize) = if let Some(preset) = &focus {
        let (mn, mx, c) = focus_aabb_from_joints(preset, &bind, &transforms);
        if c == 0 {
            // Joints not resolvable yet — bail and retry next tick.
            return;
        }
        tracing::info!(
            target: "cc_render",
            "fit: focus={} ({} joints, pad below/above/horiz={:.2}/{:.2}/{:.2}m) \
             — framing on body part",
            std::env::var("CC_AVATAR_FOCUS").unwrap_or_default(),
            c, preset.pad_below_y, preset.pad_above_y, preset.pad_horiz,
        );
        (mn, mx, c)
    } else {
        // Restrict the AABB union to the avatar SceneRoot subtree so a
        // separately-loaded environment scene's mesh extents don't
        // expand the framing. Falls through to "all aabbs" when no
        // AvatarRoot is registered (matches old behavior pre-marker).
        let avatars: Vec<Entity> = avatar_q.iter().collect();
        let scope_to_avatar = !avatars.is_empty();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut count: usize = 0;
        for (entity, gt, aabb) in aabbs.iter() {
            if scope_to_avatar && !is_descendant_of_any(entity, &parent_q, &avatars) {
                continue;
            }
            let aff = gt.affine();
            let center = aff.transform_point3(aabb.center.into());
            let he = Vec3::from(aabb.half_extents);
            let m = aff.matrix3.abs();
            let world_he = Vec3::new(
                m.x_axis.x * he.x + m.y_axis.x * he.y + m.z_axis.x * he.z,
                m.x_axis.y * he.x + m.y_axis.y * he.y + m.z_axis.y * he.z,
                m.x_axis.z * he.x + m.y_axis.z * he.y + m.z_axis.z * he.z,
            );
            min = min.min(center - world_he);
            max = max.max(center + world_he);
            count += 1;
        }
        (min, max, count)
    };
    if count == 0 || !min.is_finite() || !max.is_finite() {
        return;
    }

    // `--fit-motion` (CC_AVATAR_FIT_MOTION_MIN/MAX) inflates the bind
    // AABB by the per-frame `root_pos` envelope of the entire motion
    // (pre-scanned by render_clip.sh). Result: a single static camera
    // position that frames every pose the avatar will visit, instead
    // of just the bind silhouette.
    if let Some(motion_min) = env_vec3_opt("CC_AVATAR_FIT_MOTION_MIN") {
        min = min.min(motion_min);
    }
    if let Some(motion_max) = env_vec3_opt("CC_AVATAR_FIT_MOTION_MAX") {
        max = max.max(motion_max);
    }

    let Ok((mut cam_xform, projection)) = cam_q.get_single_mut() else {
        return;
    };
    let Projection::Perspective(persp) = projection else {
        *done = true;
        return;
    };

    let center = (min + max) * 0.5;
    let he = (max - min) * 0.5; // axis-aligned half-extents

    // Project the world-axis AABB onto the camera's view basis to get
    // tight half-width / half-height / half-depth in screen space.
    // Bounding-sphere fitting (radius = he.length()) over-estimates
    // significantly for tall, thin characters — projecting per-axis
    // gives a much tighter framing.
    let cur = cam_xform.translation;
    let mut dir = if let Some(preset) = &focus {
        // Focus mode: face the body part from the avatar's FRONT at
        // eye-level. The body-shot camera is tilted downward, so just
        // zooming in would frame the head from above. Instead we
        // build a NEW horizontal direction:
        //   axis = (current_cam - pelvis) projected to XZ plane
        // After `auto_orient_avatar`, the camera sits on the avatar's
        // forward side, so this XZ vector points along the avatar's
        // facing direction. Dropping Y makes the new camera sit at
        // the focus point's elevation — face-level for head, hand-
        // level for hand focus, etc.
        //
        // Presets with `view_from_back: true` (e.g. `butt`) negate
        // this so the camera sits on the avatar's REAR side.
        let pelvis = bind.pelvis_world.unwrap_or(center);
        let xz = Vec3::new(cur.x - pelvis.x, 0.0, cur.z - pelvis.z);
        let xz = xz.normalize_or_zero();
        if preset.view_from_back {
            -xz
        } else {
            xz
        }
    } else {
        (cur - center).normalize_or_zero()
    };
    if dir == Vec3::ZERO {
        dir = -Vec3::Z;
    }
    // Camera basis: forward points FROM camera TO subject (= -dir);
    // up is preserved from current camera orientation as best we can.
    let forward = -dir;
    let world_up = Vec3::Y;
    let right = forward.cross(world_up).normalize_or_zero();
    let right = if right == Vec3::ZERO { Vec3::X } else { right };
    let up = right.cross(forward).normalize();

    // Half-extents along each camera axis: |axis · he_signed| summed
    // over the 3 world-axis components, which equals |a.x|*he.x +
    // |a.y|*he.y + |a.z|*he.z for axis-aligned box.
    let proj = |axis: Vec3| axis.x.abs() * he.x + axis.y.abs() * he.y + axis.z.abs() * he.z;
    let half_w = proj(right);
    let half_h = proj(up);
    let half_d = proj(forward);

    let aspect = persp.aspect_ratio.max(1e-3);
    let fov_y = persp.fov;
    let tan_half_y = (fov_y * 0.5).tan().max(1e-4);
    let tan_half_x = tan_half_y * aspect;

    // Distance from AABB CENTER such that the front face fits the
    // frustum: max of (height-fit, width-fit), then push back by
    // half_d so the front face (closest to camera) is inside the FOV.
    let dist_h = half_h / tan_half_y;
    let dist_w = half_w / tan_half_x;
    let margin = env_f32("CC_AVATAR_FIT_MARGIN", 1.02);
    let dist = (dist_h.max(dist_w) + half_d) * margin;

    let new_pos = center + dir * dist;
    cam_xform.translation = new_pos;
    cam_xform.look_at(center, Vec3::Y);

    tracing::info!(
        target: "cc_render",
        "fit: AABB min={:?} max={:?} center={:?} \
         half(w/h/d)=({:.3},{:.3},{:.3}) \
         → cam_pos={:?} dist={:.3} (margin={:.2}, {} aabbs)",
        min, max, center, half_w, half_h, half_d, new_pos, dist, margin, count,
    );
    *done = true;
}

/// One-shot auto-orient: rotate the SceneRoot around Y so the avatar
/// faces the camera, regardless of which way the GLB's bind pose was
/// authored. Reads the eye axis captured by `capture_cc5_bind_rotations`
/// (R_Eye − L_Eye = avatar's right), derives bind-time forward
/// (`Y × right`), and applies a single Y-axis rotation to align it with
/// the camera direction.
///
/// Gated by `CC_AVATAR_AUTO_FACE_CAMERA` (default on). Set to `0` to
/// keep the GLB's authored orientation.
pub(crate) fn auto_orient_avatar(
    bind: Res<CC5BindRotations>,
    scenes: Query<&Transform, With<SceneRoot>>,
    mut cam_lights: Query<
        &mut Transform,
        (
            Or<(With<Camera3d>, With<DirectionalLight>)>,
            Without<SceneRoot>,
        ),
    >,
    mut done: Local<bool>,
) {
    if *done || !bind.captured {
        return;
    }
    if std::env::var("CC_AVATAR_AUTO_FACE_CAMERA").ok().as_deref() == Some("0") {
        *done = true;
        tracing::info!(
            target: "cc_render",
            "auto-orient: CC_AVATAR_AUTO_FACE_CAMERA=0, skipping"
        );
        return;
    }
    let (Some(pelvis_w), Some(eye_axis)) = (bind.pelvis_world, bind.eye_axis_world) else {
        // Eye bones absent or world transforms not yet propagated.
        return;
    };

    // Sanity: in a Y-up scene the eye axis should be largely horizontal.
    let xz_mag2 = eye_axis.x * eye_axis.x + eye_axis.z * eye_axis.z;
    if eye_axis.y * eye_axis.y > xz_mag2 * 4.0 {
        tracing::warn!(
            target: "cc_render",
            "auto-orient: eye axis {:?} dominated by Y — non-standard up axis? Skipping.",
            eye_axis,
        );
        *done = true;
        return;
    }

    let right_xz = Vec3::new(eye_axis.x, 0.0, eye_axis.z).normalize_or_zero();
    if right_xz == Vec3::ZERO {
        return;
    }
    // forward = Y × right (Y-up right-handed)
    let forward = Vec3::Y.cross(right_xz).normalize();

    let cam_pos = camera_pos();
    let target = Vec3::new(cam_pos.x - pelvis_w.x, 0.0, cam_pos.z - pelvis_w.z).normalize_or_zero();
    if target == Vec3::ZERO {
        *done = true;
        return;
    }

    let dot = forward.dot(target).clamp(-1.0, 1.0);
    let cross_y = forward.cross(target).y;
    let mut angle = cross_y.atan2(dot);

    // Polarity guard: if the rig has L/R eye swapped, the rotation we
    // computed lands forward 180° away from target. Detect + flip.
    let rotated = Quat::from_rotation_y(angle) * forward;
    if rotated.dot(target) < 0.0 {
        tracing::warn!(
            target: "cc_render",
            "auto-orient: polarity guard tripped — flipping 180° \
             (forward={:?}, target={:?}, dot_after={})",
            forward, target, rotated.dot(target),
        );
        angle += std::f32::consts::PI;
    }

    // Don't rotate the SceneRoot — that would force us to also patch
    // the bind-frame rest data captured by `capture_cc5_bind_rotations`
    // (otherwise FK retargets relative to a stale rest frame and arms
    // end up tucked behind the body), and any clean math for that
    // patch ends up frame-changing `src_local` (the SMPL pose), which
    // twists each joint's swing axis (arms permanently raised at 180°).
    //
    // Instead: rotate the *camera + portrait lights* around the
    // avatar's pelvis_y axis by the same angle. Avatar bind frame stays
    // canonical, FK math stays untouched, and the viewer ends up on
    // whatever side the avatar's bind forward points to.
    let pivot = Vec3::new(pelvis_w.x, 0.0, pelvis_w.z); // y unaffected
    let r_orbit = Quat::from_rotation_y(angle);
    let mut moved = 0usize;
    for mut t in cam_lights.iter_mut() {
        // Rotate translation around the pivot.
        let off = t.translation - pivot;
        let off_rot = r_orbit * off;
        t.translation = pivot + off_rot;
        // Rotate orientation by the same amount so look-at stays on target.
        t.rotation = r_orbit * t.rotation;
        moved += 1;
    }
    tracing::info!(
        target: "cc_render",
        "auto-orient: orbited {} camera+light entities by {:.1}° around Y \
         pivot={:?} (eye_axis={:?}, forward={:?}, target={:?})",
        moved, angle.to_degrees(), pivot, eye_axis, forward, target,
    );
    let _ = (scenes, &bind); // keep params; intentionally not mutated
    *done = true;
}

/// Render target dimensions; passed in via the renderer config.
///
/// `scene_filename` is `Some` when an environment scene GLB has been
/// configured separately from the avatar. It's loaded through the
/// `scene://` asset source (registered in `bevy_app::run`) so the
/// scene file can live anywhere on disk.
#[derive(Resource, Clone)]
pub(crate) struct GlbAsset {
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub scene_filename: Option<String>,
}

/// Marker for the avatar's `SceneRoot` entity. Lets systems that
/// should only act on the avatar (vs. an optional environment scene
/// loaded alongside it) filter their queries via `With<AvatarRoot>`.
#[derive(Component)]
pub(crate) struct AvatarRoot;

/// Marker for the optional environment scene's `SceneRoot` entity.
#[derive(Component)]
pub(crate) struct EnvironmentSceneRoot;

/// Strip authored lights that the environment scene GLB ships with so
/// they don't layer on top of the renderer's 3-light portrait rig.
///
/// Toggle via `CC_SCENE_KEEP_LIGHTS`:
///   - unset / `1` (default): keep scene lights — useful when the
///     environment is the only light source.
///   - `0`: remove `DirectionalLight` / `PointLight` / `SpotLight`
///     components from any entity descended from `EnvironmentSceneRoot`
///     as Bevy's `SceneSpawner` instantiates it.
///
/// We remove the light *components* rather than despawning the carrier
/// entity so any non-light children, transforms, or naming hierarchy
/// in the GLB remain intact.
pub(crate) fn strip_scene_lights(
    mut commands: Commands,
    new_lights: Query<Entity, Or<(Added<DirectionalLight>, Added<PointLight>, Added<SpotLight>)>>,
    parent_q: Query<&Parent>,
    env_root_q: Query<Entity, With<EnvironmentSceneRoot>>,
) {
    if std::env::var("CC_SCENE_KEEP_LIGHTS").ok().as_deref() != Some("0") {
        return;
    }
    let env_roots: Vec<Entity> = env_root_q.iter().collect();
    if env_roots.is_empty() {
        return;
    }
    let mut stripped = 0_usize;
    for entity in new_lights.iter() {
        if !is_descendant_of_any(entity, &parent_q, &env_roots) {
            continue;
        }
        commands
            .entity(entity)
            .remove::<DirectionalLight>()
            .remove::<PointLight>()
            .remove::<SpotLight>();
        stripped += 1;
    }
    if stripped > 0 {
        tracing::info!(
            target: "cc_render",
            "scene: stripped {} authored light(s) from environment scene \
             (CC_SCENE_KEEP_LIGHTS=0)",
            stripped,
        );
    }
}

/// True iff `entity` is itself in `roots` or descended from any of them.
/// Bounded parent walk; safe against cycles in pathological scenes.
fn is_descendant_of_any(entity: Entity, parent_q: &Query<&Parent>, roots: &[Entity]) -> bool {
    if roots.is_empty() {
        return false;
    }
    if roots.contains(&entity) {
        return true;
    }
    let mut current = entity;
    for _ in 0..64 {
        let Ok(parent_ref) = parent_q.get(current) else {
            return false;
        };
        let parent = parent_ref.get();
        if roots.contains(&parent) {
            return true;
        }
        current = parent;
    }
    false
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn setup_scene(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut pose_anim: ResMut<PoseAnimation>,
    asset_server: Res<AssetServer>,
    glb: Res<GlbAsset>,
) {
    // ─── Headless render target ────────────────────────────────────────
    let size = Extent3d {
        width: glb.width,
        height: glb.height,
        depth_or_array_layers: 1,
    };
    let mut target_image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("cc_render_target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        asset_usage: RenderAssetUsages::default(),
        ..default()
    };
    target_image.resize(size);
    let target_handle = images.add(target_image);
    commands.insert_resource(CaptureTarget {
        image: target_handle.clone(),
        width: glb.width,
        height: glb.height,
    });

    // ─── Camera ────────────────────────────────────────────────────────
    commands
        .spawn((
            Camera3d::default(),
            Camera {
                target: RenderTarget::Image(target_handle),
                clear_color: ClearColorConfig::Custom({
                    let bg = env_vec3_opt("CC_AVATAR_CLEAR_COLOR")
                        .unwrap_or(Vec3::new(0.10, 0.09, 0.08));
                    Color::srgb(bg.x, bg.y, bg.z)
                }),
                hdr: true,
                ..default()
            },
            Msaa::Sample4,
            Tonemapping::AgX,
            Bloom {
                intensity: 0.04,
                low_frequency_boost: 0.5,
                high_pass_frequency: 0.95,
                ..default()
            },
            EnvironmentMapLight {
                // `env://` is registered as a separate AssetSource in
                // bevy_app/mod.rs so KTX2 maps load from a fixed location
                // regardless of the GLB's directory.
                diffuse_map: asset_server.load("env://pisa_diffuse_rgb9e5_zstd.ktx2"),
                specular_map: asset_server.load("env://pisa_specular_rgb9e5_zstd.ktx2"),
                intensity: 8.0,
                rotation: Quat::IDENTITY,
            },
            Transform::from_translation(camera_pos()).looking_at(camera_look_at(), Vec3::Y),
            // Mark this camera as bevy_ui's default render target so the
            // joint-debug overlay (text labels) draws into the same
            // texture as the 3D scene. Harmless when no UI nodes exist.
            IsDefaultUiCamera,
        ))
        .insert(Projection::Perspective(PerspectiveProjection {
            fov: camera_fov_deg().to_radians(),
            ..default()
        }));

    // ─── 3-light portrait setup ────────────────────────────────────────
    // Key — pushed hard to upper-right so glossy visors/goggles reflect
    // a single highlight in the top-right of the lens, not dead-center.
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.00, 0.93, 0.82),
            illuminance: lux::AMBIENT_DAYLIGHT * 0.025,
            shadows_enabled: true,
            shadow_depth_bias: 0.04,
            shadow_normal_bias: 1.0,
            ..default()
        },
        Transform::from_xyz(2.5, 3.5, 1.2).looking_at(Vec3::new(0.0, 1.55, 0.0), Vec3::Y),
        CascadeShadowConfigBuilder {
            num_cascades: 2,
            maximum_distance: 3.0,
            first_cascade_far_bound: 1.0,
            ..default()
        }
        .build(),
    ));
    // Fill — front-left, dimmed so it doesn't paint a second highlight on
    // chrome/visor surfaces opposite the key.
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.00, 0.88, 0.84),
            illuminance: lux::AMBIENT_DAYLIGHT * 0.008,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(-1.0, 1.4, 1.2).looking_at(Vec3::new(0.0, 1.55, 0.0), Vec3::Y),
    ));
    // Hair / back kicker — also nudged right so its reflection lands on
    // the same upper-right quadrant as the key, not a separate bright dot.
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.00, 0.92, 0.82),
            illuminance: lux::AMBIENT_DAYLIGHT * 0.020,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(1.6, 2.6, -1.0).looking_at(Vec3::new(0.0, 1.55, 0.0), Vec3::Y),
    ));
    // Top hair fill — moved off-center to the right so the visor doesn't
    // pick up a top-of-head specular halo running across the lens.
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.00, 0.92, 0.80),
            illuminance: lux::AMBIENT_DAYLIGHT * 0.012,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(1.5, 3.0, 0.3).looking_at(Vec3::new(0.0, 1.55, 0.0), Vec3::Y),
    ));
    commands.insert_resource(AmbientLight {
        color: Color::srgb(1.00, 0.88, 0.83),
        brightness: env_f32("CC_AVATAR_AMBIENT", 50.0),
    });

    // ─── Avatar scene ──────────────────────────────────────────────────
    let scene: Handle<Scene> = asset_server.load(format!("{}#Scene0", glb.filename));
    commands.spawn((SceneRoot(scene), Transform::IDENTITY, AvatarRoot));

    // ─── Optional environment scene ────────────────────────────────────
    // Loaded through the `scene://` asset source registered in
    // `bevy_app::run` so the scene GLB can live anywhere on disk.
    //
    // Placement env-var tunables (all optional, applied to the scene's
    // SceneRoot Transform — they don't affect the avatar):
    //   CC_SCENE_POS=x,y,z         — translation, default 0,0,0
    //   CC_SCENE_ROT_DEG_Y=<deg>   — rotation around Y, default 0
    //   CC_SCENE_SCALE=<f>         — uniform scale, default 1
    if let Some(scene_filename) = glb.scene_filename.as_deref() {
        let env_scene: Handle<Scene> =
            asset_server.load(format!("scene://{}#Scene0", scene_filename));
        let scene_pos = env_vec3_opt("CC_SCENE_POS").unwrap_or(Vec3::ZERO);
        let scene_rot_deg_y = env_f32("CC_SCENE_ROT_DEG_Y", 0.0);
        let scene_scale = env_f32("CC_SCENE_SCALE", 1.0).max(1e-6);
        let scene_transform = Transform {
            translation: scene_pos,
            rotation: Quat::from_rotation_y(scene_rot_deg_y.to_radians()),
            scale: Vec3::splat(scene_scale),
        };
        commands.spawn((SceneRoot(env_scene), scene_transform, EnvironmentSceneRoot));
        tracing::info!(
            target: "cc_render",
            "scene: spawned environment SceneRoot from scene://{}#Scene0 \
             (pos={:?} rot_deg_y={:.1} scale={:.3})",
            scene_filename, scene_pos, scene_rot_deg_y, scene_scale,
        );
    }

    // FBX-exported pose animation. Bevy 0.15 doesn't auto-play; we
    // build the graph here, and `start_pose_animation` wires it onto
    // the AnimationPlayer once SceneSpawner spawns the entity.
    let clip: Handle<AnimationClip> = asset_server.load(format!("{}#Animation0", glb.filename));
    let (graph, node) = AnimationGraph::from_clip(clip);
    let graph_handle = graphs.add(graph);
    pose_anim.graph = Some(graph_handle);
    pose_anim.node = Some(node);
}

/// Per-frame camera tracker — translates the camera so the avatar's
/// pelvis stays at the same screen-space position it had after the
/// initial fit. Preserves the camera's framing/distance and only adds
/// translation; orientation is re-aimed at the moving pelvis each tick.
///
/// Anchors on the SMPL-22 pelvis bone (`bind.entities[0]` =
/// `CC_Base_Hip`). On the first valid tick, records the offset
/// `cam_pos - pelvis_world` and reuses it forever — so wherever the
/// pelvis goes, the camera follows by the same offset.
///
/// Optional smoothing: `CC_AVATAR_FOLLOW_LERP` (0.0–1.0, default 1.0
/// = snap). Set to 0.15 for cinematic dampening.
///
/// Gated by `CC_AVATAR_FOLLOW=1`. Reads pelvis from the PREVIOUS
/// frame's `GlobalTransform` (system runs before
/// `TransformPropagate`), so there's a 1-frame visual lag — invisible
/// at 30fps and avoids needing to walk the bone hierarchy by hand.
pub(crate) fn follow_avatar_camera(
    bind: Res<CC5BindRotations>,
    transforms: Query<&GlobalTransform>,
    mut cam_q: Query<&mut Transform, (With<Camera3d>, Without<DirectionalLight>)>,
    mut anchor: Local<Option<Vec3>>,
) {
    if std::env::var("CC_AVATAR_FOLLOW").ok().as_deref() != Some("1") {
        return;
    }
    if !bind.captured {
        return;
    }

    // Anchor: focus-centroid if `CC_AVATAR_FOCUS` is set, else pelvis.
    // Using the focus centroid means the camera tracks the body part
    // the user actually wants centered (e.g. head close-ups stay
    // glued to the head as it moves).
    let pelvis_pos = if let Some(preset) = focus_spec() {
        // For follow, anchor on the centroid of the focus joints PLUS
        // half of `pad_above_y` so the camera tracks the actual mesh
        // (face) rather than the joint (neck-base). Same upward bias
        // as the fit AABB center.
        let mut sum = Vec3::ZERO;
        let mut n = 0_f32;
        for &i in preset.joints {
            if i >= 22 {
                continue;
            }
            let Some(e) = bind.entities[i] else { continue };
            let Ok(gt) = transforms.get(e) else { continue };
            sum += gt.translation();
            n += 1.0;
        }
        if n == 0.0 {
            return;
        }
        let mut centroid = sum / n;
        centroid.y += (preset.pad_above_y - preset.pad_below_y) * 0.5;
        centroid
    } else {
        let Some(pelvis_e) = bind.entities[0] else {
            return;
        };
        let Ok(pelvis_gt) = transforms.get(pelvis_e) else {
            return;
        };
        pelvis_gt.translation()
    };
    if !pelvis_pos.is_finite() {
        return;
    }

    let Ok(mut cam_xform) = cam_q.get_single_mut() else {
        return;
    };

    if anchor.is_none() {
        // Lock the offset between camera and pelvis at the start of
        // tracking — this is what we'll keep constant.
        *anchor = Some(cam_xform.translation - pelvis_pos);
        return;
    }
    let offset = anchor.unwrap();
    let target_cam_pos = pelvis_pos + offset;

    let lerp = env_f32("CC_AVATAR_FOLLOW_LERP", 1.0).clamp(0.0, 1.0);
    cam_xform.translation = if lerp >= 0.999 {
        target_cam_pos
    } else {
        cam_xform.translation.lerp(target_cam_pos, lerp)
    };
    // Re-aim. Look-at target also smoothed via the same translation
    // lerp implicitly (since cam_pos drives where Y-up look-at lands).
    cam_xform.look_at(pelvis_pos, Vec3::Y);
}
