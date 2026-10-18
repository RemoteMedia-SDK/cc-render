//! Matrix-free implicit elasticity experiment. The global solve retains the
//! coupling between neighbouring elements that local XPBD sweeps leave behind.
use super::*;
use bevy::math::DMat3;

struct Element {
    ids: [usize; 4],
    shape: [DVec3; 4],
    volume_gradient: [DVec3; 4],
    shear_weight: f64,
    volume_weight: f64,
}

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

fn dot(a: &[DVec3], b: &[DVec3]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a.dot(*b)).sum()
}

struct System {
    elements: Vec<Element>,
    diagonal_mass: Vec<f64>,
    inverse_diagonal: Vec<DMat3>,
    gradient: Vec<DVec3>,
}

impl System {
    fn multiply(&self, input: &[DVec3]) -> Vec<DVec3> {
        let mut output: Vec<_> = input
            .iter()
            .zip(&self.diagonal_mass)
            .map(|(x, m)| *x * *m)
            .collect();
        for e in &self.elements {
            let df: DMat3 = (0..4).map(|k| outer(input[e.ids[k]], e.shape[k])).sum();
            let dj: f64 = (0..4)
                .map(|k| input[e.ids[k]].dot(e.volume_gradient[k]))
                .sum();
            for k in 0..4 {
                output[e.ids[k]] +=
                    e.shear_weight * df * e.shape[k] + e.volume_weight * dj * e.volume_gradient[k];
            }
        }
        for (value, inverse) in output.iter_mut().zip(&self.inverse_diagonal) {
            if *inverse == DMat3::ZERO {
                *value = DVec3::ZERO;
            }
        }
        output
    }

    fn direction(&self) -> Vec<DVec3> {
        let mut result = vec![DVec3::ZERO; self.gradient.len()];
        let mut residual: Vec<_> = self.gradient.iter().map(|g| -*g).collect();
        let precondition = |r: &[DVec3]| -> Vec<_> {
            r.iter()
                .zip(&self.inverse_diagonal)
                .map(|(r, d)| *d * *r)
                .collect()
        };
        let mut z = precondition(&residual);
        let mut direction = z.clone();
        let mut rz = dot(&residual, &z);
        let threshold = dot(&residual, &residual) * 1e-14;
        for _ in 0..128 {
            if dot(&residual, &residual) <= threshold || rz <= 1e-30 {
                break;
            }
            let product = self.multiply(&direction);
            let alpha = rz / dot(&direction, &product);
            for i in 0..result.len() {
                result[i] += alpha * direction[i];
                residual[i] -= alpha * product[i];
            }
            z = precondition(&residual);
            let next_rz = dot(&residual, &z);
            let beta = next_rz / rz;
            for i in 0..result.len() {
                direction[i] = z[i] + beta * direction[i];
            }
            rz = next_rz;
        }
        result
    }
}

impl Tissue {
    /// Offline authoring/reference solve. Does not advance playback time and
    /// must never be used to reset moving tissue during runtime.
    pub fn equilibrate(
        &mut self,
        gravity: DVec3,
        max_iterations: usize,
    ) -> Result<f64, TissueError> {
        if !gravity.is_finite() || !(1..=4096).contains(&max_iterations) {
            return Err(TissueError::InvalidInput("equilibrium parameters"));
        }
        let mut x = self.positions.clone();
        let objective = |x: &[DVec3]| {
            self.objective(x, &self.positions, self.frame, f64::INFINITY)
                - x.iter()
                    .zip(&self.positions)
                    .zip(&self.masses)
                    .map(|((x, p), m)| m * gravity.dot(*x - *p))
                    .sum::<f64>()
        };
        for _ in 0..max_iterations {
            let mut system = self.linearize(&x, &self.positions, self.frame, f64::INFINITY);
            for (i, g) in system.gradient.iter_mut().enumerate() {
                if self.inverse_mass[i] > 0.0 {
                    *g -= self.masses[i] * gravity;
                }
            }
            let residual = system
                .gradient
                .iter()
                .map(|g| g.length())
                .fold(0.0, f64::max);
            if residual < 2e-5 {
                self.positions = x;
                self.velocities.fill(DVec3::ZERO);
                return Ok(residual);
            }
            let delta = system.direction();
            x = self.descend(&x, &delta, dot(&system.gradient, &delta), &objective)?;
        }
        Err(TissueError::InvalidInput("equilibrium did not converge"))
    }

    fn descend(
        &self,
        x: &[DVec3],
        delta: &[DVec3],
        slope: f64,
        objective: impl Fn(&[DVec3]) -> f64,
    ) -> Result<Vec<DVec3>, TissueError> {
        let energy = objective(x);
        let mut scale = 1.0;
        for _ in 0..20 {
            let candidate: Vec<_> = x.iter().zip(delta).map(|(x, d)| *x + scale * *d).collect();
            if self.validate_positions(&candidate).is_ok()
                && objective(&candidate) <= energy + 1e-4 * scale * slope + 1e-15
            {
                return Ok(candidate);
            }
            scale *= 0.5;
        }
        Err(TissueError::InvalidInput("implicit material descent"))
    }

