//! Procedural moving cloud sky, independent of avatar pose and motion.
use bevy::{
    pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster, NotShadowReceiver},
    prelude::*,
    render::{
        mesh::MeshVertexBufferLayoutRef,
        render_resource::{
            AsBindGroup, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
        },
    },
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct CondoSky {
    /// Time, cloud cover, night blend, wind speed.
    #[uniform(0)]
    conditions: Vec4,
}

impl Material for CondoSky {
    fn fragment_shader() -> ShaderRef {
        "scene://sky.wgsl".into()
    }

    fn specialize(
        _: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub(super) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<CondoSky>::default())
        .add_systems(Startup, spawn_sky)
        .add_systems(Update, update_sky);
}

fn update_sky(weather: Res<super::weather_transition::AppliedWeather>, mut materials: ResMut<Assets<CondoSky>>) {
    for (_, material) in materials.iter_mut() {
        material.conditions = weather.current.sky;
    }
}

fn spawn_sky(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CondoSky>>,
    weather: Res<super::weather::CondoWeather>,
) {
    let material = materials.add(CondoSky {
        conditions: weather.sky(),
    });
    commands.spawn((
        Name::new("Moving waterfront sky"),
        Mesh3d(meshes.add(Sphere::new(9000.).mesh().uv(64, 32))),
        MeshMaterial3d(material.clone()),
        Transform::IDENTITY,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}
