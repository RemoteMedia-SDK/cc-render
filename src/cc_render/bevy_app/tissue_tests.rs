use super::*;
use crate::cc_render::{
    renderer::SkeletalPose,
    soft_tissue::{Attachment, Material, RegionAsset},
};
use bevy::ecs::system::SystemState;

#[test]
fn tissue_parallel_regions_match_serial_state_and_report_first_failure() {
    let asset = TissueAsset::from_json(include_bytes!(
        "../../../assets/tissue/beautiful_assistant.base-viscous-prototype.json"
    )).unwrap();
    let mut serial: Vec<_> = asset.regions.iter().map(|r| r.solver(Frame::IDENTITY).unwrap()).collect();
    let mut parallel = serial.clone();
    let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(2).build();
    for tick in 1..=24 {
        let time = tick as f64 / 240.0;
        let frames: Vec<_> = (0..serial.len()).map(|i| Frame {
            translation: DVec3::new(0.001 * (time * 7.0).sin(), 0.002 * (time * 13.0).sin(), 0.0),
            rotation: bevy::math::DQuat::from_rotation_z(0.01 * (time * (i + 1) as f64).sin()),
        }).collect();
        step_regions(&mut serial, &frames, 1.0 / 240.0, None).unwrap();
        step_regions(&mut parallel, &frames, 1.0 / 240.0, Some(&pool)).unwrap();
        for (a, b) in serial.iter().zip(&parallel) {
            assert_eq!(a.positions(), b.positions());
            assert_eq!(a.velocities(), b.velocities());
        }
    }
    let invalid = Frame { translation: DVec3::splat(f64::NAN), ..Frame::IDENTITY };
    for frames in [vec![invalid, invalid], vec![Frame::IDENTITY, invalid]] {
        let expected = usize::from(frames[0].translation.is_finite());
        for mode in [None, Some(&pool)] {
            let mut trial = parallel.clone();
            let (index, error) = step_regions(&mut trial, &frames, 1.0 / 240.0, mode).unwrap_err();
            assert_eq!(index, expected);
            assert!(matches!(error, TissueError::InvalidInput("attachment frame")));
        }
    }
}

#[test]
fn tissue_region_failure_does_not_commit_successful_sibling() {
    let mut asset = fixture_asset();
    asset.regions.push(asset.regions[0].clone());
    let mut solvers: Vec<_> = asset.regions.iter().map(|r| r.solver(Frame::IDENTITY).unwrap()).collect();
    let before: Vec<_> = solvers.iter().map(|s| (s.positions().to_vec(), s.velocities().to_vec())).collect();
    let pose_at = |time: f64| Ok(Arc::new(ContactPose {
        frames: vec![Frame::IDENTITY, Frame {
            translation: if time > 0.0 { DVec3::splat(f64::NAN) } else { DVec3::ZERO },
            ..Frame::IDENTITY
        }],
        skin: vec![],
    }));
    let error = integrate_interval(&mut solvers, 0.0, 1000.0 / 240.0, 0, &pose_at, &BodyContacts::new(&[]).unwrap(), &asset)
        .err().expect("the invalid second region must reject the whole interval");
    assert!(error.to_string().starts_with("region 1,"));
    for (solver, (positions, velocities)) in solvers.iter().zip(before) {
        assert_eq!(solver.positions(), positions);
        assert_eq!(solver.velocities(), velocities);
    }
}

