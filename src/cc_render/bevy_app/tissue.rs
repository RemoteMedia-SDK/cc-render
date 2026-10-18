//! Volumetric breast adapter. Articulated transforms are read-only here.
use super::{
    camera::AvatarRoot,
    pose::{is_under_avatar, CC5BindRotations},
    skinning::NormalizedSkinWeights,
};
use super::{
    tissue_collision::{BodyContacts, ContactPose, ContactUnresolved},
    tissue_motion::{self, TissueRig},
    tissue_surface::{positions_hash, TissueSurface},
};
use crate::cc_render::{
    office::timeline::PoseTimeline,
    soft_tissue::{Frame, SurfaceBinding, Tissue, TissueAsset, TissueError},
};
use bevy::{
    math::DVec3,
    prelude::*,
    render::mesh::{
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
        VertexAttributeValues,
    },
    transform::TransformSystem,
};
use std::{collections::HashMap, path::Path, sync::Arc};

#[cfg(test)]
#[path = "tissue_tests.rs"]
mod tests;

#[path = "tissue_trace.rs"]
mod trace;

#[derive(Resource)]
pub(super) struct TissueRequest {
    asset: TissueAsset,
    rate: u32,
    snapshot: Option<(std::path::PathBuf, u64)>,
    trace: Option<trace::Trace>,
}

#[derive(Resource, Default)]
pub(super) struct TissueReadiness {
    pub ready: bool,
    pub error: Option<String>,
}

#[derive(Resource, Default)]
struct TissueState {
    active: Option<Active>,
    stable_frames: u32,
}

struct Active {
    rig: TissueRig,
    anchors: Vec<Entity>,
    surfaces: Vec<TissueSurface>,
    contacts: BodyContacts,
    solvers: Vec<Tissue>,
    session: Option<String>,
    origin_ms: f64,
    tick: u64,
    report_ms: u64,
    snapshot_written: bool,
    motion_initialized: bool,
    morph_history: super::tissue_morph::MorphHistory,
}

