use super::controller::*;
use crate::cc_render::bevy_app::{
    physics::{
        grab::GrabFsms,
        props::{Prop, PropRegistry, SceneStateTx},
    },
    pose::{CC5BindRotations, SMPL_PRIMARY_CHILD},
};
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

/// Route office actions before handing physical grab/release commands to the
/// existing FSM. Legacy callers use the same inbox without an office resource.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct OfficeCommandInbox<'w> {
    rx: Option<Res<'w, crate::cc_render::bevy_app::physics::props::GrabCmdRx>>,
    office: Option<ResMut<'w, OfficeController>>,
    scene: Option<Res<'w, SceneStateTx>>,
}
impl OfficeCommandInbox<'_> {
    pub fn drain(&mut self) -> Vec<crate::cc_render::renderer::GrabOrRelease> {
        use crate::cc_render::renderer::GrabOrRelease;
        let mut commands = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(command) = rx.0.lock().unwrap().try_recv() {
                match command {
                    GrabOrRelease::Office(action) => {
                        if let (Some(office), Some(scene)) =
                            (self.office.as_mut(), self.scene.as_ref())
                        {
                            office.command(action, &scene.0.borrow());
                        }
                    }
                    command => commands.push(command),
                }
            }
        }
        if let Some(office) = self.office.as_mut() {
            commands.append(&mut office.grab_commands);
        }
        commands
    }
}