fn fixture_asset() -> TissueAsset {
    let positions = vec![
        [0.0, 0.0, 0.0],
        [0.1, 0.0, 0.0],
        [0.0, 0.1, 0.0],
        [0.0, 0.0, 0.1],
    ];
    TissueAsset {
        schema_version: 1,
        contact_surfaces: vec![],
        contact_inactive_morph_targets: vec![],
        source_sha256: vec!["0".repeat(64)],
        surfaces: vec![],
        regions: vec![RegionAsset {
            name: "test".into(),
            anchor_bone: "anchor".into(),
            rest_positions: positions.clone(),
            initial_positions: positions,
            tetrahedra: vec![[0, 1, 2, 3]],
            attachments: vec![
                Attachment::Fixed,
                Attachment::Fixed,
                Attachment::Fixed,
                Attachment::Free,
            ],
            material: Material {
                pressure_discretization: Default::default(),
                pressure_stabilization_pa: 0.0,
                element_pressure_quality_cutoff: 0.0,
                time_integration: Default::default(),
                viscosity: None,
                density_kg_m3: 950.0,
                shear_pa: 1500.0,
                strain_stiffening: 0.0,
                bulk_pa: 30000.0,
                strain_damping_per_second: 60.0,
                iterations: 4,
            },
        }],
    }
}

fn fixture() -> (World, Active) {
    let mut world = World::new();
    let root = world.spawn(Transform::IDENTITY).id();
    let anchor = world.spawn(Transform::from_xyz(0.0, 1.0, 0.0)).id();
    world.entity_mut(root).add_child(anchor);
    let mut bind = CC5BindRotations::default();
    bind.entities[0] = Some(root);
    bind.inter_rest = [Quat::IDENTITY; 22];
    bind.calibrated_world = [Quat::IDENTITY; 22];
    bind.parent_chain_rest = [Quat::IDENTITY; 22];
    let mut query = SystemState::<Query<(&Transform, Option<&Parent>)>>::new(&mut world);
    let rig = TissueRig::capture(&[anchor], &query.get(&world), &bind, &[]).unwrap();
    (
        world,
        Active {
            rig,
            anchors: vec![anchor],
            surfaces: vec![],
            contacts: BodyContacts::new(&[]).unwrap(),
            solvers: vec![],
            session: None,
            origin_ms: 0.0,
            tick: 0,
            report_ms: 0,
            snapshot_written: false,
            motion_initialized: false,
            morph_history: Default::default(),
        },
    )
}

fn trajectory() -> PoseTimeline {
    let mut timeline = PoseTimeline::default();
    for i in 0..=12 {
        let mut pose = SkeletalPose::rest();
        pose.pts_ms = i * 50;
        pose.root_pos = [(i as f32 * 0.5).sin() * 0.005, 0.0, 0.0];
        pose.joint_quats[0] = Quat::from_rotation_y(i as f32 * 0.01).to_array();
        pose.stream = Some(crate::cc_render::office::timeline::StreamIdentity {
            session_id: "test".into(),
            generation_revision: 1,
            skeleton_id: "smpl22".into(),
            root_mode: "world".into(),
        });
        timeline.insert(pose, 0.0);
    }
    timeline
}

#[test]
fn tissue_playback_is_render_rate_independent_and_pauses_without_skeleton_writes() {
    playback_render_rate_check(crate::cc_render::soft_tissue::TimeIntegration::ImplicitEuler, None);
}

#[test]
fn tissue_bdf2_history_is_render_rate_independent_and_pauses_without_skeleton_writes() {
    playback_render_rate_check(crate::cc_render::soft_tissue::TimeIntegration::Bdf2, None);
}

#[test]
fn tissue_continuum_viscosity_is_render_rate_independent_and_pauses_without_skeleton_writes() {
    playback_render_rate_check(crate::cc_render::soft_tissue::TimeIntegration::Bdf2,
        Some(crate::cc_render::soft_tissue::Viscosity {shear_pa_s:15.,bulk_pa_s:300.}));
}

