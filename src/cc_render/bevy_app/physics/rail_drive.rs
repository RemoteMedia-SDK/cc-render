//! Driven rails and the hand that follows them (owner approval 2026-10-17, openspec
//! add-declared-articulated-grips). A part on a prismatic rail gripped by a hand is held
//! kinematically on its rail; a declared driven travel moves it along the rail on an eased
//! profile, and the gripping hand's arm is solved so the wrist stays where it gripped the
//! handle. Let go, the part is handed back to its passive rail at rest. Dragged by a
//! force-limited spring instead, the part stuck, lurched past the hand, swung, and coasted on
//! when let go.

use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use super::grab::{ArticulatedGrip, GrabFsms, GrabState};

/// The gripping hand's arm blends onto, and off, its handle over this long.
const HAND_ON_HANDLE_BLEND_S: f32 = 0.15;

/// A rail part held by a grip: at `travel(t)` along `axis` from where it was gripped.
#[derive(Component, Debug, Clone)]
pub(crate) struct RailDrive {
    axis: Vec3,
    base: Vec3,
    base_travel: f32,
    limits: [f32; 2],
    from: f32,
    to: f32,
    start_s: f32,
    over_s: f32,
}

impl RailDrive {
    /// Held still where it is. `None` unless `joint` is a rail (prismatic) on a top-level part.
    pub(crate) fn hold(joint: &ImpulseJoint, parent: &GlobalTransform, part: &Transform, now_s: f32) -> Option<Self> {
        let TypedJoint::PrismaticJoint(rail) = &joint.data else { return None };
        let data = joint.data.as_ref();
        let parent = parent.compute_transform();
        let axis = (parent.rotation * data.local_axis1()).normalize_or_zero();
        let a = parent.transform_point(data.local_anchor1());
        let b = part.transform_point(data.local_anchor2());
        let travel = (b - a).dot(axis);
        let limits = rail.limits().map_or([f32::MIN, f32::MAX], |l| [l.min, l.max]);
        Some(Self { axis, base: part.translation, base_travel: travel, limits, from: travel, to: travel,
                    start_s: now_s, over_s: 0. })
    }

    pub(crate) fn travel(&self, now_s: f32) -> f32 {
        let s = if self.over_s <= 0. { 1. } else { ((now_s - self.start_s) / self.over_s).clamp(0., 1.) };
        self.from + (self.to - self.from) * s * s * (3. - 2. * s)
    }

    /// Drive on from where it is now to `to_m` (within the rail's limits) over `over_s`.
    pub(crate) fn drive(&mut self, now_s: f32, to_m: f32, over_s: f32) {
        self.from = self.travel(now_s);
        self.to = to_m.clamp(self.limits[0], self.limits[1]);
        self.start_s = now_s;
        self.over_s = over_s.max(0.);
    }

    fn translation(&self, now_s: f32) -> Vec3 {
        self.base + self.axis * (self.travel(now_s) - self.base_travel)
    }
}

/// Places each driven rail part for this frame (kinematic: the solver moves it there, pushing
/// what it carries).
pub(crate) fn drive_rails(time: Res<Time>, mut parts: Query<(&RailDrive, &mut Transform)>) {
    let now = time.elapsed_secs();
    for (drive, mut transform) in &mut parts {
        transform.translation = drive.translation(now);
    }
}

/// World rotations that take a two-bone chain (shoulder `a`, elbow `b`, wrist `c`) so the wrist
/// reaches `target` (or as near as the arm's length allows), the elbow staying on the side it
/// bends to: the upper arm's rotation, then the forearm's further rotation.
pub(crate) fn two_bone(a: Vec3, b: Vec3, c: Vec3, target: Vec3) -> (Quat, Quat) {
    let (l1, l2) = (a.distance(b), b.distance(c));
    let to = target - a;
    if l1 < 1e-5 || l2 < 1e-5 || to.length() < 1e-5 { return (Quat::IDENTITY, Quat::IDENTITY); }
    let u = to.normalize();
    let d = to.length().clamp((l1 - l2).abs() + 1e-4, l1 + l2 - 1e-4);
    let bend = (b - a) - u * (b - a).dot(u);
    let side = if bend.length() > 1e-5 { bend.normalize() } else { u.any_orthonormal_vector() };
    let cos = ((l1 * l1 + d * d - l2 * l2) / (2. * l1 * d)).clamp(-1., 1.);
    let elbow = a + (u * cos + side * (1. - cos * cos).max(0.).sqrt()) * l1;
    let upper = Quat::from_rotation_arc((b - a).normalize(), (elbow - a).normalize());
    let wrist = a + upper * (c - a);
    let fore = Quat::from_rotation_arc((wrist - elbow).normalize(), (a + u * d - elbow).normalize());
    (upper, fore)
}