pub fn install(app: &mut App) {
    // Body output is model-owned. No contact repair or goal-directed actuator
    // may be registered here; tests/test_motion_ownership.py guards this seam.
    let mut controller = OfficeController::default();
    let mut bundled = true;
    if let Some(package) = app.world().get_resource::<crate::cc_render::scene_package::ScenePackage>() {
        controller.objects = package.office_objects();
        controller.constrained_objects = package.mechanisms.iter().flat_map(|m| &m.parts)
            .filter(|part| part.joint.is_some()).map(|part| part.id.clone()).collect();
        controller.homes = super::controller::home_poses(&controller.objects);
        bundled = false;
    } else if let Ok(path) = std::env::var("CC_OFFICE_SCENE") {
        match std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<super::OfficeObject>>(&s).ok())
            .filter(|objects| super::validate_objects(objects).is_ok())
        {
            Some(objects) => {
                controller.homes = super::controller::home_poses(&objects);
                controller.objects = objects;
                bundled = false;
            }
            None => tracing::error!("CC_OFFICE_SCENE invalid; using bundled starter office"),
        }
    }
    app.insert_resource(controller)
        .insert_resource(super::home::BundledHome(bundled))
        .init_resource::<super::hand::HandRigs>()
        .init_resource::<super::hand::HeldInHand>()
        .add_systems(Startup, spawn_office)
        .add_systems(Update, super::hand::capture_hand_rigs)
        .add_systems(PreUpdate, classify_animated_colliders)
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::transform::TransformSystem::TransformPropagate)
                .after(crate::cc_render::bevy_app::physics::props::publish_scene_state),
        )
        .add_systems(
            PostUpdate,
            // After `observe`: its physics sync still carries the pre-release pose this frame.
            super::coupling::apply_couplings.after(bevy::transform::TransformSystem::TransformPropagate).after(observe),
        );
}
fn spawn_office(
    mut commands: Commands,
    office: Res<OfficeController>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut registry: ResMut<PropRegistry>,
    layout: Option<Res<super::home::BundledHome>>,
    package: Option<Res<crate::cc_render::scene_package::ScenePackage>>,
) {
    // Package assets and collision solids share one authoring source.
    if package.is_some() { return; }
    for o in &office.objects {
        let h = Vec3::from(o.half_extents);
        let material = materials.add(Color::srgb(o.color[0], o.color[1], o.color[2]));
        let mut entity = commands.spawn((
            Name::new(o.label.clone()),
            o.transform(),
            Visibility::default(),
            if o.dynamic {
                RigidBody::Dynamic
            } else {
                RigidBody::Fixed
            },
            Friction::coefficient(0.7),
        ));
        if o.parts.is_empty() {
            let mesh = if o.id == "mug" {
                meshes.add(Cylinder::new(h.x, h.y * 2.))
            } else {
                meshes.add(Cuboid::from_size(h * 2.))
            };
            entity.insert((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Collider::cuboid(h.x, h.y, h.z),
            ));
        } else {
            entity.with_children(|parent| {
                for part in &o.parts {
                    let size = Vec3::from(part.half_extents);
                    let mut part_entity = parent.spawn((
                        Mesh3d(if part.cylinder {
                            meshes.add(Cylinder::new(size.x, size.y * 2.))
                        } else {
                            meshes.add(Cuboid::from_size(size * 2.))
                        }),
                        MeshMaterial3d(part.color.map_or_else(
                            || material.clone(),
                            |c| materials.add(Color::srgb(c[0], c[1], c[2])),
                        )),
                        Transform::from_translation(Vec3::from(part.position)),
                        Collider::cuboid(size.x, size.y, size.z),
                    ));
                    // Rapier reads groups from each Collider; child shapes do
                    // not inherit the rigid body's collision filtering.
                    if o.dynamic {
                        part_entity.insert(CollisionGroups::new(
                            Group::GROUP_1,
                            Group::ALL & !Group::GROUP_32,
                        ));
                    }
                }
            });
        }
        if o.dynamic {
            if o.pushable {
                entity.insert((
                    LockedAxes::ROTATION_LOCKED,
                    Damping { linear_damping: 4.0, angular_damping: 8.0 },
                    ExternalForce::default(),
                ));
            }
            entity.insert((
                Prop {
                    id: o.id.clone(),
                    loading: false,
                    grasp_local: o
                        .local_keypoint("grasp")
                        .or_else(|| o.grasp.map(|p| Transform::from_translation(Vec3::from(p))))
                        .map(|p| p.compute_matrix().to_cols_array_2d()),
                },
                // Animation-driven bodies must not launch props before a
                // verified grasp. Props still collide with the physical world.
                CollisionGroups::new(Group::GROUP_1, Group::ALL & !Group::GROUP_32),
                Velocity::default(),
                AdditionalMassProperties::Mass(if o.pushable { 8.0 } else { 0.3 }),
            ));
            registry.by_id.insert(o.id.clone(), entity.id());
        }
    }
    super::home::spawn_surfaces(
        &mut commands,
        &mut meshes,
        &mut materials,
        layout.as_ref().is_none_or(|layout| layout.0),
    );
}
/// A contact-actuated force, not a position animation. Rapier owns furniture
/// displacement and collisions; the controller only supplies the requested
/// direction and the hands must reach the observed near face first.
#[cfg(test)]
fn apply_furniture_push(
    office: Res<OfficeController>,
    mut furniture: Query<(&Prop, &Transform, &Velocity, &mut ExternalForce)>,
) {
    let intent = office.push_force_intent();
    for (prop, transform, velocity, mut force) in &mut furniture {
        force.force = Vec3::ZERO;
        force.torque = Vec3::ZERO;
        if let Some(plan) = intent.as_ref().filter(|plan| plan.id == prop.id) {
            force.force = furniture_push_force(plan, transform.translation, velocity.linvel,
                Vec3::from(office.observed.pelvis), office.observed.wrists.map(Vec3::from));
        }
    }
}

#[cfg(test)]
fn furniture_push_force(plan: &PushForceIntent, center: Vec3, velocity: Vec3,
    pelvis: Vec3, wrists: [Vec3; 2]) -> Vec3 {
    let remaining = (plan.goal - center).dot(plan.direction);
    if remaining < 0.03 || (plan.goal - center).length() > 1.25 { return Vec3::ZERO; }
    let contact = center + plan.contact_offset;
    if wrists.iter().any(|w| w.distance(contact) > 0.22)
        || (contact - pelvis).dot(plan.direction) < 0.12
    { return Vec3::ZERO; }
    let object_progress = (center - plan.start).dot(plan.direction);
    let avatar_progress = (pelvis - plan.avatar_start).dot(plan.direction);
    if object_progress > avatar_progress + 0.05 { return Vec3::ZERO; }
    let forward_speed = velocity.dot(plan.direction);
    // Static friction must be overcome, but cap speed so the body can
    // follow the chair rather than letting it skate ahead of the hands.
    if forward_speed >= 0.28 { return Vec3::ZERO; }
    plan.direction * (100.0 + 24.0 * (0.28 - forward_speed))
        * plan.intensity.clamp(0.2, 1.0)
}

