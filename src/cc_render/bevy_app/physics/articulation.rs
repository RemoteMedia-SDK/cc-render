//! Read-only joint facts measured from the published prop transforms.
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use crate::cc_render::renderer::{ArticulationCoordinate, ArticulationSnapshot};

pub(super) fn observe(
    joint: &TypedJoint,
    parent: (&super::props::Prop, &GlobalTransform),
    child: &GlobalTransform,
    registered: bool,
) -> Option<ArticulationSnapshot> {
    let (prop, parent) = parent;
    let parent = parent.compute_transform();
    let child = child.compute_transform();
    let data = joint.as_ref();
    let a = parent.transform_point(data.local_anchor1());
    let b = child.transform_point(data.local_anchor2());
    let axis = parent.rotation * data.local_axis1();
    let (state, motor_enabled) = match joint {
        TypedJoint::RevoluteJoint(joint) => {
            let raw = joint.data.raw.as_revolute()?;
            let angle_rad = raw.angle(&parent.rotation.into(), &child.rotation.into());
            (ArticulationCoordinate::Revolute {
                angle_rad, limits_rad: joint.limits().map(|l| [l.min, l.max]),
            }, joint.motor().is_some())
        }
        TypedJoint::PrismaticJoint(joint) => (ArticulationCoordinate::Prismatic {
            travel_m: (b - a).dot(axis), limits_m: joint.limits().map(|l| [l.min, l.max]),
        }, joint.motor().is_some()),
        _ => return None,
    };
    Some(ArticulationSnapshot { parent_id: prop.id.clone(), registered,
        parent_anchor_world: a.to_array(), child_anchor_world: b.to_array(),
        axis_world: axis.to_array(), motor_enabled, state })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_observation_uses_parent_frame_and_does_not_hide_overtravel() {
        let prop = super::super::props::Prop { id: "rail".into(), loading: false, grasp_local: None };
        let parent = Transform::from_xyz(4., 2., -3.).with_rotation(Quat::from_rotation_y(0.7));
        let local_a = Vec3::new(0.4, 0.2, -0.3);
        let local_b = Vec3::new(-0.2, 0.1, 0.3);
        let joint: TypedJoint = PrismaticJointBuilder::new(Vec3::X)
            .local_anchor1(local_a).local_anchor2(local_b).limits([0., 2.]).build().into();
        for travel in [-0.02, 0., 0.72, 2., 2.03] {
            let child = Transform::from_translation(parent.transform_point(local_a)
                + parent.rotation * (Vec3::X * travel - local_b)).with_rotation(parent.rotation);
            let value = observe(&joint, (&prop, &parent.into()), &child.into(), false).unwrap();
            assert!(!value.registered);
            assert!(!value.motor_enabled);
            assert!(Vec3::from(value.axis_world).distance(parent.rotation * Vec3::X) < 1e-6);
            let ArticulationCoordinate::Prismatic { travel_m, limits_m } = value.state else { panic!("rail kind"); };
            assert_eq!(limits_m, Some([0., 2.]));
            assert!((travel_m-travel).abs() < 1e-6);
        }
    }

    #[test]
    fn hinge_observation_preserves_signed_angle_under_rotated_parent() {
        let prop = super::super::props::Prop { id: "frame".into(), loading: false, grasp_local: None };
        let parent = Transform::from_xyz(1., 0., 3.)
            .with_rotation(Quat::from_euler(EulerRot::XYZ, 0.2, 0.7, -0.3));
        let joint: TypedJoint = RevoluteJointBuilder::new(Vec3::Y).limits([-1.6, 0.]).build().into();
        for angle in [-1.7, -1., 0., 0.02] {
            let child = Transform::from_translation(parent.translation)
                .with_rotation(parent.rotation * Quat::from_rotation_y(angle));
            let value = observe(&joint, (&prop, &parent.into()), &child.into(), true).unwrap();
            assert!(value.registered);
            let ArticulationCoordinate::Revolute { angle_rad, limits_rad } = value.state else { panic!("hinge kind"); };
            assert_eq!(limits_rad, Some([-1.6, 0.]));
            assert!((angle_rad-angle).abs() < 1e-6);
        }
        let fixed: TypedJoint = FixedJointBuilder::new().build().into();
        assert!(observe(&fixed, (&prop, &parent.into()), &parent.into(), true).is_none());
    }
}
