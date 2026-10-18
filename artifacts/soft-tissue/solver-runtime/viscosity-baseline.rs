//! Isotropic finite-strain Kelvin-Voigt dissipation in reference volume.
//! Green strain is unchanged by time-dependent rigid rotation. Its rate is
//! paired with second Piola stress; no absolute world velocity is damped.
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

pub(super) fn green(f: DMat3) -> DMat3 {
    0.5 * (f.transpose() * f - DMat3::IDENTITY)
}

fn trace(a: DMat3) -> f64 {
    a.x_axis.x + a.y_axis.y + a.z_axis.z
}
fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

pub(super) struct Element {
    pub tet: usize,
    pub history_strain: DMat3,
    // Pa s divided by the effective differentiation interval, hence Pa.
    pub shear: f64,
    pub bulk: f64,
}

impl Element {
    pub fn energy(&self, tet: &Tet, x: &[DVec3]) -> f64 {
        let delta = green(tet.deformation(x)) - self.history_strain;
        let volume = trace(delta);
        let deviator = delta - DMat3::IDENTITY * (volume / 3.);
        tet.volume
            * (self.shear * material::squared_norm(deviator) + 0.5 * self.bulk * volume * volume)
    }

    /// Exact gradient and Gauss-Newton PSD tangent of the strain potential.
    pub fn differential(&self, tet: &Tet, x: &[DVec3]) -> ([DVec3; 4], [[DMat3; 4]; 4]) {
        let f = tet.deformation(x);
        let shape = material::nodal_gradient(tet.inverse_rest.transpose());
        let transformed = shape.map(|g| f * g);
        let delta = green(f) - self.history_strain;
        let lambda = self.bulk - 2. * self.shear / 3.;
        let stress = 2. * self.shear * delta + lambda * trace(delta) * DMat3::IDENTITY;
        let gradient = shape.map(|g| tet.volume * f * stress * g);
        let metric = f * f.transpose();
        let blocks = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                tet.volume
                    * (self.shear
                        * (shape[i].dot(shape[j]) * metric + outer(transformed[j], transformed[i]))
                        + lambda * outer(transformed[i], transformed[j]))
            })
        });
        (gradient, blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                shear,
                bulk,
            };
            let (gradient, hessian) = element.differential(tet, &x);
            let (rotated, _) = element.differential(tet, &moved);
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
            let de = 0.5 * (f.transpose() * df + df.transpose() * f);
            let volume = trace(de);
            let deviator = de - DMat3::IDENTITY * (volume / 3.);
            let expected = tet.volume
                * (2. * shear * material::squared_norm(deviator) + bulk * volume * volume);
            let measured: f64 = (0..4)
                .flat_map(|a| (0..4).map(move |b| d[a].dot(hessian[a][b] * d[b])))
                .sum();
            assert!((measured - expected).abs() < 1e-8 * expected.max(1.));
            // The previous and current orientations may differ arbitrarily.
            element.history_strain = green(f);
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