/// Torso (pelvis -> neck) tilt from world up beyond which the CC5 body is upside
/// down. Generated ARDY motion reaches ~113 degrees in a deep forward fold (head
/// 0.26 m below the pelvis, picking something off a low table); a retarget or
/// calibration flip puts the torso near 180 degrees.
const CC5_TORSO_UPSIDE_DOWN_DEG: f32 = 150.;

/// A last-line CC5 FK guard against retarget and skinning defects, independent
/// of the source-model gate. It rejects bodies that no real pose produces
/// (non-finite, scattered, stretched, through the floor, upside down), not poses
/// that are merely unusual: deep bends, folds, crouches and lying down all pass.
/// It reads only the observed body, never the controller's posture label, which
/// describes the requested action rather than the pose on screen.
fn extreme_body_deformation(pelvis: Vec3, neck: Vec3, head: Vec3) -> Option<String> {
    if !pelvis.is_finite() || !neck.is_finite() || !head.is_finite() {
        return Some("extreme_deformation: non-finite CC5 body position".into());
    }
    if neck.distance(pelvis) < 0.05 {
        return Some("extreme_deformation: CC5 torso collapsed (neck at pelvis)".into());
    }
    let tilt = (neck - pelvis).angle_between(Vec3::Y).to_degrees();
    if !tilt.is_finite() || tilt > CC5_TORSO_UPSIDE_DOWN_DEG {
        return Some(format!(
            "extreme_deformation: CC5 torso upside down ({tilt:.0} degrees from upright)"));
    }
    None
}

fn extreme_skeleton_deformation(joints: &[Vec3; 22]) -> Option<String> {
    let pelvis = joints[0];
    if joints.iter().any(|joint| !joint.is_finite()) {
        return Some("extreme_deformation: non-finite CC5 joint".into());
    }
    if joints.iter().any(|joint| joint.distance(pelvis) > 2.5) {
        return Some("extreme_deformation: CC5 joint outside body envelope".into());
    }
    // A reclining or kneeling body may put the head below the hips, but no
    // posture permits a leg or wrist to pass substantially through the floor.
    if [4, 5, 7, 8, 10, 11, 20, 21]
        .into_iter()
        .any(|index| joints[index].y < -0.20)
    {
        return Some("extreme_deformation: CC5 limb penetrates floor".into());
    }
    extreme_body_deformation(pelvis, joints[12], joints[15])
}

/// Head-on-neck rotation is anatomical, so it applies in every pose.
fn extreme_head_rotation(neck_delta: Quat, head_delta: Quat) -> Option<String> {
    if !neck_delta.is_finite() || !head_delta.is_finite()
        || neck_delta.length_squared() < 0.5 || head_delta.length_squared() < 0.5
    {
        return Some("extreme_deformation: invalid CC5 head rotation".into());
    }
    let relative = (neck_delta.inverse() * head_delta).normalize();
    let degrees = 2. * relative.w.abs().clamp(0., 1.).acos().to_degrees();
    if degrees > 85. {
        return Some(format!("extreme_deformation: CC5 head twisted {degrees:.1} degrees relative to neck"));
    }
    None
}

fn extreme_bone_stretch(joints: &[Vec3; 22], rest_lengths: &[f32; 22]) -> Option<String> {
    for (i, child) in SMPL_PRIMARY_CHILD.iter().enumerate() {
        let Some(child) = child else { continue };
        let rest = rest_lengths[i];
        if rest < 0.025 || !rest.is_finite() {
            continue; // coincident facial/auxiliary joints are not length probes
        }
        let ratio = joints[i].distance(joints[*child]) / rest;
        if !ratio.is_finite() || !(0.45..=1.85).contains(&ratio) {
            return Some(format!(
                "extreme_deformation: CC5 bone {i}->{child} length {:.2} times rest",
                ratio
            ));
        }
    }
    None
}

