use super::*;
use crate::cc_render::{cloth_solver::ClothParameters, office::timeline::StreamIdentity};

fn solver() -> ClothSolver {
    let points = vec![Vec3::ZERO, Vec3::X, Vec3::Z];
    ClothSolver::new(
        points.clone(),
        points,
        vec![[0, 1, 2]],
        vec![Vec2::ZERO; 3],
        ClothParameters {
            total_mass_kg: 0.8,
            thickness_m: 0.001,
            stretch_compliance: 1e-7,
            bend_compliance: 0.05,
            air_drag_per_second: 0.2,
            spring_damping_ratio: 1.,
            cloth_static_friction: 0.2,
            cloth_kinetic_friction: 0.2,
            substeps: 24,
        },
    )
    .unwrap()
}

fn pad_pose(radius: f32) -> GlobalTransform {
    Transform::from_xyz(0.25, radius + 0.0004, 0.25)
        .with_rotation(Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2))
        .into()
}

#[test]
fn distal_contact_uses_observed_orientation_and_shell_not_proximity() {
    let cloth = solver();
    let finger = &crate::cc_render::office::hand::tests::right_hand().fingers[1];
    let global = pad_pose(finger.radius);
    let before = cloth.surface().positions().to_vec();
    let observed = finger_contacts(cloth.surface(), "Index", finger, &global).unwrap();
    assert_eq!(observed.contacts.len(), 1);
    assert!(observed.contacts[0].separation_m < 0.);
    assert!((observed.segment_end[1] - observed.segment_start[1]).abs() < 1e-6);
    assert!(observed.segment_end[0] > observed.segment_start[0]);
    assert_eq!(observed.contacts[0].surface_revision, 0);
    assert_eq!(cloth.surface().positions(), before);
    let far = GlobalTransform::from_translation(Vec3::new(0.25, finger.radius + 0.001, 0.25));
    assert!(finger_contacts(cloth.surface(), "Index", finger, &far)
        .unwrap()
        .contacts
        .is_empty());
    let invalid = GlobalTransform::from_translation(Vec3::splat(f32::NAN));
    assert!(finger_contacts(cloth.surface(), "Index", finger, &invalid).is_err());
}

fn app() -> (App, Entity, Entity) {
    let mut app = App::new();
    let rig = crate::cc_render::office::hand::tests::right_hand();
    let bones = std::array::from_fn(|i| {
        let bone = app.world_mut().spawn(pad_pose(rig.fingers[i].radius)).id();
        [bone; 3]
    });
    let first = bones[0][2];
    let wrist = app.world_mut().spawn(GlobalTransform::IDENTITY).id();
    app.insert_resource(HandRigs {
        hands: [
            None,
            Some(CapturedHand {
                rig,
                hand: wrist,
                bones,
            }),
        ],
    });
    let mut clock = ClothClock::default();
    clock.observe("contact-test", 100).unwrap();
    let cloth = LiveCloth {
        id: "test-sheet".into(),
        solver: solver(),
        colliders: Vec::new(),
        clock,
        error: None,
    };
    let snapshot = cloth.snapshot(None, false);
    let entity = app.world_mut().spawn(cloth).id();
    let mut office = OfficeController::default();
    office.observed.native_stream = Some(StreamIdentity {
        session_id: "contact-test".into(),
        generation_revision: 1,
        skeleton_id: "smpl22".into(),
        root_mode: "world".into(),
    });
    office.observed.displayed_pts_ms = 100;
    app.insert_resource(office)
        .insert_resource(ClothObservations(vec![snapshot]))
        .add_systems(Update, observe);
    (app, entity, first)
}

#[test]
fn contact_observer_reads_globals_without_posing_fingers_or_moving_cloth() {
    let (mut app, cloth, first) = app();
    let before = *app.world().get::<GlobalTransform>(first).unwrap();
    let points = app
        .world()
        .get::<LiveCloth>(cloth)
        .unwrap()
        .solver
        .surface()
        .positions()
        .to_vec();
    app.update();
    let hands = &app.world().resource::<ClothObservations>().0[0].hand_contacts;
    assert_eq!(hands[0].status, "rig_unavailable");
    assert_eq!(hands[1].status, "observed");
    assert_eq!(hands[1].fingers.len(), 5);
    assert!(hands[1].fingers.iter().all(|f| !f.contacts.is_empty()));
    assert_eq!(*app.world().get::<GlobalTransform>(first).unwrap(), before);
    assert_eq!(
        app.world()
            .get::<LiveCloth>(cloth)
            .unwrap()
            .solver
            .surface()
            .positions(),
        points
    );
    assert_eq!(
        app.world()
            .get::<LiveCloth>(cloth)
            .unwrap()
            .solver
            .surface()
            .revision(),
        0
    );
    app.world_mut()
        .entity_mut(first)
        .insert(GlobalTransform::from_translation(Vec3::Y));
    app.update();
    assert!(
        app.world().resource::<ClothObservations>().0[0].hand_contacts[1].fingers[0]
            .contacts
            .is_empty()
    );
}

#[test]
fn unavailable_or_unsynchronized_motion_never_reuses_old_hand_contacts() {
    let (mut app, _, _) = app();
    app.update();
    app.world_mut()
        .resource_mut::<OfficeController>()
        .observed
        .displayed_pts_ms = 117;
    app.update();
    let hands = &app.world().resource::<ClothObservations>().0[0].hand_contacts;
    assert!(hands
        .iter()
        .all(|h| h.status == "unsynchronized_pose" && h.fingers.is_empty()));
    {
        let mut office = app.world_mut().resource_mut::<OfficeController>();
        office.observed.displayed_pts_ms = 100;
        office.observed.native_stream.as_mut().unwrap().session_id = "other-session".into();
    }
    app.update();
    assert!(app.world().resource::<ClothObservations>().0[0]
        .hand_contacts
        .iter()
        .all(|h| h.status == "unsynchronized_pose" && h.fingers.is_empty()));
    app.world_mut()
        .resource_mut::<OfficeController>()
        .motion_error = Some("test failure".into());
    app.update();
    assert!(app.world().resource::<ClothObservations>().0[0]
        .hand_contacts
        .iter()
        .all(|h| h.status == "motion_unavailable" && h.fingers.is_empty()));
}

#[test]
fn production_office_schedule_has_no_finger_pose_writer() {
    let mut app = App::new();
    crate::cc_render::office::systems::install(&mut app);
    let names: Vec<_> = app
        .get_schedule(PostUpdate)
        .unwrap()
        .graph()
        .systems()
        .map(|(_, system, _)| system.name().to_string())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.ends_with("office::systems::observe")),
        "{names:?}"
    );
    assert!(names.iter().all(|n| !n.contains("::hand::")), "{names:?}");
}