fn playback_render_rate_check(method: crate::cc_render::soft_tissue::TimeIntegration, viscosity: Option<crate::cc_render::soft_tissue::Viscosity>) {
    let mut results = Vec::new();
    for fps in [30, 60, 120] {
        let (world, mut active) = fixture();
        let original: Vec<_> = world
            .iter_entities()
            .filter_map(|e| e.get::<Transform>().map(|t| (e.id(), *t)))
            .collect();
        let mut asset = fixture_asset();
        for region in &mut asset.regions {
            region.material.time_integration = method;
            region.material.viscosity = viscosity;
            if viscosity.is_some() { region.material.strain_damping_per_second = 0.; }
        }
        let request = TissueRequest {
            asset,
            rate: 120,
            snapshot: None,
            trace: None,
        };
        let mut timeline = trajectory();
        let mut meshes = Assets::<Mesh>::default();
        assert!(active.simulate(&request, &timeline, &HashMap::new(), &[vec![]], &mut meshes).is_err());
        assert_eq!(
            active
                .simulate(&request, &timeline, &HashMap::new(), &[], &mut meshes)
                .unwrap(),
            None
        );
        for tick in 0..=fps / 2 {
            let pose = timeline.sample(tick as f64 / fps as f64).unwrap();
            let globals = active.rig.sample(&pose);
            let rendered_pts = active
                .simulate(&request, &timeline, &globals, &[], &mut meshes)
                .unwrap();
            assert_eq!(rendered_pts, Some(pose.pts_ms));
        }
        let before = active.solvers[0].positions().to_vec();
        let tick = active.tick;
        let pose = timeline.sample(0.5).unwrap();
        for _ in 0..5 {
            let rendered_pts = active
                .simulate(&request, &timeline, &active.rig.sample(&pose), &[], &mut meshes)
                .unwrap();
            assert_eq!(rendered_pts, Some(pose.pts_ms));
        }
        assert_eq!(active.tick, tick);
        assert_eq!(active.solvers[0].positions(), before);
        for (entity, transform) in original {
            assert_eq!(*world.get::<Transform>(entity).unwrap(), transform);
        }
        results.push(before);
    }
    for result in &results[1..] {
        for (a, b) in result.iter().zip(&results[0]) {
            assert!(
                a.distance(*b) < 1e-8,
                "render-rate drift {}",
                a.distance(*b)
            );
        }
    }
}

#[test]
fn late_second_source_knot_does_not_change_initial_tissue_velocity() {
    let mut results = Vec::new();
    for preload in [true, false] {
        let (_, mut active) = fixture();
        let request = TissueRequest {
            asset: fixture_asset(),
            rate: 120,
            snapshot: None,
            trace: None,
        };
        let mut timeline = trajectory();
        let first = timeline.sample(0.0).unwrap();
        if !preload {
            timeline = PoseTimeline::default();
            timeline.insert(first.clone(), 0.0);
            timeline.sample(0.0).unwrap();
        }
        let mut meshes = Assets::default();
        active
            .simulate(&request, &timeline, &active.rig.sample(&first), &[], &mut meshes)
            .unwrap();
        // Both deliveries now expose exactly the same committed source window.
        // Only the availability of the second knot on the first display differed.
        timeline = trajectory();
        let pose = timeline.sample(0.1).unwrap();
        active
            .simulate(&request, &timeline, &active.rig.sample(&pose), &[], &mut meshes)
            .unwrap();
        results.push(active.solvers[0].positions().to_vec());
    }
    let error = results[0]
        .iter()
        .zip(&results[1])
        .map(|(a, b)| a.distance(*b))
        .fold(0.0_f64, f64::max);
    assert!(
        error < 1e-8,
        "source delivery changed initial tissue motion by {error} metres"
    );
}

