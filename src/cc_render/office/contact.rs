//! Small post-retarget corrections. Large errors remain model/controller failures.
use super::controller::OfficeController;
use crate::cc_render::bevy_app::pose::CC5BindRotations;
use bevy::prelude::*;

/// Suppress high-frequency generation noise only in a supported seated idle.
/// New body actions and explicit timeline edits bypass this filter immediately.
#[derive(Default)]
pub(crate) struct SeatedIdleFilter {
    previous: Option<[Quat; 22]>,
}
impl SeatedIdleFilter {
    fn apply(&mut self, current: [Quat; 22], idle: bool, dt: f32) -> [Quat; 22] {
        let result = match self.previous {
            Some(previous) if idle => {
                let alpha = 1. - (-dt.max(0.) / 0.22).exp();
                std::array::from_fn(|i| previous[i].slerp(current[i], alpha))
            }
            _ => current,
        };
        self.previous = Some(result);
        result
    }
}
pub(crate) fn damp_seated_idle(
    office: Res<OfficeController>,
    bind: Res<CC5BindRotations>,
    time: Res<Time>,
    mut transforms: Query<&mut Transform>,
    mut filter: Local<SeatedIdleFilter>,
) {
    if !bind.captured {
        return;
    }
    let current = std::array::from_fn(|i| {
        bind.entities[i]
            .and_then(|e| transforms.get(e).ok())
            .map_or(Quat::IDENTITY, |t| t.rotation)
    });
    let filtered = filter.apply(current, office.seated_idle(), time.delta_secs());
    for (i, entity) in bind.entities.iter().enumerate() {
        if let Some(entity) = entity {
            if let Ok(mut transform) = transforms.get_mut(*entity) {
                transform.rotation = filtered[i];
            }
        }
    }
}

/// Solve the elbow location with the current bend plane as a pole hint.
fn elbow_point(shoulder: Vec3, elbow: Vec3, wrist: Vec3, target: Vec3) -> Option<Vec3> {
    let upper = shoulder.distance(elbow);
    let lower = elbow.distance(wrist);
    let offset = target - shoulder;
    let d = offset.length();
    if upper < 1e-5 || lower < 1e-5 || d < 1e-5 || d >= upper + lower || d <= (upper - lower).abs()
    {
        return None;
    }
    let axis = offset / d;
    let projected = (elbow - shoulder) - axis * (elbow - shoulder).dot(axis);
    let pole = if projected.length_squared() > 1e-8 {
        projected.normalize()
    } else {
        axis.any_orthonormal_vector()
    };
    let along = (upper * upper - lower * lower + d * d) / (2. * d);
    let height = (upper * upper - along * along).max(0.).sqrt();
    Some(shoulder + axis * along + pole * height)
}

/// Read this frame's retargeted pose before TransformPropagate runs. Reading
/// GlobalTransform here would solve against the previous displayed frame.
fn posed_global(
    entity: Entity,
    parents: &Query<&Parent>,
    transforms: &Query<&mut Transform>,
) -> Option<GlobalTransform> {
    let mut chain = Vec::new();
    let mut current = entity;
    loop {
        chain.push(*transforms.get(current).ok()?);
        match parents.get(current) {
            Ok(parent) if chain.len() < 128 => current = parent.get(),
            Ok(_) => return None,
            Err(_) => break,
        }
    }
    Some(
        chain
            .into_iter()
            .rev()
            .fold(GlobalTransform::IDENTITY, |world, local| world * local),
    )
}

#[derive(Clone, Copy)]
pub(crate) struct ContactResidual {
    offset: Vec3,
    rotation: Quat,
    pts_ms: u64,
}

