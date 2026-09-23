//! Normalize the influences retained by Bevy's four-weight glTF loader.
use super::{camera::AvatarRoot, pose::is_under_avatar};
use bevy::{
    prelude::*,
    render::mesh::{skinning::SkinnedMesh, VertexAttributeValues},
};

#[derive(Component)]
pub(super) struct NormalizedSkinWeights;

/// CC exports may include WEIGHTS_1, which Bevy 0.15 discards. Leaving
/// WEIGHTS_0 with a sum below one scales world positions toward the origin,
/// stretching clothing farther as the avatar walks away from its spawn point.
pub(super) fn normalize_avatar_skin_weights(
    mut commands: Commands,
    skins: Query<(Entity, &Mesh3d), (With<SkinnedMesh>, Without<NormalizedSkinWeights>)>,
    parents: Query<&Parent>,
    roots: Query<Entity, With<AvatarRoot>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let avatars: Vec<_> = roots.iter().collect();
    for (entity, handle) in &skins {
        if !is_under_avatar(entity, &parents, &avatars) {
            continue;
        }
        let Some(mesh) = meshes.get_mut(&handle.0) else {
            continue;
        };
        if let Some(VertexAttributeValues::Float32x4(weights)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_JOINT_WEIGHT)
        {
            normalize_weights(weights);
        }
        commands.entity(entity).insert(NormalizedSkinWeights);
    }
}

fn normalize_weights(weights: &mut [[f32; 4]]) {
    for weight in weights {
        let sum: f32 = weight.iter().sum();
        if sum.is_finite() && sum > 0.0 {
            for component in weight {
                *component /= sum;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_weights_preserve_rigid_translation_away_from_origin() {
        let mut weights = [[0.4, 0.25, 0.15, 0.05], [1.0, 0.0, 0.0, 0.0]];
        normalize_weights(&mut weights);
        let vertex = Vec3::new(0.1, 1.4, 0.2);
        let movement = Vec3::new(4.0, 0.0, -3.0);
        for row in weights {
            let skinned: Vec3 = row.iter().map(|weight| (vertex + movement) * *weight).sum();
            assert!(skinned.abs_diff_eq(vertex + movement, 1e-6));
        }
        assert!((weights[0][0] / weights[0][1] - 1.6).abs() < 1e-6);
    }

    #[test]
    fn normalization_waits_for_assets_and_only_changes_avatar_skins() {
        use bevy::{asset::RenderAssetUsages, render::render_resource::PrimitiveTopology};
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>();
        app.add_systems(Update, normalize_avatar_skin_weights);
        let handle = app.world().resource::<Assets<Mesh>>().reserve_handle();
        let avatar = app.world_mut().spawn(AvatarRoot).id();
        let skin = app
            .world_mut()
            .spawn((
                Mesh3d(handle.clone()),
                SkinnedMesh {
                    inverse_bindposes: default(),
                    joints: vec![],
                },
            ))
            .id();
        app.world_mut().entity_mut(avatar).add_child(skin);
        let prop = app.world_mut().spawn(Mesh3d(handle.clone())).id();
        app.update();
        assert!(app.world().get::<NormalizedSkinWeights>(skin).is_none());
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[0.4, 0.2, 0.2, 0.0]]);
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .insert(handle.id(), mesh);
        app.update();
        assert!(app.world().get::<NormalizedSkinWeights>(skin).is_some());
        assert!(app.world().get::<NormalizedSkinWeights>(prop).is_none());
        let assets = app.world().resource::<Assets<Mesh>>();
        let Some(VertexAttributeValues::Float32x4(weights)) = assets
            .get(&handle)
            .unwrap()
            .attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
        else {
            panic!("missing weights")
        };
        assert_eq!(weights[0], [0.5, 0.25, 0.25, 0.0]);
    }
}