fn observe(
    mut office: ResMut<OfficeController>,
    bind: Res<CC5BindRotations>,
    globals: Query<&GlobalTransform>,
    tx: Res<SceneStateTx>,
    time: Res<Time>,
    _fsms: Res<GrabFsms>,
) {
    office.now = time.elapsed_secs_f64();
    if let Some(p) = bind.entities[0].and_then(|e| globals.get(e).ok()) {
        office.observed.ready = bind.captured && office.motion_ready;
        office.observed.position_valid = bind.captured;
        office.observed.pelvis = p.translation().to_array();
    }
    if let (Some(left), Some(right)) = (
        bind.entities[1].and_then(|e| globals.get(e).ok()),
        bind.entities[2].and_then(|e| globals.get(e).ok()),
    ) {
        if let Some(heading) = hip_heading(left.translation(), right.translation()) {
            office.observed.heading = heading;
        }
    }
    for (i, j) in [20, 21].iter().enumerate() {
        if let Some(p) = bind.entities[*j].and_then(|e| globals.get(e).ok()) {
            office.observed.wrists[i] = p.translation().to_array();
        }
    }
    for (i, j) in [7, 8].iter().enumerate() {
        if let Some(p) = bind.entities[*j].and_then(|e| globals.get(e).ok()) {
            office.observed.feet[i] = p.translation().to_array();
        }
    }
    if let Some(neck) = bind.entities[12].and_then(|e| globals.get(e).ok()) {
        // Observe the actual torso line. A chest joint's orientation alone can
        // look upright while the lower spine and the body remain folded over.
        office.observed.torso_up = (neck.translation() - Vec3::from(office.observed.pelvis))
            .normalize_or_zero()
            .to_array();
    }
    if std::env::var("OFFICE_DEBUG_MOTION").as_deref() == Ok("1") {
        let deltas: Option<Vec<_>> = [9, 12, 15].into_iter().map(|joint| {
            let transform = globals.get(bind.entities[joint]?).ok()?;
            Some((transform.rotation() * bind.calibrated_world[joint].inverse()).normalize().to_array())
        }).collect();
        office.observed.debug_rotation_deltas = deltas.and_then(|v| v.try_into().ok());
    }
    let world = tx.0.borrow().clone();
    if office.observed.ready && office.motion_error.is_none() {
        let joints: Option<Vec<Vec3>> = bind.entities.iter().map(|entity|
            entity.and_then(|e| globals.get(e).ok()).map(|gt| gt.translation())
        ).collect();
        let error = match joints.and_then(|j| j.try_into().ok()) {
            Some(joints) => extreme_skeleton_deformation(&joints)
                .or_else(|| extreme_bone_stretch(&joints, &bind.rest_bone_lengths))
                .or_else(|| {
                    let neck = globals.get(bind.entities[12]?).ok()?.rotation()
                        * bind.calibrated_world[12].inverse();
                    let head = globals.get(bind.entities[15]?).ok()?.rotation()
                        * bind.calibrated_world[15].inverse();
                    extreme_head_rotation(neck, head)
                }),
            None => Some("extreme_deformation: missing CC5 joint".into()),
        };
        if let Some(error) = error {
            office.command(OfficeCommand::MotionError { error }, &world);
            tx.0.send_modify(|s| s.office = Some(office.snapshot()));
            return;
        }
    }
    let moved = office.update_prop_observations(&world.props);
    // Compile the first idle only after the avatar's world position is known.
    // Otherwise a slow LLM turn starts with an unconstrained root-only stream.
    if office.observed.position_valid && office.motion.timeline.is_none() {
        office.rebuild(&world);
    }
    office.advance(&world);
    if (moved || office.blocked) && office.now - office.last_replan >= 0.5 {
        office.blocked = false;
        office.last_replan = office.now;
        office.rebuild(&world);
    }
    tx.0.send_modify(|s| s.office = Some(office.snapshot()));
}

/// Facing on the floor, independent of pelvis pitch and bone-local bind axes.
/// Euler yaw can flip by pi when a sitting pelvis passes its pitch singularity.
fn hip_heading(left: Vec3, right: Vec3) -> Option<f32> {
    let lateral = left - right;
    (lateral.x * lateral.x + lateral.z * lateral.z > 1e-8).then(|| (-lateral.z).atan2(lateral.x))
}

#[cfg(test)]
mod home_tests {
    use super::*;

