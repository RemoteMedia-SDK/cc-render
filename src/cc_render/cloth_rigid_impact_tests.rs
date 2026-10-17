use super::*;
use crate::cc_render::cloth_solver::{sweep::moving::RigidClothSweep, FixedClothCollider};
use bevy::math::DQuat;
use bevy_rapier3d::rapier::parry::{math::Isometry, shape::SharedShape};

struct Fixture {
    points: [DVec3; 3],
    velocities: [DVec3; 3],
    masses: [f64; 3],
    hit: RigidSweepHit,
    body: RigidContactBody,
}

impl Fixture {
    fn dynamic() -> Self {
        let points = [
            DVec3::new(-0.4, 0.02, -0.2),
            DVec3::new(0.6, 0.02, -0.2),
            DVec3::new(0., 0.02, 0.8),
        ];
        let weights = [0.2, 0.3, 0.5];
        let q = DMat3::from_quat(DQuat::from_rotation_x(0.4));
        Self {
            points,
            velocities: [
                DVec3::new(0.2, -0.2, 0.3),
                DVec3::new(-0.2, -1.1, 0.),
                DVec3::new(0.1, -0.4, -0.2),
            ],
            masses: [0.3, 0.5, 0.8],
            hit: RigidSweepHit {
                fraction: 0.5,
                point: points.into_iter().zip(weights).map(|(p, w)| p * w).sum(),
                normal: DVec3::Y,
                separation: 0.001,
                weights,
            },
            body: RigidContactBody {
                center_of_mass: DVec3::new(-0.3, -0.4, 0.2),
                linear_velocity: DVec3::new(0.1, 0.6, -0.2),
                angular_velocity: DVec3::new(0.2, -0.3, 0.4),
                inverse_mass: DVec3::splat(0.5),
                inverse_inertia_sqrt: q
                    * DMat3::from_diagonal(DVec3::new(0.8, 1.2, 0.6))
                    * q.transpose(),
            },
        }
    }

    fn impact(&self) -> NormalImpact {
        self.hit
            .normal_impact(self.velocities, self.masses.map(f64::recip), &self.body)
            .unwrap()
    }

    fn apply(&mut self, impact: &NormalImpact) {
        for (v, dv) in self.velocities.iter_mut().zip(impact.cloth_delta_velocity) {
            *v += dv;
        }
        self.body.linear_velocity += impact.body_delta_linear_velocity;
        self.body.angular_velocity += impact.body_delta_angular_velocity;
    }

    fn relative_normal_velocity(&self) -> f64 {
        let cloth = self
            .velocities
            .into_iter()
            .zip(self.hit.weights)
            .map(|(v, w)| v * w)
            .sum::<DVec3>();
        (cloth
            - self.body.linear_velocity
            - self
                .body
                .angular_velocity
                .cross(self.hit.point - self.body.center_of_mass))
        .dot(self.hit.normal)
    }

    // Independent conservation accounting for the fully dynamic fixture.
    fn totals(&self) -> (DVec3, DVec3, f64) {
        let body_mass = self.body.inverse_mass.x.recip();
        let inertia =
            (self.body.inverse_inertia_sqrt * self.body.inverse_inertia_sqrt.transpose()).inverse();
        let body_momentum = self.body.linear_velocity * body_mass;
        let mut linear = body_momentum;
        let mut angular =
            self.body.center_of_mass.cross(body_momentum) + inertia * self.body.angular_velocity;
        let mut energy = 0.5 * body_mass * self.body.linear_velocity.length_squared()
            + 0.5
                * self
                    .body
                    .angular_velocity
                    .dot(inertia * self.body.angular_velocity);
        for i in 0..3 {
            let momentum = self.velocities[i] * self.masses[i];
            linear += momentum;
            angular += self.points[i].cross(momentum);
            energy += 0.5 * self.masses[i] * self.velocities[i].length_squared();
        }
        (linear, angular, energy)
    }
}

