use super::*;

const IDENT_SOURCE: [[f32; 4]; 22] = [[0., 0., 0., 1.]; 22];

#[test]
fn calibrated_transfer_preserves_mixed_arm_rotation_and_independent_wrist() {
    // Different bind frames, A-pose axes and unmapped intermediate joints.
    let rest = std::array::from_fn(|i| Quat::from_rotation_y(i as f32 * 0.07));
    let inter = [Quat::from_rotation_z(0.12); 22];
    let pelvis_parent = Quat::from_rotation_x(0.2);
    let own = std::array::from_fn(|i| {
        let p = SMPL_PARENTS[i];
        let parent = if p < 0 {
            pelvis_parent
        } else {
            rest[p as usize] * inter[i]
        };
        parent.inverse() * rest[i]
    });
    let axes = std::array::from_fn(|i| {
        Quat::from_euler(EulerRot::ZYX, 0.35, 0.03 * i as f32, -0.1)
            * Vec3::from_array(SMPL_BONE_AXES[i])
    });
    let reference = calibrated_cc5_world_frames(&rest, &axes, &inter, &own, pelvis_parent);
    let root = Quat::from_rotation_y(0.6);
    let collar = Quat::from_rotation_y(-0.4);
    let arm = Quat::from_euler(EulerRot::ZYX, 0.9, 0.4, 0.7);
    let elbow = Quat::from_rotation_y(-0.8);
    let wrist = Quat::from_rotation_x(1.2);
    let mut input = IDENT_SOURCE;
    for (i, q) in [(0, root), (13, collar), (16, arm), (18, elbow), (20, wrist)] {
        input[i] = q.to_array();
    }
    let output = fk_retarget_smpl_to_cc5_with_align(
        &input,
        &reference,
        &inter,
        pelvis_parent,
        Quat::IDENTITY,
    );
    let mut world = [Quat::IDENTITY; 22];
    for &i in SMPL_TOPO.iter() {
        let p = SMPL_PARENTS[i];
        world[i] = if p < 0 {
            pelvis_parent * output[i]
        } else {
            world[p as usize] * inter[i] * output[i]
        };
    }
    for (i, expected) in [
        (0, root),
        (13, root * collar),
        (16, root * collar * arm),
        (18, root * collar * arm * elbow),
        (20, root * collar * arm * elbow * wrist),
    ] {
        let transferred = world[i] * reference[i].inverse();
        assert!(
            transferred.angle_between(expected) < 0.001,
            "bone {i} lost orientation"
        );
    }
    // Bone axes still follow the same source directions despite bind tilt.
    let source_arm = root * collar * arm;
    let target_axis = world[16] * rest[16].inverse() * axes[16];
    assert!(target_axis.abs_diff_eq(source_arm * Vec3::X, 1e-5));
    assert!((world[0] * reference[0].inverse()).abs_diff_eq(root, 1e-5));
}

#[test]
fn calibrated_neutral_preserves_legacy_reference_and_pelvis_anatomy() {
    let rest = [Quat::from_rotation_y(0.2); 22];
    let own = [Quat::IDENTITY; 22];
    let inter = [Quat::IDENTITY; 22];
    let mut axes = SMPL_BONE_AXES.map(Vec3::from_array);
    axes[0] = Vec3::new(0., 0.97, 0.24).normalize();
    axes[16] = Vec3::new(0.8, -0.6, 0.);
    let parent = rest[0];
    let reference = calibrated_cc5_world_frames(&rest, &axes, &inter, &own, parent);
    assert!(reference[0].angle_between(rest[0]) < 0.001);
    let before = fk_retarget_smpl_to_cc5_axis_match_chained(
        &IDENT_SOURCE,
        &rest,
        &axes,
        &inter,
        &own,
        parent,
        Quat::IDENTITY,
        true,
    );
    let after = fk_retarget_smpl_to_cc5_with_align(
        &IDENT_SOURCE,
        &reference,
        &inter,
        parent,
        Quat::IDENTITY,
    );
    for i in 0..22 {
        assert!(
            before[i].angle_between(after[i]) < 0.001,
            "neutral bone {i} changed"
        );
    }
}
