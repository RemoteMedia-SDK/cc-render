//! Opt-in cloth mesh simulation. Reads committed motion time, never avatar poses.
use crate::cc_render::{
    cloth_solver::{ClothSolver, FixedClothCollider},
    office::controller::OfficeController,
    scene_cloth::{ClothSnapshot, SceneCloth},
    scene_package::ScenePackage,
};
use bevy::{
    math::DVec3,
    prelude::*,
    render::{
        mesh::{Indices, MeshAabb, VertexAttributeValues},
        primitives::Aabb,
        render_asset::RenderAssetUsages,
        render_resource::PrimitiveTopology,
    },
};
use bevy_rapier3d::prelude::RapierConfiguration;
use std::collections::HashSet;

#[derive(Clone, Default, PartialEq, Eq)]
struct ClothClock {
    session: Option<String>,
    origin: Option<u64>,
    displayed: Option<u64>,
    ticks: u64,
}

impl ClothClock {
    fn observe(&mut self, session: &str, pts: u64) -> Result<u64, String> {
        if self.session.as_deref().is_some_and(|s| s != session) {
            return Err("cloth session changed; reload required".into());
        }
        if self.displayed.is_some_and(|p| pts < p) {
            return Err("cloth motion clock moved backwards".into());
        }
        let origin = self.origin.unwrap_or(pts);
        let target = (pts - origin)
            .checked_mul(60)
            .ok_or("cloth clock overflow")?
            / 1000;
        if target.saturating_sub(self.ticks) > 120 {
            return Err("cloth clock advanced more than two seconds in one update".into());
        }
        self.session = Some(session.into());
        self.origin = Some(origin);
        self.displayed = Some(pts);
        Ok(target)
    }
}

#[derive(Component)]
struct LiveCloth {
    id: String,
    solver: ClothSolver,
    colliders: Vec<FixedClothCollider>,
    clock: ClothClock,
    error: Option<String>,
}

#[derive(Resource, Default)]
pub(crate) struct ClothObservations(pub Vec<ClothSnapshot>);

#[derive(Resource, Default)]
pub(super) struct ClothReadiness {
    hidden: HashSet<String>,
    count: usize,
}

impl ClothReadiness {
    pub(super) fn ready(&self, package: &ScenePackage) -> bool {
        self.count == package.cloths.len()
            && package
                .cloths
                .iter()
                .flat_map(|c| &c.replaces_nodes)
                .all(|n| self.hidden.contains(n))
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<ClothReadiness>()
        .init_resource::<ClothObservations>()
        .init_resource::<simulation::ClothSimulation>()
        .add_systems(Startup, spawn)
        .add_systems(Update, hide_static_bedding)
        .add_systems(
            PostUpdate,
            advance
                .after(super::pose::apply_skeletal_pose)
                .before(super::physics::props::publish_scene_state),
        )
        .add_systems(
            PostUpdate,
            contact::observe
                .after(advance)
                .after(bevy::transform::TransformSystem::TransformPropagate)
                .before(super::physics::props::publish_scene_state),
        );
}

fn mesh_for(cloth: &SceneCloth) -> Mesh {
    let asset = cloth.loaded.as_ref().expect("validated cloth asset");
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, asset.initial_positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, asset.material_uv.clone());
    mesh.insert_indices(Indices::U32(
        asset.triangles.iter().flatten().copied().collect(),
    ));
    mesh.compute_smooth_normals();
    mesh
}

fn spawn(
    mut commands: Commands,
    package: Res<ScenePackage>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ready: ResMut<ClothReadiness>,
) {
    for cloth in &package.cloths {
        let mesh = mesh_for(cloth);
        let bounds = mesh.compute_aabb().expect("validated cloth bounds");
        commands.spawn((
            Name::new(cloth.id.clone()),
            Mesh3d(meshes.add(mesh)),
            bounds,
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb_from_array(cloth.color_srgb),
                perceptual_roughness: 0.92,
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
            Transform::IDENTITY,
            Visibility::default(),
            LiveCloth {
                id: cloth.id.clone(),
                solver: cloth.solver().expect("validated cloth"),
                colliders: cloth
                    .colliders(&package)
                    .expect("validated fixed cloth colliders"),
                clock: ClothClock::default(),
                error: None,
            },
        ));
        ready.count += 1;
    }
}