#[test]
fn normal_impact_conserves_linear_and_angular_momentum_and_dissipates_energy() {
    let mut f = Fixture::dynamic();
    let before = f.totals();
    let closing = f.relative_normal_velocity();
    assert!(closing < 0.);
    let impact = f.impact();
    assert!(impact.impulse.dot(f.hit.normal) > 0.);
    f.apply(&impact);
    let after = f.totals();
    assert!((after.0 - before.0).length() < 1e-12);
    assert!((after.1 - before.1).length() < 1e-12);
    assert!(f.relative_normal_velocity().abs() < 1e-12);
    assert!(after.2 < before.2);
    assert!((after.2 - before.2 - 0.5 * impact.impulse.dot(f.hit.normal) * closing).abs() < 1e-12);
    // Different vertex masses receive the contact's barycentric momentum shares.
    for i in 0..3 {
        assert!(
            (impact.cloth_delta_velocity[i] * f.masses[i] - impact.impulse * f.hit.weights[i])
                .length()
                < 1e-12
        );
    }
}

#[test]
fn separating_contact_does_not_pull_the_cloth_back() {
    let mut f = Fixture::dynamic();
    let body_contact_velocity = f.body.linear_velocity
        + f.body
            .angular_velocity
            .cross(f.hit.point - f.body.center_of_mass);
    f.velocities = [body_contact_velocity + f.hit.normal; 3];
    let impact = f.impact();
    assert_eq!(impact.impulse, DVec3::ZERO);
    assert_eq!(impact.cloth_delta_velocity, [DVec3::ZERO; 3]);
    assert_eq!(impact.body_delta_linear_velocity, DVec3::ZERO);
    assert_eq!(impact.body_delta_angular_velocity, DVec3::ZERO);
}

#[test]
fn observed_kinematic_motion_pushes_cloth_without_rewriting_body_velocity() {
    let mut f = Fixture::dynamic();
    f.body.inverse_mass = DVec3::ZERO;
    f.body.inverse_inertia_sqrt = DMat3::ZERO;
    let before_linear = f.body.linear_velocity;
    let before_angular = f.body.angular_velocity;
    let impact = f.impact();
    f.apply(&impact);
    assert_eq!(f.body.linear_velocity, before_linear);
    assert_eq!(f.body.angular_velocity, before_angular);
    assert!(f.relative_normal_velocity().abs() < 1e-12);
}

#[test]
fn locked_translation_can_resolve_an_off_center_impact_through_rotation() {
    let mut f = Fixture::dynamic();
    f.body.inverse_mass = DVec3::ZERO;
    f.body.inverse_inertia_sqrt = DMat3::from_diagonal(DVec3::Z);
    let impact = f.hit.normal_impact(f.velocities, [0.; 3], &f.body).unwrap();
    f.apply(&impact);
    assert_eq!(impact.body_delta_linear_velocity, DVec3::ZERO);
    assert_eq!(impact.body_delta_angular_velocity.x, 0.);
    assert_eq!(impact.body_delta_angular_velocity.y, 0.);
    assert!(impact.body_delta_angular_velocity.z.abs() > 0.);
    assert!(f.relative_normal_velocity().abs() < 1e-12);
}

#[test]
fn immovable_closing_contact_and_invalid_mass_fail_explicitly() {
    let mut f = Fixture::dynamic();
    f.body.inverse_mass = DVec3::ZERO;
    f.body.inverse_inertia_sqrt = DMat3::ZERO;
    assert!(matches!(
        f.hit.normal_impact(f.velocities, [0.; 3], &f.body),
        Err(ClothError::RigidContactUnresolved)
    ));
    assert!(matches!(
        f.hit.normal_impact(f.velocities, [-1., 1., 1.], &f.body),
        Err(ClothError::InvalidInput)
    ));
    f.body.inverse_inertia_sqrt = DMat3::from_diagonal(DVec3::splat(f64::NAN));
    assert!(matches!(
        f.hit.normal_impact(f.velocities, [1.; 3], &f.body),
        Err(ClothError::InvalidInput)
    ));
}

