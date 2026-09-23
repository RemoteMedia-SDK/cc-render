use super::controller::*;
use crate::cc_render::bevy_app::{
    physics::{
        grab::GrabFsms,
        props::{Prop, PropRegistry, SceneStateTx},
    },
    pose::CC5BindRotations,
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
    // Diagnostic A/B switch: disable only post-retarget hand/foot contact IK.
    // Source constraints, seat support, idle damping and grips remain active.
    let contact_corrections = std::env::var("CC_OFFICE_CONTACT_CORRECTIONS").as_deref() != Ok("0");
    if !contact_corrections {
        tracing::warn!("CC_OFFICE_CONTACT_CORRECTIONS=0: renderer contact IK disabled");
    }
    let mut controller = OfficeController::default();
    let mut bundled = true;
    if let Ok(path) = std::env::var("CC_OFFICE_SCENE") {
        match std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<super::OfficeObject>>(&s).ok())
            .filter(|objects| super::validate_objects(objects).is_ok())
        {
            Some(objects) => {
                controller.objects = objects;
                bundled = false;
            }
            None => tracing::error!("CC_OFFICE_SCENE invalid; using bundled starter office"),
        }
    }
    app.insert_resource(controller)
        .insert_resource(super::home::BundledHome(bundled))
        .add_systems(Startup, spawn_office)
        .add_systems(PreUpdate, classify_animated_colliders)
        .add_systems(
            PostUpdate,
            (
                super::contact::damp_seated_idle,
                super::contact::correct_contacts.run_if(move || contact_corrections),
                super::contact::grasp_fingers,
            )
                .chain()
                .after(crate::cc_render::bevy_app::pose::apply_skeletal_pose)
                .before(bevy_rapier3d::plugin::PhysicsSet::SyncBackend)
                .before(bevy::transform::TransformSystem::TransformPropagate),
        )
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::transform::TransformSystem::TransformPropagate)
                .after(crate::cc_render::bevy_app::physics::props::publish_scene_state),
        );
}
fn spawn_office(
    mut commands: Commands,
    office: Res<OfficeController>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut registry: ResMut<PropRegistry>,
    layout: Option<Res<super::home::BundledHome>>,
) {
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
                AdditionalMassProperties::Mass(0.3),
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
    let mut moved = false;
    for prop in &world.props {
        let carried = office.held.iter().any(|(_, id)| id == &prop.id);
        if let Some(o) = office.objects.iter_mut().find(|o| o.id == prop.id) {
            let p = [
                prop.transform[3][0],
                prop.transform[3][1],
                prop.transform[3][2],
            ];
            moved |= !carried && Vec3::from(o.position).distance(Vec3::from(p)) > 0.03;
            o.position = p;
            o.rotation_xyzw = Some(
                Transform::from_matrix(Mat4::from_cols_array_2d(&prop.transform))
                    .rotation
                    .to_array(),
            );
            o.yaw_deg = Transform::from_matrix(Mat4::from_cols_array_2d(&prop.transform))
                .rotation
                .to_euler(EulerRot::YXZ)
                .0
                .to_degrees();
        }
    }
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