fn hide_static_bedding(
    mut commands: Commands,
    package: Res<ScenePackage>,
    added: Query<(Entity, &Name), Added<Name>>,
    parents: Query<&Parent>,
    roots: Query<Entity, With<super::camera::EnvironmentSceneRoot>>,
    mut ready: ResMut<ClothReadiness>,
) {
    let roots: Vec<_> = roots.iter().collect();
    for (entity, name) in &added {
        if package
            .cloths
            .iter()
            .flat_map(|c| &c.replaces_nodes)
            .any(|n| n == name.as_str())
            && super::camera::is_descendant_of_any(entity, &parents, &roots)
        {
            commands.entity(entity).insert(Visibility::Hidden);
            ready.hidden.insert(name.to_string());
        }
    }
}

impl LiveCloth {
    fn snapshot(&self, mesh: Option<&Mesh>, paused: bool) -> ClothSnapshot {
        let positions = self.solver.surface().positions();
        let values: Vec<_> = positions.iter().map(|p| p.to_array()).collect();
        let mesh_values = match mesh.and_then(|m| m.attribute(Mesh::ATTRIBUTE_POSITION)) {
            Some(VertexAttributeValues::Float32x3(v)) => v.as_slice(),
            _ => &[],
        };
        ClothSnapshot {
            id: self.id.clone(),
            error: self.error.clone(),
            status: if self.error.is_some() {
                "failed"
            } else if self.clock.origin.is_none() {
                "waiting_for_motion"
            } else if paused {
                "paused"
            } else {
                "running"
            }
            .into(),
            session_id: self.clock.session.clone(),
            clock_origin_pts_ms: self.clock.origin,
            displayed_pts_ms: self.clock.displayed,
            simulated_ticks: self.clock.ticks,
            surface_revision: self.solver.surface().revision(),
            max_speed_m_s: self.solver.max_speed(),
            max_edge_stretch_ratio: self.solver.max_edge_stretch_ratio(),
            max_particle_penetration_m: self.solver.max_particle_penetration(&self.colliders),
            surface_position_sha256: position_digest(&values),
            mesh_position_sha256: position_digest(mesh_values),
            bounds_min: positions
                .iter()
                .copied()
                .fold(Vec3::splat(f32::INFINITY), Vec3::min)
                .to_array(),
            bounds_max: positions
                .iter()
                .copied()
                .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max)
                .to_array(),
            particles: positions.len(),
            hand_contacts: Vec::new(),
        }
    }
}

#[path = "cloth_contact.rs"]
mod contact;

fn position_digest(positions: &[[f32; 3]]) -> String {
    let bytes: Vec<_> = positions
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    hmac_sha256::Hash::hash(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[path = "cloth_simulation.rs"]
mod simulation;

fn advance(
    mut cloths: Query<(&mut LiveCloth, &Mesh3d, &mut Aabb)>,
    mut meshes: ResMut<Assets<Mesh>>,
    office: Option<Res<OfficeController>>,
    config: Query<&RapierConfiguration>,
    mut observations: ResMut<ClothObservations>,
    mut simulation: ResMut<simulation::ClothSimulation>,
) {
    observations.0.clear();
    let mut entries: Vec<_> = cloths.iter_mut().collect();
    entries.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    let before: Vec<_> = entries
        .iter()
        .map(|e| (e.0.clock.ticks, e.0.solver.surface().revision()))
        .collect();
    let missing_mesh = entries.iter().any(|e| meshes.get(&e.1 .0).is_none());
    let mut layers: Vec<_> = entries.iter_mut().map(|e| &mut *e.0).collect();
    let result = if missing_mesh {
        Err("cloth render mesh missing".into())
    } else {
        simulation.advance(&mut layers, office.as_deref(), config.get_single().ok())
    };
    if let Err(error) = result {
        for cloth in layers {
            cloth.error = Some(error.clone());
        }
    }
    for ((cloth, handle, mut bounds), (ticks, revision)) in entries.into_iter().zip(before) {
        let paused = cloth.clock.ticks == ticks;
        // Simulated time advanced but the settled batch published nothing.
        let resting = !paused && cloth.solver.surface().revision() == revision;
        let mesh = meshes.get_mut(&handle.0);
        if let Some(mesh) = mesh {
            // A resting surface keeps its revision; its mesh is already current.
            if cloth.solver.surface().revision() != revision {
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_POSITION,
                    cloth
                        .solver
                        .surface()
                        .positions()
                        .iter()
                        .map(|p| p.to_array())
                        .collect::<Vec<_>>(),
                );
                mesh.compute_smooth_normals();
                if let Some(current) = mesh.compute_aabb() {
                    *bounds = current;
                }
            }
            let mut snapshot = cloth.snapshot(Some(mesh), paused);
            if resting && snapshot.status == "running" {
                snapshot.status = "resting".into();
            }
            observations.0.push(snapshot);
        } else {
            observations.0.push(cloth.snapshot(None, true));
        }
    }
}

#[cfg(test)]
#[path = "cloth_tests.rs"]
mod tests;
