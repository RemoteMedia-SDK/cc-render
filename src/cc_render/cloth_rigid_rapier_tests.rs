use super::*;
use bevy_rapier3d::rapier::{
    dynamics::{LockedAxes, MassProperties, RigidBody, RigidBodyBuilder},
    geometry::ColliderSet,
    na::{Isometry3, Point3, UnitQuaternion, Vector3},
};

fn vector(v: Vector3<f32>) -> DVec3 {
    DVec3::new(v.x as f64, v.y as f64, v.z as f64)
}

fn native_body(builder: RigidBodyBuilder) -> RigidBody {
    let mut body = builder
        .position(Isometry3::new(
            Vector3::new(0.2, -0.5, 0.3),
            Vector3::new(0.3, -0.2, 0.4),
        ))
        .additional_mass_properties(MassProperties::with_principal_inertia_frame(
            Point3::new(0.1, 0.2, -0.1),
            2.,
            Vector3::new(0.2, 0.3, 0.4),
            UnitQuaternion::from_scaled_axis(Vector3::new(-0.2, 0.4, 0.1)),
        ))
        .linvel(Vector3::new(0.3, 0.8, -0.2))
        .angvel(Vector3::new(0.4, -0.1, 0.3))
        .build();
    body.recompute_mass_properties_from_colliders(&ColliderSet::new());
    body
}

#[test]
fn snapshot_response_matches_native_rapier_point_impulses_and_locked_axes() {
    for builder in [
        RigidBodyBuilder::dynamic(),
        RigidBodyBuilder::dynamic()
            .locked_axes(LockedAxes::TRANSLATION_LOCKED_X | LockedAxes::ROTATION_LOCKED_Y),
        RigidBodyBuilder::kinematic_velocity_based(),
        RigidBodyBuilder::fixed(),
    ] {
        let mut body = native_body(builder);
        let snapshot = RigidContactBody::from_rapier(&body).unwrap();
        assert_eq!(
            snapshot.center_of_mass,
            vector(body.center_of_mass().coords)
        );
        if !body.is_dynamic() {
            assert_eq!(snapshot.inverse_mass, DVec3::ZERO);
            assert_eq!(snapshot.inverse_inertia_sqrt, DMat3::ZERO);
        }
        if body.is_fixed() {
            assert_eq!(snapshot.linear_velocity, DVec3::ZERO);
            assert_eq!(snapshot.angular_velocity, DVec3::ZERO);
        }
        let hit = RigidSweepHit {
            fraction: 0.4,
            point: DVec3::new(0.7, 0.1, -0.2),
            normal: DVec3::new(0.2, 0.9, -0.1).normalize(),
            separation: 0.001,
            weights: [0.2, 0.3, 0.5],
        };
        let impact = hit
            .normal_impact([-DVec3::Y; 3], [2., 3., 4.], &snapshot)
            .unwrap();
        let before_linear = vector(*body.linvel());
        let before_angular = vector(*body.angvel());
        let j = -impact.impulse;
        body.apply_impulse_at_point(
            Vector3::new(j.x as f32, j.y as f32, j.z as f32),
            Point3::new(hit.point.x as f32, hit.point.y as f32, hit.point.z as f32),
            true,
        );
        let linear_error =
            vector(*body.linvel()) - before_linear - impact.body_delta_linear_velocity;
        let angular_error =
            vector(*body.angvel()) - before_angular - impact.body_delta_angular_velocity;
        assert!(linear_error.length() < 2e-6, "{linear_error:?}");
        assert!(angular_error.length() < 2e-6, "{angular_error:?}");
    }
}
