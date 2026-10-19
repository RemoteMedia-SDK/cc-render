//! Passive package props share the normal contact-grip and observation path.
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use crate::cc_render::{scene_package::ScenePackage, scene_props::SceneProp};
use super::physics::props::{Prop, PropRegistry};

pub(super) fn install(app: &mut App) { app.add_systems(Startup, spawn); }

fn spawn(mut commands: Commands, package: Res<ScenePackage>, assets: Res<AssetServer>,
    registry: Option<ResMut<PropRegistry>>) {
    if package.props.is_empty() { return; }
    let mut registry = registry.expect("package props require physics");
    for prop in &package.props {
        let entity = spawn_body(&mut commands, prop);
        commands.spawn((SceneRoot(assets.load(format!("scene://{}#Scene0", prop.geometry))),
            Transform::default())).set_parent(entity);
        if prop.object.dynamic { registry.by_id.insert(prop.object.id.clone(), entity); }
    }
}

fn spawn_body(commands: &mut Commands, prop: &SceneProp) -> Entity {
    let o = &prop.object;
    let shape = |half: [f32; 3], cylinder| if cylinder {
        Collider::cylinder(half[1], half[0])
    } else { Collider::cuboid(half[0], half[1], half[2]) };
    let collider = if o.parts.is_empty() { shape(o.half_extents, false) } else {
        Collider::compound(o.parts.iter().map(|p|
            (Vec3::from(p.position), Quat::IDENTITY, shape(p.half_extents, p.cylinder))).collect())
    };
    let mut entity = commands.spawn((Name::new(o.id.clone()), o.transform(), Visibility::default(),
        collider, Friction::coefficient(0.6), Restitution::coefficient(0.),
        if o.dynamic { RigidBody::Dynamic } else { RigidBody::Fixed }));
    if let Some(mass) = prop.mass_kg {
        entity.insert((ColliderMassProperties::Mass(mass), Velocity::default(), Ccd::enabled(),
            Damping { linear_damping:0.1, angular_damping:0.1 },
            CollisionGroups::new(Group::GROUP_1, Group::ALL & !Group::GROUP_32),
            Prop { id:o.id.clone(), loading:false,
                grasp_local:o.local_keypoint("grasp")
                    .or_else(|| o.grasp.map(|p| Transform::from_translation(p.into())))
                    .map(|t| t.compute_matrix().to_cols_array_2d()) }));
    }
    entity.id()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package() -> ScenePackage {
        ScenePackage::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo/scene-shower-caddy.json")).unwrap()
    }
    #[test]
    fn caddy_package_rejects_invalid_mass_identity_asset_and_liquid_owner() {
        let p = package();
        assert_eq!(p.props.iter().filter(|p| p.object.dynamic).count(),3);
        for mut bad in [p.clone(),p.clone(),p.clone(),p.clone()] .into_iter().enumerate() {
            match bad.0 {
                0 => bad.1.props[1].mass_kg = None,
                1 => bad.1.props[1].object.id = bad.1.props[0].object.id.clone(),
                2 => bad.1.props[1].geometry = "../outside.glb".into(),
                _ => bad.1.water_fixtures.last_mut().unwrap().container.as_mut().unwrap().prop = "missing".into(),
            }
            assert!(bad.1.validate().is_err());
        }
        let mut tampered = p.props[1].clone();
        tampered.sha256 = "0".repeat(64);
        assert!(tampered.verify_asset(p.geometry.parent().unwrap()).is_err());
    }
    #[test]
    fn caddy_bottles_settle_on_actual_shelf_and_remain_free_bodies() {
        let p = package();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<SceneSpawner>();
        super::super::physics::install_backend(&mut app);
        app.insert_resource(TimestepMode::Interpolated { dt:1./60., time_scale:1., substeps:4 });
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(1./60.)));
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue,app.world());
        let entities:Vec<_> = p.props.iter().map(|p| spawn_body(&mut commands,p)).collect();
        queue.apply(app.world_mut());
        // Diagnostic free fall from 4cm above the shelf, not avatar animation.
        for e in &entities[1..] { app.world_mut().get_mut::<Transform>(*e).unwrap().translation.y += 0.04; }
        for _ in 0..600 { app.update(); }
        for (prop,e) in p.props.iter().zip(&entities).skip(1) {
            let t = app.world().get::<Transform>(*e).unwrap();
            assert!(t.translation.distance(Vec3::from(prop.object.position)) < 0.006, "{} {:?}",prop.object.id,t.translation);
            assert!(t.rotation.angle_between(Quat::IDENTITY) < 0.03);
            assert!(app.world().get::<Velocity>(*e).unwrap().linvel.length() < 0.01);
            assert!(app.world().get::<ImpulseJoint>(*e).is_none());
            assert_eq!(*app.world().get::<RigidBody>(*e).unwrap(),RigidBody::Dynamic);
        }
    }
}
