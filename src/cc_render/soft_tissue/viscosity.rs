//! Isotropic Kelvin-Voigt dissipation in reference volume. Isochoric Green
//! strain and log volume separate shape and volume rates at finite strain.
//! Both are objective; no absolute world velocity is damped.
use super::*;
use bevy::math::DMat3;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Viscosity {
    pub shear_pa_s: f64,
    pub bulk_pa_s: f64,
}

impl Viscosity {
    pub(super) fn validate(self) -> Result<(), TissueError> {
        if [self.shear_pa_s, self.bulk_pa_s]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.)
        {
            return Err(TissueError::InvalidInput("viscosity"));
        }
        Ok(())
    }
}

pub(super) fn isochoric_green(f: DMat3) -> DMat3 {
    0.5 * (f.determinant().powf(-2. / 3.) * f.transpose() * f - DMat3::IDENTITY)
}

fn inner(a: DMat3, b: DMat3) -> f64 {
    a.x_axis.dot(b.x_axis) + a.y_axis.dot(b.y_axis) + a.z_axis.dot(b.z_axis)
}
fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

pub(super) struct Element {
    pub tet: usize,
    pub history_strain: DMat3,
    pub history_log_volume: f64,
    // Pa s divided by the effective differentiation interval, hence Pa.
    pub shear: f64,
    pub bulk: f64,
}

impl Element {
    pub fn energy(&self, tet: &Tet, x: &[DVec3]) -> f64 {
        let f = tet.deformation(x);
        let delta = isochoric_green(f) - self.history_strain;
        let volume = f.determinant().ln() - self.history_log_volume;
        tet.volume
            * (self.shear * material::squared_norm(delta) + 0.5 * self.bulk * volume * volume)
    }