#[test]
fn tissue_starts_at_first_observed_pose_then_preserves_render_rate_independence() {
    let mut results = Vec::new();
    for fps in [30, 60, 120] {
        let (world, mut active) = fixture();
        let original: Vec<_> = world.iter_entities()
            .filter_map(|e| e.get::<Transform>().map(|t| (e.id(), *t))).collect();
        let request = TissueRequest { asset: fixture_asset(), rate: 120, snapshot: None, trace: None };
        let mut timeline = trajectory();
        let mut meshes = Assets::default();
        // The first presentation is inside the [50, 100] ms source interval.
        // No facial observation exists for the earlier source knots.
        let pose = timeline.sample(0.066).unwrap();
        let globals = active.rig.sample(&pose);
        assert_eq!(active.simulate(&request, &timeline, &globals, &[], &mut meshes).unwrap(), Some(66));
        assert_eq!(active.origin_ms, 66.0);
        assert_eq!(active.tick, 0);
        assert!(active.morph_history.sample(50.0).is_err());
        let frame = tissue_motion::frame(globals[&active.anchors[0]]).unwrap();
        for (point, initial) in active.solvers[0].positions().iter()
            .zip(&request.asset.regions[0].initial_positions) {
            assert!(point.distance(frame.point((*initial).into())) < 1e-12);
        }
        for tick in 1..=fps / 10 {
            let pose = timeline.sample(0.066 + tick as f64 / fps as f64).unwrap();
            active.simulate(&request, &timeline, &active.rig.sample(&pose), &[], &mut meshes).unwrap();
        }
        assert_eq!(active.origin_ms, 66.0);
        assert_eq!(active.tick, 12);
        for (entity, transform) in original {
            assert_eq!(*world.get::<Transform>(entity).unwrap(), transform);
        }
        results.push(active.solvers[0].positions().to_vec());
    }
    for result in &results[1..] {
        for (a, b) in result.iter().zip(&results[0]) {
            assert!(a.distance(*b) < 1e-8, "render-rate drift {}", a.distance(*b));
        }
    }
}

#[test]
fn tissue_request_excludes_breast_springs_without_changing_other_secondary_bones() {
    use super::super::physics::secondary::{auto_detect_chains, JiggleBone, JiggleSpawned};
    let mut app = App::new();
    app.init_resource::<JiggleSpawned>();
    let mut bind = CC5BindRotations::default();
    bind.captured = true;
    app.insert_resource(bind);
    app.insert_resource(TissueRequest {
        asset: fixture_asset(),
        rate: 120,
        snapshot: None,
            trace: None,
    });
    app.add_systems(Update, auto_detect_chains);
    let parent = app.world_mut().spawn(Transform::IDENTITY).id();
    let mut bones = Vec::new();
    for name in ["CC_Base_L_Breast", "CC_Base_R_Breast", "CC_Base_Hair_Front"] {
        let bone = app
            .world_mut()
            .spawn((Name::new(name), Transform::from_xyz(0.1, 0.0, 0.0)))
            .id();
        app.world_mut().entity_mut(parent).add_child(bone);
        bones.push(bone);
    }
    app.update();
    let world = app.world_mut();
    let bodies: Vec<_> = world.query::<&JiggleBone>().iter(world).collect();
    assert_eq!(bodies.len(), 1);
}

#[test]
fn tissue_subdivision_resolves_fast_attachment_motion_and_rolls_back_failure() {
    let asset = fixture_asset();
    let initial = asset.regions[0].solver(Frame::IDENTITY).unwrap();
    let mut solvers = vec![initial];
    let count = integrate_interval(
        &mut solvers,
        0.0,
        1000.0 / 60.0,
        0,
        &|at| {
            Ok(Arc::new(ContactPose {
                skin: vec![],
                frames: vec![Frame {
                    translation: DVec3::Z * (at * 0.006),
                    ..Frame::IDENTITY
                }],
            }))
        },
        &BodyContacts::new(&[]).unwrap(),
        &asset,
    )
    .unwrap();
    assert!(count.subdivisions > 0);
    assert!(solvers[0].metrics().minimum_volume_ratio > 0.0);
    let before = solvers[0].positions().to_vec();
    let velocity = solvers[0].velocities().to_vec();
    assert!(integrate_interval(
        &mut solvers,
        0.0,
        10.0,
        0,
        &|_| Ok(Arc::new(ContactPose {
            skin: vec![],
            frames: vec![Frame {
                translation: DVec3::splat(f64::NAN),
                ..Frame::IDENTITY
            }]
        })),
        &BodyContacts::new(&[]).unwrap(),
        &asset
    )
    .is_err());
    assert_eq!(solvers[0].positions(), before);
    assert_eq!(solvers[0].velocities(), velocity);
}

