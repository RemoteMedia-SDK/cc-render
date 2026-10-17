//! Passive furniture bodies and render bindings; no task-driven actuators.
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use std::collections::HashSet;
use crate::cc_render::{scene_mechanism::{JointKind, MechanismPart, SceneMechanism}, scene_package::ScenePackage};
use super::physics::props::{Prop, PropRegistry};

#[derive(Component)]
struct MechanismBody;

#[derive(Resource, Default)]
pub(super) struct MechanismReadiness {
    hidden_nodes: HashSet<String>,
    bodies: usize,
    joints: usize,
}

impl MechanismReadiness {
    pub(super) fn ready(&self, package: &ScenePackage) -> bool {
        self.bodies == package.mechanisms.iter().map(|m| m.parts.len()).sum::<usize>()
            && self.joints == package.mechanisms.iter().flat_map(|m| &m.parts).filter(|p| p.joint.is_some()).count()
            && package.mechanisms.iter().flat_map(|m| &m.replaces_nodes).all(|n| self.hidden_nodes.contains(n))
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<MechanismReadiness>()
        .add_systems(Startup, spawn)
        .add_systems(Update, (hide_replaced_nodes, observe_registration));
}

fn spawn(
    mut commands: Commands,
    package: Res<ScenePackage>,
    assets: Res<AssetServer>,
    registry: Option<ResMut<PropRegistry>>,
) {
    if package.mechanisms.is_empty() { return; }
    let mut registry = registry.expect("articulated packages require physics");
    for mechanism in &package.mechanisms {
        let entities = spawn_bodies(&mut commands, mechanism);
        for (part, entity) in mechanism.parts.iter().zip(entities) {
            let scene = assets.load(format!("scene://{}#Scene0", part.geometry));
            commands.spawn((SceneRoot(scene), Transform::from_translation(-part.bounds().0)))
                .set_parent(entity);
            if part.mass_kg.is_some() { registry.by_id.insert(part.id.clone(), entity); }
        }
        tracing::info!(target: "cc_render", mechanism=%mechanism.id,
            parts=mechanism.parts.len(), "passive mechanism bodies spawned");
    }
}

fn spawn_bodies(commands: &mut Commands, mechanism: &SceneMechanism) -> Vec<Entity> {
    let entities: Vec<_> = mechanism.parts.iter().map(|part| spawn_part(commands, mechanism, part)).collect();
    for (part, entity) in mechanism.parts.iter().zip(&entities) {
        let Some(joint) = &part.joint else { continue; };
        let parent = mechanism.parts.iter().position(|p| p.id == joint.parent).expect("validated parent");
        commands.entity(*entity).insert(ImpulseJoint::new(
            entities[parent], passive_joint(part, &mechanism.parts[parent])));
    }
    entities
}

fn passive_joint(part: &MechanismPart, parent: &MechanismPart) -> TypedJoint {
    let joint = part.joint.as_ref().expect("dynamic mechanism joint");
    let axis = Vec3::from(joint.axis);
    let limits = joint.limits().expect("validated joint limits");
    let mut data: TypedJoint = match joint.kind {
        JointKind::Revolute => RevoluteJointBuilder::new(axis).limits(limits).build().into(),
        JointKind::Prismatic => PrismaticJointBuilder::new(axis).limits(limits).build().into(),
    };
    data.as_mut().set_local_anchor1(Vec3::from(joint.parent_anchor) - parent.bounds().0);
    data.as_mut().set_local_anchor2(Vec3::from(joint.child_anchor) - part.bounds().0);
    data.as_mut().set_contacts_enabled(true);
    data
}

fn spawn_part(commands: &mut Commands, mechanism: &SceneMechanism, part: &MechanismPart) -> Entity {
    let center = part.bounds().0;
    let shapes = part.colliders.iter().map(|shape| {
        let [x, y, z] = shape.half_extents;
        (Vec3::from(shape.position) - center, Quat::IDENTITY, Collider::cuboid(x, y, z))
    }).collect();
    let mut entity = commands.spawn((
        Name::new(part.id.clone()), MechanismBody,
        Transform::from_translation(Vec3::from(mechanism.placement) + Vec3::from(part.position) + center),
        Visibility::default(), Collider::compound(shapes), Friction::coefficient(0.6),
        Prop { id: part.id.clone(), loading: false,
            grasp_local: part.handle_region.as_ref().map(|h|
                Mat4::from_translation(Vec3::from(h.centre) - center).to_cols_array_2d()) },
    ));
    if let Some(mass) = part.mass_kg {
        entity.insert((RigidBody::Dynamic, ColliderMassProperties::Mass(mass), Velocity::default(),
            Damping { linear_damping: 0.1, angular_damping: 0.1 },
            CollisionGroups::new(Group::GROUP_1, Group::ALL & !Group::GROUP_32)));
    } else { entity.insert(RigidBody::Fixed); }
    entity.id()
}

fn hide_replaced_nodes(
    mut commands: Commands,
    package: Res<ScenePackage>,
    added: Query<(Entity, &Name), Added<Name>>,
    parents: Query<&Parent>,
    roots: Query<Entity, With<super::camera::EnvironmentSceneRoot>>,
    mut readiness: ResMut<MechanismReadiness>,
) {
    let roots: Vec<_> = roots.iter().collect();
    for (entity, name) in &added {
        if !package.mechanisms.iter().flat_map(|m| &m.replaces_nodes).any(|n| n == name.as_str())
            || !super::camera::is_descendant_of_any(entity, &parents, &roots) { continue; }
        commands.entity(entity).insert(Visibility::Hidden);
        readiness.hidden_nodes.insert(name.to_string());
        tracing::info!(target: "cc_render", node=%name, "replaced static mechanism node hidden");
    }
}

fn observe_registration(
    bodies: Query<&RapierRigidBodyHandle, With<MechanismBody>>,
    joints: Query<&RapierImpulseJointHandle, With<MechanismBody>>,
    mut readiness: ResMut<MechanismReadiness>,
) {
    readiness.bodies = bodies.iter().count();
    readiness.joints = joints.iter().count();
}

#[cfg(test)]
#[path = "mechanisms_tests.rs"]
mod tests;