#[test]
fn swept_face_witness_drives_mass_weighted_two_way_impact() {
    let collider = FixedClothCollider::new(
        SharedShape::cuboid(0.01, 0.005, 0.01),
        Isometry::translation(0., -0.6, 0.),
        0.,
    )
    .unwrap();
    let translation = DVec3::Y * 1.2;
    let sweep = RigidClothSweep::new(&collider, DVec3::ZERO, translation, DVec3::ZERO).unwrap();
    let face = [
        DVec3::new(-0.1, 0., -0.1),
        DVec3::new(0.1, 0., -0.1),
        DVec3::new(0., 0., 0.1),
    ];
    let hit = sweep.triangle(face, face, 0.0005).unwrap().unwrap();
    let point = face
        .into_iter()
        .zip(hit.weights)
        .map(|(p, w)| p * w)
        .sum::<DVec3>();
    assert!((point - hit.point).length() < 1e-12);
    let mut f = Fixture {
        points: face,
        velocities: [DVec3::ZERO; 3],
        masses: [0.1, 0.15, 0.2],
        body: RigidContactBody {
            center_of_mass: collider.center + translation * hit.fraction,
            linear_velocity: translation * 60.,
            angular_velocity: DVec3::ZERO,
            inverse_mass: DVec3::splat(0.5),
            inverse_inertia_sqrt: DMat3::IDENTITY,
        },
        hit,
    };
    let before = f.totals();
    let impact = f.impact();
    f.apply(&impact);
    let after = f.totals();
    assert!(f.body.linear_velocity.y < 72.);
    assert!(f.relative_normal_velocity().abs() < 1e-10);
    assert!((after.0 - before.0).length() < 1e-10);
    assert!((after.1 - before.1).length() < 1e-10);
    assert!(after.2 < before.2);
}

#[test]
fn rotating_body_contact_transfers_angular_momentum_to_the_sheet() {
    let collider = FixedClothCollider::new(
        SharedShape::cuboid(1., 0.02, 0.02),
        Isometry::identity(),
        0.,
    )
    .unwrap();
    let rotation = DVec3::Z * std::f64::consts::PI;
    let sweep = RigidClothSweep::new(&collider, DVec3::ZERO, DVec3::ZERO, rotation).unwrap();
    let face = [
        DVec3::new(-0.05, 0.7, -0.05),
        DVec3::new(0.05, 0.7, -0.05),
        DVec3::new(0., 0.7, 0.05),
    ];
    let hit = sweep.triangle(face, face, 0.001).unwrap().unwrap();
    let turn = DMat3::from_quat(DQuat::from_scaled_axis(rotation * hit.fraction));
    let squared = collider.half_extents * collider.half_extents;
    let inertia = DVec3::new(
        squared.y + squared.z,
        squared.x + squared.z,
        squared.x + squared.y,
    ) * (2. / 3.);
    let inverse_sqrt = DMat3::from_diagonal(DVec3::from_array(
        inertia.to_array().map(|i| i.sqrt().recip()),
    ));
    let mut f = Fixture {
        points: face,
        velocities: [DVec3::ZERO; 3],
        masses: [0.1, 0.15, 0.2],
        body: RigidContactBody {
            center_of_mass: DVec3::ZERO,
            linear_velocity: DVec3::ZERO,
            angular_velocity: rotation * 60.,
            inverse_mass: DVec3::splat(0.5),
            inverse_inertia_sqrt: turn * inverse_sqrt * turn.transpose(),
        },
        hit,
    };
    let before = f.totals();
    let impact = f.impact();
    assert!(impact.body_delta_angular_velocity.z < 0.);
    f.apply(&impact);
    let after = f.totals();
    assert!(f.relative_normal_velocity().abs() < 1e-9);
    assert!((after.0 - before.0).length() < 1e-9);
    assert!((after.1 - before.1).length() < 1e-9);
    assert!(after.2 < before.2);
}