#[test]
fn tissue_contact_rejects_unsampled_geometry_morphs_but_allows_unrelated_expression() {
    let mut weights = vec![0.0; 64];
    weights[12] = 1.0;
    assert!(validate_contact_morphs(&[58, 59, 60, 61], &weights).is_ok());
    weights[59] = 0.1;
    assert!(validate_contact_morphs(&[58, 59, 60, 61], &weights).is_err());
    assert!(validate_contact_morphs(&[64], &weights).is_err());
}

#[test]
fn tissue_subdivision_recovers_from_nonlinear_nonconvergence() {
    let mut asset = fixture_asset();
    asset.regions[0].material.iterations = 1;
    let initial = asset.regions[0].solver(Frame::IDENTITY).unwrap();
    let duration = 1000. / 60.;
    let frame = |at| Frame { translation: DVec3::X * (at * 0.00015), ..Frame::IDENTITY };
    let mut trial = initial.clone();
    assert!(matches!(trial.step(duration / 1000., frame(duration), DVec3::NEG_Y * 9.81),
        Err(TissueError::NonlinearSolveFailed { .. })));
    let mut solvers = vec![initial];
    let mut cached = solvers.clone();
    let pose_at = |at| Ok(ContactPose {
        skin: vec![], frames: vec![frame(at)],
    });
    let report = integrate_interval(&mut solvers, 0., duration, 0,
        &|at| pose_at(at).map(Arc::new), &BodyContacts::new(&[]).unwrap(), &asset).unwrap();
    assert!(report.subdivisions > 0);
    assert_eq!(solvers[0].frame().translation, frame(duration).translation);
    assert!(solvers[0].metrics().minimum_volume_ratio > 0.);
    let cache = tissue_motion::ContactPoseCache::default();
    let cached_report = integrate_interval(&mut cached, 0., duration, 0,
        &|at| cache.sample(at, || pose_at(at)), &BodyContacts::new(&[]).unwrap(), &asset).unwrap();
    assert_eq!(cached_report.subdivisions, report.subdivisions);
    assert_eq!(cached[0].positions(), solvers[0].positions());
    assert_eq!(cached[0].velocities(), solvers[0].velocities());
    assert!(cache.counts().1 > 0);
}

#[test]
fn tissue_contact_samples_the_existing_forearm_twist_conversion_without_bone_writes() {
    let mut world = World::new();
    let root = world.spawn(Transform::IDENTITY).id();
    let hand = world.spawn(Transform::IDENTITY).id();
    let twist01 = world.spawn(Transform::IDENTITY).id();
    let twist02 = world.spawn(Transform::IDENTITY).id();
    world.entity_mut(root).add_children(&[hand, twist01]);
    world.entity_mut(twist01).add_child(twist02);
    let mut bind = CC5BindRotations::default();
    bind.entities[0] = Some(root);
    bind.entities[20] = Some(hand);
    bind.inter_rest = [Quat::IDENTITY; 22];
    bind.calibrated_world = [Quat::IDENTITY; 22];
    bind.parent_chain_rest = [Quat::IDENTITY; 22];
    let forearm = super::super::twist::ForearmRig {
        hand,
        twist01,
        twist02,
        hand_rest: Quat::IDENTITY,
        axis: Vec3::X,
        twist01_rest: Quat::IDENTITY,
        twist02_rest: Quat::IDENTITY,
    };
    let mut query = SystemState::<Query<(&Transform, Option<&Parent>)>>::new(&mut world);
    let rig = TissueRig::capture(&[hand, twist02], &query.get(&world), &bind, &[forearm]).unwrap();
    let mut pose = SkeletalPose::rest();
    pose.joint_quats[20] = Quat::from_rotation_x(1.2).to_array();
    let sampled = rig.sample(&pose);
    let observed = sampled[&twist02].to_scale_rotation_translation().1;
    assert!(observed.angle_between(Quat::from_rotation_x(0.9)) < 1e-6);
    for entity in [root, hand, twist01, twist02] {
        assert_eq!(
            *world.get::<Transform>(entity).unwrap(),
            Transform::IDENTITY
        );
    }
}