pub(super) fn install(app: &mut App, glb: &Path) -> anyhow::Result<()> {
    let Some(path) = std::env::var_os("CC_AVATAR_TISSUE_ASSET") else {
        return Ok(());
    };
    let mut asset = TissueAsset::from_json(&std::fs::read(path)?)?;
    let bytes = std::fs::read(glb)?;
    let fingerprint: String = hmac_sha256::Hash::hash(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    anyhow::ensure!(
        asset.source_sha256.contains(&fingerprint),
        "GLB is not a calibrated source of the requested tissue asset"
    );
    let rate = std::env::var("CC_AVATAR_TISSUE_HZ").map_or(Ok(120_u32), |s| s.parse())?;
    let passes = std::env::var("CC_AVATAR_TISSUE_ITERATIONS").map_or(Ok(32_usize), |s| s.parse())?;
    anyhow::ensure!(
        (60..=480).contains(&rate) && (1..=64).contains(&passes),
        "invalid tissue integration settings"
    );
    for region in &mut asset.regions {
        region.material.iterations = passes;
    }
    asset.validate()?;
    let snapshot = std::env::var_os("CC_AVATAR_TISSUE_SNAPSHOT")
        .map(|path| {
            let at = std::env::var("CC_AVATAR_TISSUE_SNAPSHOT_PTS_MS")
                .map_err(|_| anyhow::anyhow!("tissue snapshot requires a source timestamp"))?
                .parse::<u64>()?;
            anyhow::Ok((std::path::PathBuf::from(path), at))
        })
        .transpose()?;
    app.insert_resource(TissueRequest {
        asset,
        rate,
        snapshot,
        trace: trace::Trace::from_env()?,
    });
    app.init_resource::<TissueReadiness>()
        .init_resource::<super::capture::TissueFramePts>()
        .init_resource::<TissueState>();
    app.add_systems(
        PostUpdate,
        (initialize, advance)
            .chain()
            .after(bevy::render::mesh::inherit_weights)
            .after(TransformSystem::TransformPropagate),
    );
    tracing::info!(rate, passes, %fingerprint, "volumetric tissue requested; legacy breast springs excluded");
    Ok(())
}

#[derive(bevy::ecs::system::SystemParam)]
struct TissueScene<'w, 's> {
    roots: Query<'w, 's, (Entity, &'static SceneRoot), With<AvatarRoot>>,
    parents: Query<'w, 's, &'static Parent>,
    bones: Query<'w, 's, (Entity, &'static Name)>,
    locals: Query<'w, 's, (&'static Transform, Option<&'static Parent>)>,
    skins:
        Query<'w, 's, (Entity, &'static Mesh3d, &'static SkinnedMesh), With<NormalizedSkinWeights>>,
    inverse_bind: Res<'w, Assets<SkinnedMeshInverseBindposes>>,
    images: Res<'w, Assets<Image>>,
    twists: Res<'w, super::twist::ForearmRigs>,
    assets: Res<'w, AssetServer>,
}

fn stop(readiness: &mut TissueReadiness, exits: &mut EventWriter<AppExit>, error: anyhow::Error) {
    let message = format!("{error:#}");
    tracing::error!(%message, "volumetric tissue failed; stopping renderer without pose repair or legacy fallback");
    readiness.ready = false;
    readiness.error = Some(message);
    exits.send(AppExit::error());
}

fn initialize(
    mut commands: Commands,
    request: Res<TissueRequest>,
    bind: Res<CC5BindRotations>,
    scene: TissueScene,
    mut meshes: ResMut<Assets<Mesh>>,
    mut state: ResMut<TissueState>,
    mut readiness: ResMut<TissueReadiness>,
    mut exits: EventWriter<AppExit>,
) {
    if state.active.is_some() || readiness.error.is_some() || !bind.captured {
        return;
    }
    let roots: Vec<_> = scene.roots.iter().map(|(e, _)| e).collect();
    if roots.len() != 1
        || scene
            .roots
            .iter()
            .any(|(_, root)| !scene.assets.is_loaded_with_dependencies(root.0.id()))
    {
        return;
    }
    state.stable_frames += 1;
    if state.stable_frames < 8 {
        return;
    }
    match initialize_ready(&mut commands, &request, &bind, &scene, &mut meshes) {
        Ok(active) => {
            tracing::info!(
                surfaces = active.surfaces.len(),
                regions = active.anchors.len(),
                "volumetric tissue surfaces bound"
            );
            state.active = Some(active);
            readiness.ready = true;
        }
        Err(error) => stop(&mut readiness, &mut exits, error),
    }
}

fn initialize_ready(
    commands: &mut Commands,
    request: &TissueRequest,
    bind: &CC5BindRotations,
    scene: &TissueScene,
    meshes: &mut Assets<Mesh>,
) -> anyhow::Result<Active> {
    let roots: Vec<_> = scene.roots.iter().map(|(e, _)| e).collect();
    let mut anchors = Vec::new();
    for region in &request.asset.regions {
        let matches: Vec<_> = scene
            .bones
            .iter()
            .filter(|(e, name)| {
                name.as_str() == region.anchor_bone && is_under_avatar(*e, &scene.parents, &roots)
            })
            .map(|(e, _)| e)
            .collect();
        anyhow::ensure!(
            matches.len() == 1,
            "missing or ambiguous tissue anchor {}",
            region.anchor_bone
        );
        anchors.push(matches[0]);
    }
    let mut surfaces = Vec::new();
    let mut body_found = false;
    let mut contact_found = std::collections::BTreeSet::new();
    for (entity, handle, skin) in &scene.skins {
        if !is_under_avatar(entity, &scene.parents, &roots) {
            continue;
        }
        let mesh = meshes
            .get(&handle.0)
            .ok_or_else(|| anyhow::anyhow!("missing tissue mesh"))?;
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        let hash = positions_hash(positions);
        let binding = request
            .asset
            .surfaces
            .iter()
            .find(|s| s.positions_sha256 == hash);
        let collision_only;
        let binding = match binding {
            Some(binding) => binding,
            None if request.asset.contact_surfaces.contains(&hash) => {
                contact_found.insert(hash.clone());
                collision_only = SurfaceBinding {
                    mesh_name: "kinematic body contact".into(),
                    positions_sha256: hash,
                    vertex_count: positions.len(),
                    vertices: Vec::new(),
                };
                &collision_only
            }
            None => continue,
        };
        body_found |= binding.mesh_name == "CC_Base_Body";
        let inverse = scene
            .inverse_bind
            .get(&skin.inverse_bindposes)
            .ok_or_else(|| anyhow::anyhow!("missing tissue inverse bind asset"))?;
        let mut surface = TissueSurface::capture(mesh, binding, skin, inverse)?;
        surface.capture_morphs(mesh, &scene.images)?;
        surface.entity = Some(entity);
        if surface.has_deformation() {
            surface.mesh = meshes.add(mesh.clone());
            commands.entity(entity).insert(Mesh3d(surface.mesh.clone()));
        } else {
            surface.mesh = handle.0.clone();
        }
        surfaces.push(surface);
    }
    anyhow::ensure!(
        body_found && !surfaces.is_empty(),
        "calibrated body tissue surface missing"
    );
    anyhow::ensure!(
        contact_found.len() == request.asset.contact_surfaces.len(),
        "calibrated contact skin primitive missing"
    );
    let targets = anchors
        .iter()
        .copied()
        .chain(surfaces.iter().flat_map(|s| s.joints.iter().copied()))
        .collect::<Vec<_>>();
    let rig = TissueRig::capture(&targets, &scene.locals, bind, &scene.twists.0)?;
    let contacts = BodyContacts::new(if request.asset.contact_surfaces.is_empty() {
        &[]
    } else {
        &surfaces
    })?;
    tracing::info!(counts=?contacts.counts(), "source-time body contact bound");
    Ok(Active {
        rig,
        contacts,
        anchors,
        surfaces,
        solvers: Vec::new(),
        session: None,
        origin_ms: 0.0,
        tick: 0,
        report_ms: 0,
        snapshot_written: false,
        motion_initialized: false,
        morph_history: Default::default(),
    })
}

impl Active {
    fn pose_at(
        &self,
        knots: &[crate::cc_render::renderer::SkeletalPose],
        at: f64,
    ) -> anyhow::Result<ContactPose> {
        let matrices = self.rig.sample(&tissue_motion::sample_pose(knots, at)?);
        let frames = self
            .anchors
            .iter()
            .map(|e| tissue_motion::frame(matrices[e]))
            .collect::<anyhow::Result<_>>()?;
        let mut skin = Vec::new();
        let morph_weights = self.morph_history.sample(at)?;
        for (surface, weights) in self.surfaces.iter().zip(&morph_weights) {
            skin.extend(surface.kinematic_morphed_positions(&matrices, weights)?);
        }
        Ok(ContactPose { frames, skin })
    }

    fn frames_at(
        &self,
        knots: &[crate::cc_render::renderer::SkeletalPose],
        at: f64,
    ) -> anyhow::Result<Vec<Frame>> {
        let pose = tissue_motion::sample_pose(knots, at)?;
        let matrices = self.rig.sample(&pose);
        self.anchors
            .iter()
            .map(|e| tissue_motion::frame(matrices[e]))
            .collect()
    }

    fn initialize_session(
        &mut self,
        request: &TissueRequest,
        window: &crate::cc_render::office::timeline::MotionWindow,
    ) -> anyhow::Result<()> {
        // Facial contact geometry is observed on presentation. Start the new
        // physical state at that first observation, which need not coincide
        // with the first body knot (e.g. 66 ms display versus 50 ms source).
        // Subsequent steps still retain and sample skipped source knots.
        let start = window.displayed_ms as f64;
        let frames = self.frames_at(&window.knots, start)?;
        self.solvers = request
            .asset
            .regions
            .iter()
            .zip(frames)
            .map(|(r, f)| r.solver(f))
            .collect::<Result<_, _>>()?;
        self.session = Some(window.session.clone());
        self.origin_ms = start;
        self.tick = 0;
        self.report_ms = 0;
        self.motion_initialized = false;
        self.morph_history = Default::default();
        Ok(())
    }

    fn initialize_velocity(
        &mut self,
        request: &TissueRequest,
        window: &crate::cc_render::office::timeline::MotionWindow,
    ) -> anyhow::Result<()> {
        if self.motion_initialized {
            return Ok(());
        }
        let next = (self.origin_ms + 1000.0 / request.rate as f64)
            .min(window.knots.last().unwrap().pts_ms as f64);
        if next > self.origin_ms {
            // A single displayed knot does not yet determine velocity. Wait for
            // its adjacent committed knot, then initialize once before stepping.
            let frames = self.frames_at(&window.knots, next)?;
            for (solver, frame) in self.solvers.iter_mut().zip(frames) {
                solver.initialize_motion(frame, (next - self.origin_ms) / 1000.0)?;
            }
            self.motion_initialized = true;
        }
        Ok(())
    }

    fn simulate(
        &mut self,
        request: &TissueRequest,
        timeline: &PoseTimeline,
        globals: &HashMap<Entity, Mat4>,
        morph_weights: &[Vec<f32>],
        meshes: &mut Assets<Mesh>,
    ) -> anyhow::Result<Option<u64>> {
        anyhow::ensure!(morph_weights.len() == self.surfaces.len(),
            "contact morph surface count differs from the captured topology");
        let dt_ms = 1000.0 / request.rate as f64;
        let at = self
            .session
            .as_ref()
            .map(|_| self.origin_ms + self.tick as f64 * dt_ms);
        let Some(window) = timeline.motion_window(at) else {
            return Ok(None);
        };
        if self.session.as_ref() != Some(&window.session) {
            self.initialize_session(request, &window)?;
        }
        let end = window.displayed_ms as f64;
        self.morph_history.observe(end, morph_weights, self.origin_ms + self.tick as f64 * dt_ms)?;
        let elapsed = end - (self.origin_ms + self.tick as f64 * dt_ms);
        anyhow::ensure!(
            elapsed >= -1e-6 && elapsed <= 1000.0,
            "tissue playback clock discontinuity or excessive backlog"
        );
        // Inputs stay frozen during this invocation. Never retain sampled skin
        // across a new presentation, morph observation, or motion revision.
        let pose_cache = tissue_motion::ContactPoseCache::default();
        let sampled = pose_cache.sample(end, || self.pose_at(&window.knots, end))?;
        let frames = &sampled.frames;
        let mut observed_skin = Vec::new();
        for (surface, weights) in self.surfaces.iter().zip(morph_weights) {
            observed_skin.extend(surface.kinematic_morphed_positions(globals, weights)?);
        }
        let skin_error = sampled
            .skin
            .iter()
            .zip(&observed_skin)
            .map(|(a, b)| a.distance(*b))
            .fold(0.0_f64, f64::max);
        anyhow::ensure!(
            skin_error < 0.0002,
            "tissue contact FK differs from displayed skin by {skin_error} m"
        );
        for (entity, expected) in self.anchors.iter().zip(frames) {
            let observed = tissue_motion::frame(globals[entity])?;
            anyhow::ensure!(
                observed.translation.distance(expected.translation) < 0.0002
                    && observed.rotation.angle_between(expected.rotation) < 0.001,
                "tissue sampled FK does not match displayed skeletal pose"
            );
        }
        self.initialize_velocity(request, &window)?;
        let mut integration = IntegrationReport::default();
        while self.origin_ms + (self.tick + 1) as f64 * dt_ms <= end + 1e-6 {
            let start = self.origin_ms + self.tick as f64 * dt_ms;
            let mut solvers = self.solvers.clone();
            integration.merge(integrate_interval(
                &mut solvers,
                start,
                start + dt_ms,
                0,
                &|at| pose_cache.sample(at, || self.pose_at(&window.knots, at)),
                &self.contacts,
                &request.asset,
            )?);
            self.solvers = solvers;
            self.tick += 1;
        }
        // Evaluate the fractional remainder on a copy. Rendering never changes
        // the fixed-step state, including when displayed time is paused.
        let mut output = self.solvers.clone();
        let remainder = end - (self.origin_ms + self.tick as f64 * dt_ms);
        if remainder > 1e-6 {
            integration.merge(integrate_interval(
                &mut output,
                end - remainder,
                end,
                0,
                &|at| pose_cache.sample(at, || self.pose_at(&window.knots, at)),
                &self.contacts,
                &request.asset,
            )?);
        }
        (integration.pose_samples, integration.pose_cache_hits) = pose_cache.counts();
        let nodes: Vec<_> = output.iter().map(|s| s.positions().to_vec()).collect();
        let mut maximum = 0.0_f64;
        let mut surface_metrics = Vec::with_capacity(self.surfaces.len());
        for surface in &self.surfaces {
            if !surface.has_deformation() {
                continue;
            }
            let mesh = meshes
                .get_mut(&surface.mesh)
                .ok_or_else(|| anyhow::anyhow!("tissue render mesh disappeared"))?;
            let metrics = surface.deform(mesh, &request.asset, &nodes, frames, globals)?;
            maximum = maximum.max(metrics.maximum_displacement_m);
            surface_metrics.push(metrics);
        }
        if let Some((path, at)) = &request.snapshot {
            if !self.snapshot_written && window.displayed_ms >= *at {
                let surfaces = self
                    .surfaces
                    .iter()
                    .map(|s| s.diagnostic(meshes.get(&s.mesh).unwrap(), globals))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                serde_json::to_writer(
                    file,
                    &serde_json::json!({"source_pts_ms":window.displayed_ms,"surfaces":surfaces}),
                )?;
                self.snapshot_written = true;
                tracing::info!(source_pts_ms=window.displayed_ms, path=%path.display(), "tissue geometry snapshot written");
            }
        }
        if let Some(trace) = &request.trace {
            trace.record(window.displayed_ms, &request.asset, &output, &integration, maximum, skin_error)?;
        }
        if window.displayed_ms >= self.report_ms + 1000 {
            self.report_ms = window.displayed_ms;
            let metrics: Vec<_> = output.iter().map(Tissue::metrics).collect();
            tracing::info!(pts_ms=window.displayed_ms, tick=self.tick, integration=%serde_json::to_string(&integration)?, skin_error_m=skin_error, maximum_displacement_m=maximum,
                surfaces=%serde_json::to_string(&surface_metrics)?, metrics=%serde_json::to_string(&metrics)?, "volumetric tissue frame");
        }
        Ok(Some(window.displayed_ms))
    }
}

#[derive(Default, serde::Serialize)]
struct IntegrationReport {
    // Counts cover the full presentation, including its initial FK check.
    pose_samples: usize,
    pose_cache_hits: usize,
    subdivisions: usize,
    active_contact_samples: usize,
    maximum_penetration_m: f64,
    maximum_normal_force_n: f64,
    sampling_ms: f64,
    elasticity_ms: f64,
    collision_ms: f64,
}

impl IntegrationReport {
    fn merge(&mut self, other: Self) {
        self.pose_samples += other.pose_samples;
        self.pose_cache_hits += other.pose_cache_hits;
        self.subdivisions += other.subdivisions;
        self.active_contact_samples += other.active_contact_samples;
        self.maximum_penetration_m = self.maximum_penetration_m.max(other.maximum_penetration_m);
        self.maximum_normal_force_n = self
            .maximum_normal_force_n
            .max(other.maximum_normal_force_n);
        self.sampling_ms += other.sampling_ms;
        self.elasticity_ms += other.elasticity_ms;
        self.collision_ms += other.collision_ms;
    }
}

/// Retry an invalid physical step at finer intervals, resampling the same
/// committed motion. No output clamp, pose correction or state reset occurs.
/// Operates only on the caller's trial state. Every task joins before contact
/// queries or commit; a failure discards the entire trial, including successful
/// regions. Direct scope spawns return results in region order in Bevy 0.15.
fn step_regions(
    trial: &mut [Tissue],
    frames: &[Frame],
    dt: f64,
    pool: Option<&bevy::tasks::TaskPool>,
) -> Result<(), (usize, TissueError)> {
    debug_assert_eq!(trial.len(), frames.len());
    let Some(pool) = pool.filter(|_| trial.len() > 1) else {
        return trial.iter_mut().zip(frames).enumerate().try_for_each(|(i, (solver, frame))| {
            solver.step(dt, *frame, DVec3::NEG_Y * 9.81).map_err(|error| (i, error))
        });
    };
    let results = pool.scope(|scope| {
        for (solver, frame) in trial.iter_mut().zip(frames) {
            scope.spawn(async move { solver.step(dt, *frame, DVec3::NEG_Y * 9.81) });
        }
    });
    results.into_iter().enumerate().try_for_each(|(i, result)| result.map_err(|error| (i, error)))
}

fn integrate_interval(
    solvers: &mut Vec<Tissue>,
    start_ms: f64,
    end_ms: f64,
    depth: usize,
    pose_at: &impl Fn(f64) -> anyhow::Result<Arc<ContactPose>>,
    contacts: &BodyContacts,
    asset: &TissueAsset,
) -> anyhow::Result<IntegrationReport> {
    let started = std::time::Instant::now();
    let old = pose_at(start_ms)?;
    let end = pose_at(end_ms)?;
    let frames = &end.frames;
    anyhow::ensure!(
        frames.len() == solvers.len(),
        "tissue attachment count changed"
    );
    let mut candidate = solvers.clone();
    let mut failure = None;
    let mut report = IntegrationReport::default();
    report.sampling_ms = started.elapsed().as_secs_f64() * 1000.0;
    let elasticity = std::time::Instant::now();
    if let Err((index, error)) = step_regions(
        &mut candidate,
        frames,
        (end_ms - start_ms) / 1000.0,
        bevy::tasks::ComputeTaskPool::try_get().map(|pool| &**pool),
    ) {
        failure = Some((index, anyhow::Error::from(error)));
    }
    report.elasticity_ms = elasticity.elapsed().as_secs_f64() * 1000.0;
    let collision = std::time::Instant::now();
    if failure.is_none() {
        match contacts.resolve(
            solvers,
            &mut candidate,
            asset,
            &old,
            &end,
            (end_ms - start_ms) / 1000.0,
        ) {
            Ok(metrics) => {
                report.active_contact_samples = metrics.iter().map(|m| m.active_contacts).sum();
                report.maximum_penetration_m = metrics
                    .iter()
                    .map(|m| m.maximum_penetration_m)
                    .fold(0.0, f64::max);
                report.maximum_normal_force_n = metrics.iter().map(|m| m.normal_force_n).sum();
            }
            Err(error) => failure = Some((0, error)),
        }
    }
    report.collision_ms = collision.elapsed().as_secs_f64() * 1000.0;
    if let Some((index, error)) = failure {
        if depth < 6
            && (matches!(
                error.downcast_ref::<TissueError>(),
                Some(TissueError::InvertedTet(_) | TissueError::NonlinearSolveFailed { .. })
            ) || error.is::<ContactUnresolved>())
        {
            let middle = (start_ms + end_ms) * 0.5;
            let mut split = solvers.clone();
            let left = integrate_interval(
                &mut split,
                start_ms,
                middle,
                depth + 1,
                pose_at,
                contacts,
                asset,
            )?;
            let right = integrate_interval(
                &mut split,
                middle,
                end_ms,
                depth + 1,
                pose_at,
                contacts,
                asset,
            )?;
            *solvers = split;
            report.merge(left);
            report.merge(right);
            report.subdivisions += 1;
            return Ok(report);
        }
        anyhow::bail!("region {index}, interval {start_ms}..{end_ms} ms, depth {depth}: {error}; prior {:?}; from {:?} to {:?}",
            solvers[index].metrics(), solvers[index].frame(), frames[index]);
    }
    *solvers = candidate;
    Ok(report)
}

fn advance(
    request: Res<TissueRequest>,
    timeline: Res<PoseTimeline>,
    mut state: ResMut<TissueState>,
    globals: Query<(Entity, &GlobalTransform)>,
    morphs: Query<&bevy::render::mesh::morph::MeshMorphWeights>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut readiness: ResMut<TissueReadiness>,
    mut exits: EventWriter<AppExit>,
    signal: Option<Res<super::ReadySignal>>,
    mut stamp: ResMut<super::capture::TissueFramePts>,
) {
    stamp.0 = None;
    if readiness.error.is_some() {
        return;
    }
    let Some(active) = &mut state.active else {
        return;
    };
    let morph_status = active.surfaces.iter().try_for_each(|surface| {
        if request.asset.contact_inactive_morph_targets.is_empty() {
            return Ok(());
        }
        let weights = morphs
            .get(surface.entity.unwrap())
            .map_err(|_| anyhow::anyhow!("missing contact skin morph weights"))?;
        validate_contact_morphs(
            &request.asset.contact_inactive_morph_targets,
            weights.weights(),
        )
    });
    let current_weights = morph_status.and_then(|_| active.surfaces.iter().map(|surface|
        surface.contact_morph_weights(surface.entity.and_then(|entity| morphs.get(entity).ok()).map(|m| m.weights()))
    ).collect::<anyhow::Result<Vec<_>>>());
    let matrices = globals
        .iter()
        .map(|(e, t)| (e, t.compute_matrix()))
        .collect();
    match current_weights.and_then(|weights| active.simulate(&request, &timeline, &matrices, &weights, &mut meshes)) {
        Ok(pts) => stamp.0 = pts,
        Err(error) => {
            if let Some(signal) = signal {
                signal.0.store(false, std::sync::atomic::Ordering::Release);
            }
            stop(&mut readiness, &mut exits, error);
        }
    }
}

fn validate_contact_morphs(indices: &[usize], weights: &[f32]) -> anyhow::Result<()> {
    anyhow::ensure!(indices.iter().all(|i| weights.get(*i).is_some_and(|w| w.is_finite() && w.abs() < 1e-7)),
        "body contact prototype requires inactive geometry-affecting jaw morphs; source-time facial contact is not yet supported");
    Ok(())
}