    #[test]
    fn furniture_force_requires_two_hands_and_pelvis_follow_through() {
        let intent = PushForceIntent { id: "chair".into(), start: Vec3::new(0., 0.23, 0.8),
            goal: Vec3::new(0., 0.23, 1.02), avatar_start: Vec3::new(0., 0.95, 0.17),
            direction: Vec3::Z, contact_offset: Vec3::new(0., 0.85, -0.3), intensity: 1. };
        let wrists = [Vec3::new(0.15, 1.08, 0.5), Vec3::new(-0.15, 1.08, 0.5)];
        assert!(furniture_push_force(&intent, intent.start, Vec3::ZERO,
            intent.avatar_start, wrists).z > 0.);
        assert_eq!(furniture_push_force(&intent, intent.start, Vec3::ZERO,
            intent.avatar_start, [wrists[0], Vec3::new(-0.15, 1.08, 0.)]), Vec3::ZERO);
        assert_eq!(furniture_push_force(&intent, intent.start + Vec3::Z * 0.12,
            Vec3::ZERO, intent.avatar_start, wrists), Vec3::ZERO,
            "furniture must not skate ahead while ARDY's root is blocked");
    }

    #[test]
    fn home_spawns_furniture_materials_and_floor_for_every_room() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<PropRegistry>()
            .insert_resource(OfficeController::default())
            .add_systems(Startup, spawn_office);
        app.update();
        let world = app.world_mut();
        let names: Vec<_> = world
            .query::<&Name>()
            .iter(world)
            .map(|n| n.as_str().to_owned())
            .collect();
        for object in super::super::starter_objects() {
            assert!(names.contains(&object.label), "missing {}", object.id);
        }
        assert_eq!(world.query::<&PointLight>().iter(world).count(), 5);
        let floor = world
            .query::<(&Name, &Transform, &Collider)>()
            .iter(world)
            .find(|(name, _, _)| name.as_str() == "Home floor")
            .unwrap();
        let half = floor.2.as_cuboid().unwrap().half_extents();
        for room in super::super::home::ROOMS {
            assert!(room.min_xz[0] >= floor.1.translation.x - half.x);
            assert!(room.max_xz[0] <= floor.1.translation.x + half.x);
            assert!(room.min_xz[1] >= floor.1.translation.z - half.z);
            assert!(room.max_xz[1] <= floor.1.translation.z + half.z);
        }
        let colors = world.resource::<Assets<StandardMaterial>>();
        assert!(
            colors
                .iter()
                .any(|(_, m)| m.base_color == Color::srgb(0.18, 0.37, 0.44)),
            "desk screen should use its authored part color"
        );
    }
}

#[cfg(test)]
mod heading_tests {
    use super::*;

    #[test]
    fn cc5_torso_check_rejects_upside_down_bodies_not_deep_bends() {
        let hips = Vec3::new(0., 0.9, 0.);
        let pose = |tilt_deg: f32| {
            let dir = Vec3::new(tilt_deg.to_radians().sin(), tilt_deg.to_radians().cos(), 0.);
            (hips + dir * 0.5, hips + dir * 0.7)
        };
        // Upright, a forward bend over a low table (head 3 cm above the pelvis,
        // the frame that stopped the coffee-table pickup), a deep fold with the
        // head 0.26 m below the pelvis, and lying flat are all real poses.
        for tilt in [0., 82., 113., 90.] {
            let (neck, head) = pose(tilt);
            assert!(extreme_body_deformation(hips, neck, head).is_none(), "tilt {tilt}");
        }
        let (neck, head) = pose(170.);
        assert!(extreme_body_deformation(hips, neck, head).unwrap().contains("upside down"));
        assert!(extreme_body_deformation(hips, Vec3::splat(f32::NAN), Vec3::Y).is_some());
        assert!(extreme_body_deformation(hips, hips, Vec3::Y).unwrap().contains("collapsed"));
    }

