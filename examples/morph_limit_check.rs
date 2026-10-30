//! GPU check for the patched morph target limit (vendor/README.md): a quad with
//! 128 morph targets where only the last one moves it, rendered with that
//! target's weight at `w`, saved to `out.png`.
//!
//! Usage: morph_limit_check out.png w
//!
//! At w = 0 the red quad covers the left half of the frame; at w = 1 target 127
//! has moved it to the right half (0.5: the middle). With upstream Bevy (64)
//! the patch check at startup fails.
use bevy::asset::RenderAssetUsages;
use bevy::render::mesh::morph::{MeshMorphWeights, MorphAttributes, MorphTargetImage, MAX_MORPH_WEIGHTS};
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::prelude::*;

const TARGETS: usize = 128;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 3, "morph_limit_check out.png w");
    assert_eq!(MAX_MORPH_WEIGHTS, TARGETS, "the patched bevy_mesh is not in use");
    let out = args[1].clone();
    let w: f32 = args[2].parse().unwrap();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: bevy::window::WindowResolution::new(256.0, 128.0).with_scale_factor_override(1.0),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Shot { out, w, frames: 0 })
        .add_systems(Startup, setup)
        .add_systems(Update, shoot)
        .run();
}

#[derive(Resource)]
struct Shot {
    out: String,
    w: f32,
    frames: u32,
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut images: ResMut<Assets<Image>>, mut materials: ResMut<Assets<StandardMaterial>>, shot: Res<Shot>) {
    commands.spawn((
        Camera3d::default(),
        Camera { clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        Projection::Orthographic(OrthographicProjection { scaling_mode: bevy::render::camera::ScalingMode::Fixed { width: 2.0, height: 1.0 }, ..OrthographicProjection::default_3d() }),
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // quad over x in [-1, 0], y in [-1, 1]
    let pos = vec![[-1.0, -1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [-1.0, 1.0, 0.0]];
    let targets = (0..TARGETS).map(|t| {
        let dx = if t == TARGETS - 1 { 1.0 } else { 0.0 };
        (0..4).map(move |_| MorphAttributes { position: Vec3::new(dx, 0.0, 0.0), normal: Vec3::ZERO, tangent: Vec3::ZERO })
    });
    let image = MorphTargetImage::new(targets, 4, RenderAssetUsages::RENDER_WORLD).expect("128 targets");
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
        .with_inserted_indices(Indices::U16(vec![0, 1, 2, 0, 2, 3]));
    let material = MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgb(1.0, 0.0, 0.0), unlit: true, ..default() }));
    mesh.set_morph_targets(images.add(image.0));
    let mut weights = vec![0.0; TARGETS];
    weights[TARGETS - 1] = shot.w;
    commands.spawn((Mesh3d(meshes.add(mesh)), material, MeshMorphWeights::new(weights).unwrap()));
}

fn shoot(mut commands: Commands, mut shot: ResMut<Shot>, mut exit: EventWriter<AppExit>) {
    shot.frames += 1;
    // pipelines compile in the background: the quad is drawn after a few frames
    if shot.frames == 300 {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(shot.out.clone()));
    }
    if shot.frames == 330 {
        exit.send(AppExit::Success);
    }
}
