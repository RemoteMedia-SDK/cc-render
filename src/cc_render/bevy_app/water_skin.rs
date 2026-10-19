//! Observe the displayed avatar after skin-weight normalization, morph inheritance,
//! transform propagation and optional tissue deformation. Never writes body data.
use super::{
    camera::AvatarRoot,
    pose::is_under_avatar,
    skinning::NormalizedSkinWeights,
    tissue_morph::MorphGeometry,
    water_collision::{bounds, WaterSurfaces},
};
use bevy::{
    math::DVec3,
    prelude::*,
    render::mesh::{
        morph::MeshMorphWeights,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
        VertexAttributeValues,
    },
};
use std::collections::HashMap;

#[derive(Default, Resource)]
pub(super) struct SkinCache(HashMap<Entity, CachedSkin>);
#[derive(Default, Resource)]
pub(super) struct WaterActivity(pub bool);
struct CachedSkin {
    mesh: AssetId<Mesh>,
    morphs: Option<MorphGeometry>,
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct SkinInputs<'w, 's> {
    skins: Query<
        'w,
        's,
        (
            Entity,
            &'static Mesh3d,
            &'static SkinnedMesh,
            Option<&'static MeshMorphWeights>,
        ),
        With<NormalizedSkinWeights>,
    >,
    globals: Query<'w, 's, &'static GlobalTransform>,
    parents: Query<'w, 's, &'static Parent>,
    roots: Query<'w, 's, Entity, With<AvatarRoot>>,
    meshes: Res<'w, Assets<Mesh>>,
    images: Res<'w, Assets<Image>>,
    inverse: Res<'w, Assets<SkinnedMeshInverseBindposes>>,
    valves: Res<'w, crate::cc_render::scene_water::WaterBridge>,
    activity: Res<'w, WaterActivity>,
}

pub(super) fn observe_skin(
    input: SkinInputs,
    mut cache: ResMut<SkinCache>,
    package: Res<crate::cc_render::scene_package::ScenePackage>,
    mut surfaces: ResMut<WaterSurfaces>,
) {
    if package.water_fixtures.is_empty() {
        return;
    }
    if !input.activity.0
        // A tilted container can begin emitting without a valve command. Observe
        // current skin before that first parcel, including while taps are off.
        && !package.water_fixtures.iter().any(|f| f.container.is_some())
        && input
            .valves
            .requests
            .borrow()
            .values()
            .all(|s| s.flow == 0.)
        && input.fully_cached(&cache)
    {
        surfaces.history_valid = false;
        surfaces.update_ms = 0.;
        return;
    }
    let started = std::time::Instant::now();
    let result = (|| -> anyhow::Result<()> {
        let (positions, triangles, count) = input.geometry(&mut cache)?;
        // Keep runoff contact valid as the avatar walks away from an outlet.
        // Every displayed body triangle belongs to this observed world bound.
        let regions: Vec<_> = (!positions.is_empty())
            .then(|| bounds(positions.iter().copied(), super::water_collision::RADIUS))
            .into_iter()
            .collect();
        surfaces.update(positions, triangles, &regions)?;
        surfaces.meshes = count;
        Ok(())
    })();
    if let Err(error) = result {
        let message = error.to_string();
        if surfaces.error.as_ref() != Some(&message) {
            tracing::warn!(%message,"water body collision unavailable; emission paused");
        }
        surfaces.error = Some(message);
    }
    surfaces.update_ms = started.elapsed().as_secs_f64() * 1000.;
}

impl SkinInputs<'_, '_> {
    fn fully_cached(&self, cache: &SkinCache) -> bool {
        let roots: Vec<_> = self.roots.iter().collect();
        self.skins.iter().all(|(entity, handle, ..)| {
            !is_under_avatar(entity, &self.parents, &roots)
                || cache.0.get(&entity).is_some_and(|c| c.mesh == handle.id())
        })
    }
    fn palette(&self, skin: &SkinnedMesh) -> anyhow::Result<Vec<Mat4>> {
        let inverse = self
            .inverse
            .get(&skin.inverse_bindposes)
            .ok_or_else(|| anyhow::anyhow!("water inverse bind poses missing"))?;
        let palette = skin
            .joints
            .iter()
            .zip(inverse.iter())
            .map(|(joint, inv)| {
                self.globals
                    .get(*joint)
                    .map(|g| g.compute_matrix() * *inv)
                    .map_err(|_| anyhow::anyhow!("water skin joint missing"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(palette)
    }
    fn geometry(
        &self,
        cache: &mut SkinCache,
    ) -> anyhow::Result<(Vec<DVec3>, Vec<[usize; 3]>, usize)> {
        let roots: Vec<_> = self.roots.iter().collect();
        let mut items: Vec<_> = self
            .skins
            .iter()
            .filter(|(entity, ..)| is_under_avatar(*entity, &self.parents, &roots))
            .collect();
        items.sort_by_key(|(entity, ..)| entity.to_bits());
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        for (entity, handle, skin, morph) in &items {
            let mesh = self
                .meshes
                .get(&handle.0)
                .ok_or_else(|| anyhow::anyhow!("water skin mesh not loaded"))?;
            if !cache.0.get(entity).is_some_and(|c| c.mesh == handle.id()) {
                cache.0.insert(
                    *entity,
                    CachedSkin {
                        mesh: handle.id(),
                        morphs: MorphGeometry::capture(mesh, &self.images)?,
                    },
                );
            }
            let palette = self.palette(skin)?;
            let vertices = skin_vertices(
                mesh,
                &palette,
                cache.0[entity].morphs.as_ref(),
                morph.map(|w| w.weights()).unwrap_or(&[]),
            )?;
            let offset = positions.len();
            let indices: Vec<_> = mesh
                .indices()
                .map(|i| i.iter().collect())
                .unwrap_or_else(|| (0..vertices.len()).collect());
            anyhow::ensure!(
                indices.len() % 3 == 0 && indices.iter().all(|i| *i < vertices.len()),
                "invalid water skin topology"
            );
            triangles.extend(
                indices
                    .chunks_exact(3)
                    .map(|t| [t[0] + offset, t[1] + offset, t[2] + offset]),
            );
            positions.extend(vertices);
        }
        Ok((positions, triangles, items.len()))
    }
}

fn skin_vertices(
    mesh: &Mesh,
    palette: &[Mat4],
    morphs: Option<&MorphGeometry>,
    weights: &[f32],
) -> anyhow::Result<Vec<DVec3>> {
    let Some(VertexAttributeValues::Float32x3(base)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        anyhow::bail!("water skin positions");
    };
    let Some(VertexAttributeValues::Float32x4(joint_weights)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
    else {
        anyhow::bail!("water skin weights");
    };
    let Some(VertexAttributeValues::Uint16x4(joints)) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
    else {
        anyhow::bail!("water skin indices");
    };
    let base: Vec<_> = base.iter().copied().map(Vec3::from).collect();
    let base = match morphs {
        Some(m) => m.apply(&base, weights)?,
        None => base,
    };
    anyhow::ensure!(
        base.len() == joint_weights.len()
            && base.len() == joints.len()
            && joints
                .iter()
                .flatten()
                .all(|i| (*i as usize) < palette.len()),
        "water skin array/palette bounds"
    );
    Ok(base
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let matrix: Mat4 = (0..4)
                .map(|k| palette[joints[i][k] as usize] * joint_weights[i][k])
                .sum();
            matrix.transform_point3(*p).as_dvec3()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn water_skin_observation_uses_actual_weights_and_never_edits_render_mesh() {
        use bevy::{asset::RenderAssetUsages, render::render_resource::PrimitiveTopology};
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[1., 2., 3.]]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[0.25, 0.75, 0., 0.]]);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(vec![[0, 1, 0, 0]]),
        );
        let p = skin_vertices(
            &mesh,
            &[Mat4::IDENTITY, Mat4::from_translation(Vec3::Y * 4.)],
            None,
            &[],
        )
        .unwrap();
        assert_eq!(p[0], DVec3::new(1., 5., 3.));
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap(),
            &[[1., 2., 3.]]
        );
    }
}