    #[test]
    fn cc5_gate_checks_all_joints_even_when_reclining() {
        let mut joints = [Vec3::new(0., 0.5, 0.); 22];
        joints[12].y = 1.1;
        joints[15].y = 1.4;
        for index in [4, 5, 7, 8, 10, 11] { joints[index].y = 0.02; }
        joints[20].y = -0.08; // mild floor contact is not a failure
        assert!(extreme_skeleton_deformation(&joints).is_none());
        joints[10].y = -0.25;
        assert!(extreme_skeleton_deformation(&joints).unwrap().contains("penetrates floor"));
        joints[10].y = 0.02;
        joints[21].x = 3.;
        assert!(extreme_skeleton_deformation(&joints).unwrap().contains("body envelope"));
        joints[21].x = f32::NAN;
        assert!(extreme_skeleton_deformation(&joints).unwrap().contains("non-finite"));
    }

    #[test]
    fn cc5_head_twist_fails_without_waiting_for_torso_position_to_invert() {
        let normal = Quat::from_rotation_y(25_f32.to_radians());
        assert!(extreme_head_rotation(Quat::IDENTITY, normal).is_none());
        let twisted = Quat::from_rotation_y(100_f32.to_radians());
        assert!(extreme_head_rotation(Quat::IDENTITY, twisted).unwrap().contains("head twisted"));
        assert!(extreme_head_rotation(Quat::from_xyzw(0., 0., 0., 0.), normal).is_some());
    }

    #[test]
    fn cc5_gate_rejects_stretched_and_collapsed_bones() {
        let mut joints = [Vec3::ZERO; 22];
        let mut lengths = [0.; 22];
        let child = SMPL_PRIMARY_CHILD[0].unwrap();
        lengths[0] = 0.3;
        joints[child] = Vec3::new(0., 0.3, 0.);
        assert!(extreme_bone_stretch(&joints, &lengths).is_none());
        joints[child].y = 0.6;
        assert!(extreme_bone_stretch(&joints, &lengths).unwrap().contains("extreme_deformation"));
        joints[child].y = 0.05;
        assert!(extreme_bone_stretch(&joints, &lengths).unwrap().contains("extreme_deformation"));
    }

    #[test]
    fn multipart_prop_colliders_keep_the_animated_body_exclusion() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<PropRegistry>()
            .insert_resource(OfficeController::default())
            .add_systems(Startup, spawn_office);
        app.update();
        let world = app.world_mut();
        let mug = world
            .query::<(Entity, &Prop)>()
            .iter(world)
            .find(|(_, p)| p.id == "mug")
            .unwrap()
            .0;
        let mut shapes = 0;
        for (parent, groups) in world
            .query_filtered::<(&Parent, Option<&CollisionGroups>), With<Collider>>()
            .iter(world)
        {
            if parent.get() == mug {
                let groups = groups.expect("each prop collider needs its own groups");
                assert!(!groups.filters.intersects(Group::GROUP_32));
                assert!(groups.filters.intersects(Group::GROUP_1));
                shapes += 1;
            }
        }
        assert!(shapes >= 4, "exercise the body and handle colliders");
    }

    #[test]
    fn facing_survives_pelvis_pitch_singularity() {
        for yaw in [-2.8_f32, -0.5, 0., 1.8, 3.0] {
            for pitch in [0., 1.4, 1.7, 2.8] {
                let rotation = Quat::from_rotation_y(yaw)
                    * Quat::from_rotation_z(0.2)
                    * Quat::from_rotation_x(pitch);
                let origin = Vec3::new(2., 0.6, -4.);
                let lateral = rotation * Vec3::X;
                let actual = hip_heading(origin + lateral * 0.1, origin - lateral * 0.1).unwrap();
                assert!((actual - yaw).abs() < 1e-5);
            }
        }
        assert!(hip_heading(Vec3::Y, Vec3::ZERO).is_none());
        assert!(hip_heading(Vec3::ZERO, Vec3::ZERO).is_none());
    }
}

/// Office-only collision policy; legacy scenes retain their existing contacts.
fn classify_animated_colliders(
    mut commands: Commands,
    bodies: Query<
        Entity,
        (
            With<crate::cc_render::bevy_app::physics::body_colliders::BodyBoneCollider>,
            Without<CollisionGroups>,
        ),
    >,
) {
    for entity in &bodies {
        commands
            .entity(entity)
            .insert(CollisionGroups::new(Group::GROUP_32, Group::ALL));
    }
}
