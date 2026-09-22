//! Skinned vertices move in joint space while the imported mesh AABB stays at rest.
use super::{camera::AvatarRoot, pose::is_under_avatar};
use bevy::{
    prelude::*,
    render::{mesh::skinning::SkinnedMesh, view::NoFrustumCulling},
};

/// Bevy 0.15 does not update mesh bounds for skeletal deformation. Opt only
/// avatar skins out of static frustum culling, including shadow culling, so a
/// following camera does not lose the avatar when its rest bounds leave view.
pub(super) fn keep_avatar_skins_visible(
    mut commands: Commands,
    skins: Query<Entity, (With<SkinnedMesh>, Without<NoFrustumCulling>)>,
    parents: Query<&Parent>,
    roots: Query<Entity, With<AvatarRoot>>,
) {
    let avatars: Vec<_> = roots.iter().collect();
    if avatars.is_empty() {
        return;
    }
    for entity in &skins {
        if is_under_avatar(entity, &parents, &avatars) {
            commands.entity(entity).insert(NoFrustumCulling);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_avatar_skins_bypass_static_bounds() {
        let mut app = App::new();
        app.add_systems(Update, keep_avatar_skins_visible);
        let skin = || SkinnedMesh {
            inverse_bindposes: Default::default(),
            joints: vec![],
        };
        let environment = app.world_mut().spawn(skin()).id();
        app.update();
        assert!(app.world().get::<NoFrustumCulling>(environment).is_none());
        let avatar = app.world_mut().spawn(AvatarRoot).id();
        let body = app.world_mut().spawn(skin()).id();
        let prop = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(avatar)
            .add_children(&[body, prop]);
        app.update();
        assert!(app.world().get::<NoFrustumCulling>(body).is_some());
        assert!(app.world().get::<NoFrustumCulling>(environment).is_none());
        assert!(app.world().get::<NoFrustumCulling>(prop).is_none());
    }
}