    /// Exact gradient and Gauss-Newton PSD tangent of the strain potential.
    /// Tangent blocks touching inactive (fixed) nodes are omitted.
    pub fn differential(&self, tet: &Tet, x: &[DVec3], active: [bool; 4]) -> ([DVec3; 4], [[DMat3; 4]; 4]) {
        let f = tet.deformation(x);
        let shape = material::nodal_gradient(tet.inverse_rest.transpose());
        let transformed = shape.map(|g| f * g);
        let inverse_transpose = f.inverse().transpose();
        let volume_gradient = shape.map(|g| inverse_transpose * g);
        let c = f.transpose() * f;
        let scale = f.determinant().powf(-2. / 3.);
        let delta = 0.5 * (scale * c - DMat3::IDENTITY) - self.history_strain;
        let volume = f.determinant().ln() - self.history_log_volume;
        let stress = 2. * self.shear * scale * delta;
        let pressure = self.bulk * volume - inner(stress, c) / 3.;
        let gradient = std::array::from_fn(|i| {
            tet.volume * (f * stress * shape[i] + pressure * volume_gradient[i])
        });
        let metric = f * f.transpose();
        let coupled = shape.map(|g| f * c * g);
        let c_squared = material::squared_norm(c);
        // The tangent is symmetric. Assembly consumes the diagonal and upper
        // blocks; mirror the remaining six instead of recomputing them.
        let mut blocks = [[DMat3::ZERO; 4]; 4];
        for i in 0..4 {
            if !active[i] { continue; }
            for j in i..4 {
                if !active[j] { continue; }
                let block = tet.volume
                    * (self.shear
                        * scale
                        * scale
                        * (shape[i].dot(shape[j]) * metric
                            + outer(transformed[j], transformed[i])
                            - (2. / 3.)
                                * (outer(coupled[i], volume_gradient[j])
                                    + outer(volume_gradient[i], coupled[j]))
                            + (2. / 9.)
                                * c_squared
                                * outer(volume_gradient[i], volume_gradient[j]))
                        + self.bulk * outer(volume_gradient[i], volume_gradient[j]));
                blocks[i][j] = block;
                if i != j {
                    blocks[j][i] = block.transpose();
                }
            }
        }
        (gradient, blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_viscosity_does_not_resist_volume_preserving_shear() {
        let tissue = super::super::tests::cube(Frame::IDENTITY, false);
        let tet = &tissue.tets[0];
        let element = Element {
            tet: 0,
            history_strain: DMat3::ZERO,
            history_log_volume: 0.,
            shear: 0.,
            bulk: 1_250_000.,
        };
        for amount in [0.1, 0.5, 1.] {
            let f = DMat3::from_cols(DVec3::X, DVec3::Y + amount * DVec3::X, DVec3::Z);
            let x: Vec<_> = tissue.rest.iter().map(|p| f * *p).collect();
            assert!((tet.deformation(&x).determinant() - 1.).abs() < 1e-12);
            let energy = element.energy(tet, &x);
            assert!(
                energy < 1e-20,
                "isochoric shear {amount}: bulk energy {energy}"
            );
            let (gradient, _) = element.differential(tet, &x, [true; 4]);
            assert!(gradient.iter().all(|g| g.length() < 1e-10));
        }
    }

    #[test]
    fn finite_strain_viscosity_separates_dilation_from_distortion() {
        let tissue = super::super::tests::cube(Frame::IDENTITY, false);
        let tet = &tissue.tets[0];
        let f = DMat3::from_cols(DVec3::X, DVec3::Y + 0.5 * DVec3::X, DVec3::Z);
        let mut element = Element {
            tet: 0,
            history_strain: isochoric_green(f),
            history_log_volume: 0.,
            shear: 3750.,
            bulk: 0.,
        };
        for scale in [0.8_f64, 1.2, 2.] {
            let x: Vec<_> = tissue.rest.iter().map(|p| scale * f * *p).collect();
            // Scaling an already distorted state changes only its volume.
            assert!(element.energy(tet, &x) < 1e-24);
            let (gradient, _) = element.differential(tet, &x, [true; 4]);
            assert!(gradient.iter().all(|g| g.length() < 1e-10));
        }
        element.shear = 0.;
        element.bulk = 1_250_000.;
        for scale in [0.8_f64, 1.2, 2.] {
            let x: Vec<_> = tissue.rest.iter().map(|p| scale * f * *p).collect();
            let log_volume = 3. * scale.ln();
            let expected = 0.5 * tet.volume * element.bulk * log_volume.powi(2);
            assert!((element.energy(tet, &x) - expected).abs() < 1e-10);
            let (gradient, _) = element.differential(tet, &x, [true; 4]);
            // Nodal virtual work reconstructs Cauchy stress independently.
            let stress: DMat3 = (0..4)
                .map(|i| outer(gradient[i], x[tet.ids[i]]))
                .fold(DMat3::ZERO, |a, b| a + b)
                / (tet.volume * scale.powi(3));
            let hydrostatic = DMat3::IDENTITY * (element.bulk * log_volume / scale.powi(3));
            assert!(material::squared_norm(stress - hydrostatic).sqrt() < 1e-8);
        }
    }

    #[test]
    fn volume_rate_history_uses_bdf2_strain_samples_and_restarts_on_large_step_growth() {
        let mut tissue = super::super::tests::cube(Frame::IDENTITY, false);
        tissue.material.time_integration = TimeIntegration::Bdf2;
        tissue.material.strain_damping_per_second = 0.;
        tissue.material.viscosity = Some(Viscosity {
            shear_pa_s: 15.,
            bulk_pa_s: 5000.,
        });
        tissue.positions = tissue.rest.iter().map(|p| 1.1 * *p).collect();
        tissue.previous = Some(timestep::History {
            positions: tissue.rest.iter().map(|p| 0.9 * *p).collect(),
            velocities: vec![DVec3::ZERO; tissue.rest.len()],
            dt: 1. / 120.,
        });
        for ratio in [0.5, 1., 1.75, 3.] {
            let interval = ratio / 120.;
            let step = timestep::IntegrationStep::new(&tissue, interval, DVec3::ZERO);
            let (current, older, effective) = if ratio <= 2. {
                let denominator = 1. + 2. * ratio;
                (
                    (1. + ratio).powi(2) / denominator,
                    -ratio * ratio / denominator,
                    interval * (1. + ratio) / denominator,
                )
            } else {
                (1., 0., interval)
            };
            let expected = 3. * (current * 1.1_f64.ln() + older * 0.9_f64.ln());
            for element in &step.damping.elements {
                assert!((element.history_log_volume - expected).abs() < 1e-12);
                assert!(material::squared_norm(element.history_strain) < 1e-24);
                assert!((element.bulk - 5000. / effective).abs() < 1e-8);
            }
        }
    }

    #[test]
    fn material_viscosity_has_objective_energy_exact_forces_and_a_psd_strain_tangent() {
        let tissue = super::super::tests::cube(Frame::IDENTITY, false);
        let tet = &tissue.tets[0];
        let f = DMat3::from_cols(
            DVec3::new(1.1, 0.03, 0.),
            DVec3::new(0.2, 0.9, 0.04),
            DVec3::new(0.01, 0., 1.03),
        );
        let x: Vec<_> = tissue.rest.iter().map(|p| f * *p).collect();
        let q = DQuat::from_rotation_x(0.7) * DQuat::from_rotation_y(-0.3);
        let moved: Vec<_> = x
            .iter()
            .map(|p| q * *p + DVec3::new(1., -2., 0.5))
            .collect();
        for (shear, bulk) in [(3750., 1250000.), (3750., 0.), (0., 1250000.)] {
            let mut element = Element {
                tet: 0,
                history_strain: DMat3::ZERO,
                history_log_volume: 0.,
                shear,
                bulk,
            };
            let (gradient, hessian) = element.differential(tet, &x, [true; 4]);
            let (rotated, _) = element.differential(tet, &moved, [true; 4]);
            assert!((element.energy(tet, &x) - element.energy(tet, &moved)).abs() < 1e-10);
            for a in 0..4 {
                assert!(rotated[a].distance(q * gradient[a]) < 1e-9);
                for axis in 0..3 {
                    let mut plus = x.clone();
                    let mut minus = x.clone();
                    plus[tet.ids[a]][axis] += 1e-7;
                    minus[tet.ids[a]][axis] -= 1e-7;
                    let finite = (element.energy(tet, &plus) - element.energy(tet, &minus)) / 2e-7;
                    assert!(
                        (finite - gradient[a][axis]).abs() < 1e-5,
                        "{finite} {:?}",
                        gradient[a]
                    );
                }
            }
            let d = [
                DVec3::new(0.1, 0.2, 0.3),
                DVec3::new(0.3, -0.1, 0.2),
                DVec3::new(-0.2, 0.4, 0.1),
                DVec3::new(0.2, 0.3, -0.4),
            ];
            let df = material::edges(d) * tet.inverse_rest;
            // Independently differentiate the strain coordinates numerically,
            // rather than duplicating the analytic block tangent expression.
            let plus = f + 1e-6 * df;
            let minus = f - 1e-6 * df;
            let de = (isochoric_green(plus) - isochoric_green(minus)) / 2e-6;
            let volume = (plus.determinant().ln() - minus.determinant().ln()) / 2e-6;
            let expected =
                tet.volume * (2. * shear * material::squared_norm(de) + bulk * volume * volume);
            let measured: f64 = (0..4)
                .flat_map(|a| (0..4).map(move |b| d[a].dot(hessian[a][b] * d[b])))
                .sum();
            assert!((measured - expected).abs() < 1e-8 * expected.max(1.));
            // The previous and current orientations may differ arbitrarily.
            element.history_strain = isochoric_green(f);
            element.history_log_volume = f.determinant().ln();
            assert!(element.energy(tet, &moved) < 1e-24);
        }
    }

    #[test]
    fn invalid_or_double_counted_viscosity_is_rejected() {
        let mut material = super::super::tests::cube(Frame::IDENTITY, false).material;
        material.viscosity = Some(Viscosity {
            shear_pa_s: 10.,
            bulk_pa_s: 40.,
        });
        assert!(material.validate().is_err());
        material.strain_damping_per_second = 0.;
        material.validate().unwrap();
        for value in [-1., f64::INFINITY, f64::NAN] {
            material.viscosity.as_mut().unwrap().bulk_pa_s = value;
            assert!(material.validate().is_err());
        }
    }
}