pub(crate) fn correct_contacts(
    office: Res<OfficeController>,
    bind: Res<CC5BindRotations>,
    parents: Query<&Parent>,
    mut transforms: Query<&mut Transform>,
    mut residuals: Local<[Option<ContactResidual>; 4]>,
) {
    if !bind.captured {
        *residuals = [None; 4];
        return;
    }
    let pts_ms = office.observed.displayed_pts_ms;
    for (limb, (joint, indices)) in [
        ("LeftHand", [16, 18, 20]), ("RightHand", [17, 19, 21]),
        ("LeftFoot", [1, 4, 7]), ("RightFoot", [2, 5, 8]),
    ].into_iter().enumerate() {
        let [Some(shoulder), Some(elbow), Some(wrist)] = indices.map(|i| bind.entities[i]) else {
            continue;
        };
        let (Some(s), Some(e), Some(w)) = (
            posed_global(shoulder, &parents, &transforms),
            posed_global(elbow, &parents, &transforms),
            posed_global(wrist, &parents, &transforms),
        ) else {
            continue;
        };
        let active = office.display_motion().joints.iter().find(|c| {
            c.joint == joint && !c.rotation_only && pts_ms >= c.contact_pts_ms
        }).and_then(|constraint| {
            let target = Vec3::from(constraint.position)
                + if constraint.root_relative { Vec3::from(office.observed.pelvis) } else { Vec3::ZERO };
            // The model must first reach the neighborhood. Fade at the spatial
            // boundary as well as activation, avoiding a late 12 cm snap.
            let time = (pts_ms.saturating_sub(constraint.contact_pts_ms) as f32 / 200.).clamp(0., 1.);
            let proximity = ((0.35 - w.translation().distance(target)) / 0.15).clamp(0., 1.);
            let weight = time * time * (3. - 2. * time) * proximity * proximity * (3. - 2. * proximity);
            if weight <= 0. { return None; }
            let orientation = if joint.ends_with("Foot") {
                Quat::from_rotation_y(office.display_motion().heading.unwrap_or(office.observed.heading))
                    * bind.cc5_rest_world[indices[2]]
            } else {
                Quat::from_array(constraint.rotation_xyzw) * bind.calibrated_world[indices[2]]
            };
            let current = w.compute_transform().rotation;
            Some(ContactResidual {
                offset: (target - w.translation()).clamp_length_max(0.12) * weight,
                rotation: current.slerp(orientation, weight) * current.inverse(),
                pts_ms,
            })
        });
        let correction = active.or_else(|| residuals[limb].and_then(|last| {
            if pts_ms < last.pts_ms { return None; }
            let t = pts_ms.saturating_sub(last.pts_ms) as f32 / 200.;
            if t >= 1. { return None; }
            let weight = 1. - t * t * (3. - 2. * t);
            Some(ContactResidual { offset: last.offset * weight,
                rotation: Quat::IDENTITY.slerp(last.rotation, weight), ..last })
        }));
        let Some(mut correction) = correction else { residuals[limb] = None; continue; };
        if active.is_none() && office.display_motion().joints.iter().any(|c| c.joint == joint && c.rotation_only) {
            // The source pose already supplies the continuing grip orientation.
            // Fade the reach offset without applying an old wrist rotation twice.
            correction.rotation = Quat::IDENTITY;
        }
        // Release only the small retargeting residual, leaving the new generated
        // body motion authoritative. Never retain a hidden old body pose.
        let target = w.translation() + correction.offset;
        let Some(new_elbow) =
            elbow_point(s.translation(), e.translation(), w.translation(), target)
        else {
            tracing::debug!(target: "office_contact", joint,
                shoulder=?s.translation(), wrist=?w.translation(), goal=?target,
                reach=s.translation().distance(e.translation()) + e.translation().distance(w.translation()),
                "contact outside arm reach");
            continue;
        };
        if let Some(active) = active { residuals[limb] = Some(active); }
        let upper_delta = Quat::from_rotation_arc(
            (e.translation() - s.translation()).normalize(),
            (new_elbow - s.translation()).normalize(),
        );
        let lower_delta = Quat::from_rotation_arc(
            (upper_delta * (w.translation() - e.translation())).normalize(),
            (target - new_elbow).normalize(),
        );
        let (Ok(sp), Ok(ep), Ok(wp)) = (
            parents.get(shoulder),
            parents.get(elbow),
            parents.get(wrist),
        ) else {
            continue;
        };
        let (Some(sg), Some(eg), Some(wg)) = (
            posed_global(sp.get(), &parents, &transforms),
            posed_global(ep.get(), &parents, &transforms),
            posed_global(wp.get(), &parents, &transforms),
        ) else {
            continue;
        };
        if let Ok(mut t) = transforms.get_mut(shoulder) {
            t.rotation = sg.compute_transform().rotation.inverse()
                * upper_delta
                * s.compute_transform().rotation;
        }
        if let Ok(mut t) = transforms.get_mut(elbow) {
            t.rotation = (upper_delta * eg.compute_transform().rotation).inverse()
                * lower_delta
                * upper_delta
                * e.compute_transform().rotation;
        }
        if let Ok(mut t) = transforms.get_mut(wrist) {
            t.rotation = (lower_delta * upper_delta * wg.compute_transform().rotation).inverse()
                * correction.rotation * w.compute_transform().rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contact_preserves_calibrated_wrist_frame_at_the_target() {
        use crate::cc_render::office::controller::JointTarget;
        let mut app = App::new();
        let yaw = Quat::from_rotation_y(0.7);
        let calibration = Quat::from_rotation_x(0.5);
        let root = app.world_mut().spawn(Transform::from_rotation(yaw)).id();
        let shoulder = app.world_mut().spawn(Transform::IDENTITY).id();
        let elbow = app
            .world_mut()
            .spawn(Transform::from_xyz(0.25, 0.15, 0.))
            .id();
        let wrist = app
            .world_mut()
            .spawn(Transform::from_xyz(0.25, -0.15, 0.).with_rotation(calibration))
            .id();
        app.world_mut().entity_mut(root).add_child(shoulder);
        app.world_mut().entity_mut(shoulder).add_child(elbow);
        app.world_mut().entity_mut(elbow).add_child(wrist);
        let mut bind = CC5BindRotations::default();
        bind.captured = true;
        for (index, entity) in [(16, shoulder), (18, elbow), (20, wrist)] {
            bind.entities[index] = Some(entity);
        }
        bind.calibrated_world[20] = calibration;
        let mut office = OfficeController::default();
        office.observed.displayed_pts_ms = 200;
        office.motion.joints.push(JointTarget {
            joint: "LeftHand".into(),
            position: (yaw * Vec3::new(0.5, 0., 0.)).to_array(),
            rotation_xyzw: yaw.to_array(),
            root_relative: false,
            rotation_only: false,
            heading_reference: None,
            contact_pts_ms: 0,
        });
        app.insert_resource(bind)
            .insert_resource(office)
            .add_systems(Update, correct_contacts);
        app.update();
        let mut state =
            bevy::ecs::system::SystemState::<(Query<&Parent>, Query<&mut Transform>)>::new(
                app.world_mut(),
            );
        let (parents, transforms) = state.get_mut(app.world_mut());
        let actual = posed_global(wrist, &parents, &transforms)
            .unwrap()
            .compute_transform();
        assert!(actual.translation.distance(yaw * Vec3::new(0.5, 0., 0.)) < 1e-5);
        assert!(
            actual.rotation.angle_between(yaw * calibration) < 0.001,
            "contact discarded the target rig calibration"
        );

        // Generation supplies a fresh pose each render tick. A newly active
        // correction must approach the handle over time, not jump there.
        let origin = yaw * Vec3::new(0.5, 0., 0.);
        let goal = yaw * Vec3::new(0.46, 0.06, 0.);
        for (elapsed, fraction) in [(0, 0.), (100, 0.5), (200, 1.)] {
            let mut office = app.world_mut().resource_mut::<OfficeController>();
            office.observed.displayed_pts_ms = 1000 + elapsed;
            office.motion.joints[0].contact_pts_ms = 1000;
            office.motion.joints[0].position = goal.to_array();
            app.world_mut().get_mut::<Transform>(shoulder).unwrap().rotation = Quat::IDENTITY;
            app.world_mut().get_mut::<Transform>(elbow).unwrap().rotation = Quat::IDENTITY;
            app.world_mut().get_mut::<Transform>(wrist).unwrap().rotation = calibration;
            app.update();
            let (parents, transforms) = state.get_mut(app.world_mut());
            let actual = posed_global(wrist, &parents, &transforms).unwrap().compute_transform();
            assert!(actual.translation.distance(origin.lerp(goal, fraction)) < 1e-5,
                "contact must blend from the freshly retargeted pose at {elapsed} ms");
            assert!(actual.rotation.angle_between(yaw * calibration) < 0.001);
        }
        // Releasing the positional reach after attachment must not expose the
        // entire retargeting residual in one frame either.
        app.world_mut().resource_mut::<OfficeController>().motion.joints.clear();
        for (elapsed, fraction) in [(100, 0.5), (200, 0.)] {
            app.world_mut().resource_mut::<OfficeController>().observed.displayed_pts_ms = 1200 + elapsed;
            app.world_mut().get_mut::<Transform>(shoulder).unwrap().rotation = Quat::IDENTITY;
            app.world_mut().get_mut::<Transform>(elbow).unwrap().rotation = Quat::IDENTITY;
            app.world_mut().get_mut::<Transform>(wrist).unwrap().rotation = calibration;
            app.update();
            let (parents, transforms) = state.get_mut(app.world_mut());
            let actual = posed_global(wrist, &parents, &transforms).unwrap().compute_transform();
            assert!(actual.translation.distance(origin.lerp(goal, fraction)) < 1e-5,
                "releasing a contact must fade its residual at {elapsed} ms");
        }
    }

    #[test]
    fn idle_filter_damps_noise_but_new_motion_bypasses_it() {
        let mut filter = SeatedIdleFilter::default();
        let identity = [Quat::IDENTITY; 22];
        filter.apply(identity, false, 0.05);
        let motion = [Quat::from_rotation_x(0.2); 22];
        let smoothed = filter.apply(motion, true, 0.05);
        assert!(smoothed[0].angle_between(Quat::IDENTITY) < 0.05);
        assert_eq!(filter.apply(motion, false, 0.05), motion);
        // Idle entry starts from the latest action pose, not an old cached idle.
        assert!(filter.apply(motion, true, 0.05)[0].angle_between(motion[0]) < 1e-5);
    }
    #[test]
    fn idle_damping_is_independent_of_render_rate() {
        let mut slow = SeatedIdleFilter::default();
        let mut fast = SeatedIdleFilter::default();
        let identity = [Quat::IDENTITY; 22];
        slow.apply(identity, false, 0.);
        fast.apply(identity, false, 0.);
        let target = [Quat::from_rotation_x(0.5); 22];
        let one = slow.apply(target, true, 0.1);
        fast.apply(target, true, 0.05);
        let two = fast.apply(target, true, 0.05);
        assert!(1. - one[0].dot(two[0]).abs() < 1e-6);
    }
    #[test]
    fn reads_current_local_pose_before_global_propagation() {
        let mut world = World::new();
        let parent = world
            .spawn((
                Transform::from_xyz(2., 0., 0.)
                    .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
                GlobalTransform::IDENTITY,
            ))
            .id();
        let child = world
            .spawn((Transform::from_xyz(0., 0., 1.), GlobalTransform::IDENTITY))
            .id();
        world.entity_mut(parent).add_child(child);
        let mut state =
            bevy::ecs::system::SystemState::<(Query<&Parent>, Query<&mut Transform>)>::new(
                &mut world,
            );
        let (parents, transforms) = state.get_mut(&mut world);
        let current = posed_global(child, &parents, &transforms).unwrap();
        assert!(current.translation().distance(Vec3::new(3., 0., 0.)) < 1e-5);
    }
    #[test]
    fn preserves_lengths_and_rejects_unreachable() {
        let s = Vec3::ZERO;
        let e = Vec3::new(0.3, 0.2, 0.);
        let w = Vec3::new(0.6, 0., 0.);
        let goal = Vec3::new(0.5, 0.1, 0.1);
        let p = elbow_point(s, e, w, goal).unwrap();
        assert!((p.distance(s) - e.length()).abs() < 1e-5);
        assert!((p.distance(goal) - e.distance(w)).abs() < 1e-5);
        assert!(elbow_point(s, e, w, Vec3::new(2., 0., 0.)).is_none());
    }
}

/// Authored coarse closure for carried props; finger simulation is not required.
pub(crate) fn grasp_fingers(
    office: Res<OfficeController>,
    mut bones: Query<(Entity, &Name, &mut Transform)>,
    mut rests: Local<std::collections::HashMap<Entity, Quat>>,
) {
    for (entity, name, mut transform) in &mut bones {
        let hand = if name.as_str().starts_with("CC_Base_L_") {
            crate::cc_render::renderer::Hand::Left
        } else if name.as_str().starts_with("CC_Base_R_") {
            crate::cc_render::renderer::Hand::Right
        } else {
            continue;
        };
        let finger = ["Thumb", "Index", "Mid", "Ring", "Pinky"]
            .iter()
            .any(|part| name.as_str().contains(part));
        if !finger || name.as_str().contains("Toe") {
            continue;
        }
        let rest = *rests.entry(entity).or_insert(transform.rotation);
        let held = office.held.iter().any(|(h, _)| *h == hand);
        let curl = if held {
            if name.as_str().contains("Thumb") {
                0.35
            } else {
                0.8
            }
        } else {
            0.
        };
        transform.rotation = rest
            * Quat::from_rotation_z(if hand == crate::cc_render::renderer::Hand::Left {
                curl
            } else {
                -curl
            });
    }
}