/// This frame's posed world transform of `entity`, from the local transforms written so far
/// (before propagation).
fn posed(entity: Entity, parents: &Query<&Parent>, transforms: &Query<&mut Transform>) -> Option<Affine3A> {
    let mut affine = transforms.get(entity).ok()?.compute_affine();
    let mut at = entity;
    while let Ok(parent) = parents.get(at) {
        at = parent.get();
        affine = transforms.get(at).ok()?.compute_affine() * affine;
    }
    Some(affine)
}

/// Keeps each hand gripping a driven rail part on its grip point: the arm (upper arm, forearm)
/// is solved so the wrist reaches it, the hand keeping its generated world rotation. Blends in
/// as the grip is taken and back to the generated arm once let go.
pub(crate) fn keep_hand_on_handle(
    time: Res<Time>,
    bind: Option<Res<crate::cc_render::bevy_app::pose::CC5BindRotations>>,
    fsms: Res<GrabFsms>,
    grips: Query<&ArticulatedGrip>,
    driven: Query<(), With<RailDrive>>,
    parents: Query<&Parent>,
    mut transforms: Query<&mut Transform>,
    mut hands: Local<[(f32, Option<Vec3>); 2]>,
) {
    let Some(bind) = bind.filter(|b| b.captured) else { return };
    let step = time.delta_secs() / HAND_ON_HANDLE_BLEND_S;
    // SMPL-22 (shoulder, elbow, wrist) per hand.
    for (i, (state, chain)) in [(&fsms.left, [16, 18, 20]), (&fsms.right, [17, 19, 21])].into_iter().enumerate() {
        let gripped = match state {
            GrabState::Attached { prop_entity, grip_entity, .. } if driven.contains(*prop_entity) =>
                grips.get(*grip_entity).ok().zip(transforms.get(*prop_entity).ok())
                    .map(|(grip, part)| part.transform_point(grip.point)),
            _ => None,
        };
        let (weight, last) = &mut hands[i];
        if gripped.is_some() { *last = gripped; }
        *weight = (*weight + if gripped.is_some() { step } else { -step }).clamp(0., 1.);
        let (Some(target), true) = (*last, *weight > 0.) else { continue };
        let Some([shoulder, elbow, wrist]) = chain.map(|j| bind.entities[j]).iter().copied()
            .collect::<Option<Vec<_>>>().and_then(|v| <[Entity; 3]>::try_from(v).ok()) else { continue };
        // The chain must be shoulder -> elbow -> wrist for the solve to hold.
        if parents.get(elbow).map(Parent::get) != Ok(shoulder) || parents.get(wrist).map(Parent::get) != Ok(elbow) {
            continue;
        }
        let (Some(above), Some(sa), Some(ea), Some(wa)) = (parents.get(shoulder).ok().and_then(|p| posed(p.get(), &parents, &transforms)),
            posed(shoulder, &parents, &transforms), posed(elbow, &parents, &transforms), posed(wrist, &parents, &transforms))
        else { continue };
        let rotation = |a: &Affine3A| a.to_scale_rotation_translation().1;
        let (a, b, c) = (Vec3::from(sa.translation), Vec3::from(ea.translation), Vec3::from(wa.translation));
        let (upper, fore) = two_bone(a, b, c, c.lerp(target, *weight));
        let shoulder_world = upper * rotation(&sa);
        let elbow_world = fore * upper * rotation(&ea);
        let locals = [(shoulder, rotation(&above).inverse() * shoulder_world),
                      (elbow, shoulder_world.inverse() * elbow_world),
                      (wrist, elbow_world.inverse() * rotation(&wa))];
        for (entity, local) in locals {
            if let Ok(mut transform) = transforms.get_mut(entity) { transform.rotation = local.normalize(); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_bone_puts_the_wrist_on_a_reachable_target_keeping_the_elbow_side() {
        let (a, b, c) = (Vec3::new(0., 1.4, 0.), Vec3::new(0.05, 1.12, 0.08), Vec3::new(0.1, 0.9, 0.3));
        let target = Vec3::new(0.05, 1.0, 0.45);
        let (upper, fore) = two_bone(a, b, c, target);
        let elbow = a + upper * (b - a);
        let wrist = elbow + fore * upper * (c - b);
        assert!(wrist.distance(target) < 1e-3, "wrist at {wrist}, target {target}");
        assert!((elbow.distance(a) - b.distance(a)).abs() < 1e-4 && (wrist.distance(elbow) - c.distance(b)).abs() < 1e-4);
        // The elbow still bends to the side it bent to.
        let u = (target - a).normalize();
        let side = |e: Vec3| (e - a) - u * (e - a).dot(u);
        assert!(side(elbow).dot(side(b)) > 0.);
        // Out of reach: the arm points straight at it.
        let (upper, fore) = two_bone(a, b, c, a + Vec3::Z * 2.);
        let wrist = a + upper * (b - a) + fore * upper * (c - b);
        assert!((wrist - a).normalize().dot(Vec3::Z) > 0.999);
    }

    #[test]
    fn a_hand_gripping_a_driven_drawer_is_solved_onto_its_grip_point_and_released_back() {
        use crate::cc_render::bevy_app::pose::CC5BindRotations;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, HierarchyPlugin));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(1. / 60.)));
        app.init_resource::<GrabFsms>();
        app.add_systems(PostUpdate, (drive_rails, keep_hand_on_handle).chain()
            .before(bevy::transform::TransformSystem::TransformPropagate));
        // A right arm (scaled avatar root) hanging beside a drawer front.
        let world = app.world_mut();
        let root = world.spawn(Transform::from_xyz(0., 0., 0.).with_scale(Vec3::splat(1.))).id();
        let collar = world.spawn(Transform::from_xyz(-0.18, 1.4, 0.)).set_parent(root).id();
        let shoulder = world.spawn(Transform::from_xyz(0., 0., 0.)).set_parent(collar).id();
        let elbow = world.spawn(Transform::from_xyz(0.03, -0.28, 0.05)).set_parent(shoulder).id();
        let wrist = world.spawn(Transform::from_xyz(0.02, -0.25, 0.08)
            .with_rotation(Quat::from_rotation_x(0.3))).set_parent(elbow).id();
        let mut bind = CC5BindRotations { captured: true, ..Default::default() };
        (bind.entities[17], bind.entities[19], bind.entities[21]) = (Some(shoulder), Some(elbow), Some(wrist));
        world.insert_resource(bind);
        let drawer = world.spawn((Transform::from_xyz(-0.2, 1.0, 0.1), RailDrive { axis: Vec3::Z,
            base: Vec3::new(-0.2, 1.0, 0.1), base_travel: 0., limits: [0., 0.4], from: 0., to: 0., start_s: 0., over_s: 0. })).id();
        let point = Vec3::new(0.05, 0.1, -0.1);
        let grip = world.spawn(ArticulatedGrip { point }).id();
        for _ in 0..2 { app.update(); }
        let hand_rotation = app.world().get::<GlobalTransform>(wrist).unwrap().compute_transform().rotation;
        app.world_mut().resource_mut::<GrabFsms>().right =
            GrabState::Attached { target: "drawer".into(), prop_entity: drawer, grip_entity: grip };
        app.world_mut().get_mut::<RailDrive>(drawer).unwrap().drive(0., 0.15, 0.5);
        for _ in 0..40 { app.update(); }
        let handle = app.world().get::<Transform>(drawer).unwrap().transform_point(point);
        let at = app.world().get::<GlobalTransform>(wrist).unwrap().compute_transform();
        assert!((app.world().get::<Transform>(drawer).unwrap().translation.z - 0.25).abs() < 1e-3, "drawer driven out");
        assert!(at.translation.distance(handle) < 2e-3, "wrist at {}, handle grip {}", at.translation, handle);
        assert!(at.rotation.angle_between(hand_rotation) < 1e-3, "the hand keeps its generated rotation");
        // Let go: the arm blends back to the generated pose (only rotations were ever written).
        app.world_mut().resource_mut::<GrabFsms>().right = GrabState::Idle;
        for (entity, local) in [(shoulder, Quat::IDENTITY), (elbow, Quat::IDENTITY), (wrist, Quat::from_rotation_x(0.3))] {
            app.world_mut().get_mut::<Transform>(entity).unwrap().rotation = local;  // the next generated frame
        }
        for _ in 0..12 {
            for (entity, local) in [(shoulder, Quat::IDENTITY), (elbow, Quat::IDENTITY), (wrist, Quat::from_rotation_x(0.3))] {
                app.world_mut().get_mut::<Transform>(entity).unwrap().rotation = local;
            }
            app.update();
        }
        assert_eq!(app.world().get::<Transform>(elbow).unwrap().rotation, Quat::IDENTITY);
    }

    #[test]
    fn a_driven_rail_eases_from_where_it_is_to_its_target_within_its_limits() {
        let mut drive = RailDrive { axis: Vec3::X, base: Vec3::new(1., 0.7, 0.), base_travel: 0., limits: [0., 0.4],
                                    from: 0., to: 0., start_s: 0., over_s: 0. };
        assert_eq!(drive.travel(5.), 0.);
        drive.drive(1., 0.6, 1.);
        assert_eq!(drive.travel(1.), 0.);
        assert!((drive.travel(1.5) - 0.2).abs() < 1e-6);
        assert_eq!(drive.travel(2.5), 0.4);
        assert_eq!(drive.translation(3.), Vec3::new(1.4, 0.7, 0.));
        // Redriven midway, it goes on from where it is.
        drive.drive(1.5, 0., 1.);
        assert!((drive.travel(1.5) - 0.2).abs() < 1e-6);
    }
}