    fn linearize(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> System {
        let mut diagonal_mass: Vec<_> = self.masses.iter().map(|m| *m / (dt * dt)).collect();
        let mut gradient: Vec<_> = x
            .iter()
            .zip(predicted)
            .zip(&diagonal_mass)
            .map(|((x, p), m)| *m * (*x - *p))
            .collect();
        for (i, attachment) in self.attachments.iter().enumerate() {
            if let Attachment::Compliant { compliance_m_per_n } = attachment {
                let stiffness = 1.0 / compliance_m_per_n;
                diagonal_mass[i] += stiffness;
                gradient[i] += stiffness * (x[i] - frame.point(self.attachment_positions[i]));
            }
        }
        let mut diagonal: Vec<_> = diagonal_mass.iter().map(|m| DMat3::IDENTITY * *m).collect();
        let elements = self
            .tets
            .iter()
            .map(|tet| {
                let f = tet.deformation(x);
                let shape = material::nodal_gradient(tet.inverse_rest.transpose());
                let cofactor = DMat3::from_cols(
                    f.y_axis.cross(f.z_axis),
                    f.z_axis.cross(f.x_axis),
                    f.x_axis.cross(f.y_axis),
                );
                let volume_gradient = shape.map(|g| cofactor * g);
                let shear_weight = 2.0 * self.material.shear_pa * tet.volume;
                let volume_weight = self.material.lambda() * tet.volume;
                let shear_stress = shear_weight * (f - material::polar_rotation(f));
                let pressure = volume_weight * (f.determinant() - 1.0);
                for k in 0..4 {
                    let i = tet.ids[k];
                    gradient[i] += shear_stress * shape[k] + pressure * volume_gradient[k];
                    diagonal[i] += DMat3::IDENTITY * (shear_weight * shape[k].length_squared())
                        + volume_weight * outer(volume_gradient[k], volume_gradient[k]);
                }
                Element {
                    ids: tet.ids,
                    shape,
                    volume_gradient,
                    shear_weight,
                    volume_weight,
                }
            })
            .collect();
        let inverse_diagonal = diagonal
            .into_iter()
            .enumerate()
            .map(|(i, d)| {
                if self.inverse_mass[i] == 0.0 {
                    gradient[i] = DVec3::ZERO;
                    DMat3::ZERO
                } else {
                    d.inverse()
                }
            })
            .collect();
        System {
            elements,
            diagonal_mass,
            inverse_diagonal,
            gradient,
        }
    }

    fn objective(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> f64 {
        let inertial: f64 = x
            .iter()
            .zip(predicted)
            .zip(&self.masses)
            .map(|((x, p), m)| 0.5 * m * x.distance_squared(*p) / (dt * dt))
            .sum();
        let elastic: f64 = self
            .tets
            .iter()
            .map(|t| t.energy(x, self.material.shear_pa, self.material.lambda()))
            .sum();
        let attachments: f64 = self
            .attachments
            .iter()
            .enumerate()
            .map(|(i, a)| match a {
                Attachment::Compliant { compliance_m_per_n } => {
                    0.5 * x[i].distance_squared(frame.point(self.attachment_positions[i]))
                        / compliance_m_per_n
                }
                _ => 0.0,
            })
            .sum();
        inertial + elastic + attachments
    }

    pub(super) fn solve_implicit(
        &self,
        x: &mut Vec<DVec3>,
        predicted: &[DVec3],
        frame: Frame,
        dt: f64,
    ) -> Result<(), TissueError> {
        for _ in 0..self.material.iterations {
            let system = self.linearize(x, predicted, frame, dt);
            if dot(&system.gradient, &system.gradient) < 1e-16 {
                break;
            }
            let delta = system.direction();
            let slope = dot(&system.gradient, &delta);
            *x = self.descend(x, &delta, slope, |x| {
                self.objective(x, predicted, frame, dt)
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objective_gradient_and_matrix_free_system_are_consistent() {
        let mut tissue = super::super::tests::cube(Frame::IDENTITY, true);
        tissue.attachments[7] = Attachment::Compliant {
            compliance_m_per_n: 0.001,
        };
        let mut x = tissue.positions.clone();
        x[5] += DVec3::new(0.001, -0.002, 0.0003);
        let predicted: Vec<_> = x.iter().map(|p| *p + DVec3::Y * 0.0001).collect();
        let dt = 1.0 / 240.0;
        let system = tissue.linearize(&x, &predicted, Frame::IDENTITY, dt);
        for node in [1, 3, 5, 7] {
            for axis in 0..3 {
                let eps = 1e-7;
                x[node][axis] += eps;
                let plus = tissue.objective(&x, &predicted, Frame::IDENTITY, dt);
                x[node][axis] -= 2.0 * eps;
                let minus = tissue.objective(&x, &predicted, Frame::IDENTITY, dt);
                x[node][axis] += eps;
                assert!(((plus - minus) / (2.0 * eps) - system.gradient[node][axis]).abs() < 1e-6);
            }
        }
        let a: Vec<_> = system.gradient.clone();
        let b: Vec<_> = a.iter().map(|v| DVec3::new(v.z, v.x, v.y)).collect();
        let ha = system.multiply(&a);
        let hb = system.multiply(&b);
        assert!(dot(&a, &ha) > 0.0);
        assert!((dot(&a, &hb) - dot(&b, &ha)).abs() < 1e-7);
        let direction = system.direction();
        let product = system.multiply(&direction);
        let residual: Vec<_> = product.iter().zip(&a).map(|(p, g)| *p + *g).collect();
        assert!(dot(&residual, &residual) < dot(&a, &a) * 1e-12);
    }

    #[test]
    fn offline_equilibrium_satisfies_force_balance_without_advancing_motion() {
        let mut tissue = super::super::tests::cube(Frame::IDENTITY, true);
        let gravity = DVec3::NEG_Y * 9.81;
        let force_error = tissue.equilibrate(gravity, 512).unwrap();
        assert!(force_error < 2e-5);
        assert!(tissue.positions[7].y < tissue.rest[7].y - 0.001);
        let before = tissue.positions.clone();
        for _ in 0..120 {
            tissue.step(1.0 / 240.0, Frame::IDENTITY, gravity).unwrap();
        }
        assert!(tissue
            .positions
            .iter()
            .zip(&before)
            .all(|(a, b)| a.distance(*b) < 1e-5));
    }
}
