//! Sparse implicit elasticity. The global solve retains the
//! coupling between neighbouring elements that local XPBD sweeps leave behind.
use super::*;
use bevy::math::DMat3;
static NONLINEAR: std::sync::Mutex<Vec<Vec<f64>>> = std::sync::Mutex::new(Vec::new());
static LINEAR: std::sync::Mutex<Vec<(usize,f64)>> = std::sync::Mutex::new(Vec::new());
pub fn profile() { eprintln!("{}",serde_json::json!({"nonlinear_force_norms_n":*NONLINEAR.lock().unwrap(),"linear_iterations_and_relative_residual":*LINEAR.lock().unwrap()})); }


struct Coupling {
    nodes: [usize; 2],
    block: DMat3,
}

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

fn dot(a: &[DVec3], b: &[DVec3]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a.dot(*b)).sum()
}

struct System {
    couplings: Vec<Coupling>,
    diagonal: Vec<DMat3>,
    inverse_diagonal: Vec<DMat3>,
    gradient: Vec<DVec3>,
    row_offsets: Vec<usize>,
}

impl System {
    fn multiply(&self, input: &[DVec3]) -> Vec<DVec3> {
        let mut output: Vec<_> = input
            .iter()
            .zip(&self.diagonal)
            .map(|(x, m)| *m * *x)
            .collect();
        for coupling in &self.couplings {
            let [a, b] = coupling.nodes;
            output[a] += coupling.block * input[b];
            output[b] += coupling.block.transpose() * input[a];
        }
        output
    }

    fn direction(&self) -> Vec<DVec3> {
        let mut result = vec![DVec3::ZERO; self.gradient.len()];
        let mut residual: Vec<_> = self.gradient.iter().map(|g| -*g).collect();
        let precondition = |r: &[DVec3]| -> Vec<_> {
            let mut out=r.to_vec();
            for i in 0..out.len() {
                out[i]=self.inverse_diagonal[i]*out[i];
                let value=out[i];
                for e in &self.couplings[self.row_offsets[i]..self.row_offsets[i+1]] {
                    out[e.nodes[1]]-=e.block.transpose()*value;
                }
            }
            for i in (0..out.len()).rev() {
                let correction:DVec3=self.couplings[self.row_offsets[i]..self.row_offsets[i+1]].iter()
                    .map(|e|e.block*out[e.nodes[1]]).sum();
                out[i]-=self.inverse_diagonal[i]*correction;
            }
            out
        };
        let mut z = precondition(&residual);
        let mut direction = z.clone();
        let mut rz = dot(&residual, &z);
        let threshold = dot(&residual, &residual) * 1e-14;
        let mut iterations=0;
        for _ in 0..128 {
            if dot(&residual, &residual) <= threshold || rz <= 1e-30 {
                break;
            }
            iterations+=1;
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
        let actual:Vec<_>=self.multiply(&result).into_iter().zip(&self.gradient).map(|(a,b)|a+*b).collect();
        LINEAR.lock().unwrap().push((iterations,(dot(&actual,&actual)/dot(&self.gradient,&self.gradient)).sqrt()));
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
            let mut system = self.linearize(&x, &self.positions, self.frame, f64::INFINITY, &[]);
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

    fn linearize(
        &self,
        x: &[DVec3],
        predicted: &[DVec3],
        frame: Frame,
        dt: f64,
        contacts: &[PlaneContact],
    ) -> System {
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
        let mut couplings: Vec<_> = self
            .edges
            .iter()
            .map(|nodes| Coupling {
                nodes: *nodes,
                block: DMat3::ZERO,
            })
            .collect();
        for tet in &self.tets {
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
            for a in 0..4 {
                for b in a + 1..4 {
                    let mut nodes = [tet.ids[a], tet.ids[b]];
                    let mut block = DMat3::IDENTITY * (shear_weight * shape[a].dot(shape[b]))
                        + volume_weight * outer(volume_gradient[a], volume_gradient[b]);
                    if nodes[0] > nodes[1] {
                        nodes.swap(0, 1);
                        block = block.transpose();
                    }
                    let index = self
                        .edges
                        .binary_search(&nodes)
                        .expect("tet edge is indexed");
                    couplings[index].block += block;
                }
            }
        }
        for contact in contacts {
            let depth = contact.depth(x, &self.tets);
            if depth == 0.0 {
                continue;
            }
            let ids = self.tets[contact.point.tetrahedron].ids;
            let derivatives = contact
                .point
                .barycentric
                .map(|w| contact.normal * (contact.point.influence * w));
            let stiffness = contact.stiffness_n_per_m;
            for a in 0..4 {
                gradient[ids[a]] -= stiffness * depth * derivatives[a];
                diagonal[ids[a]] += stiffness * outer(derivatives[a], derivatives[a]);
                for b in a + 1..4 {
                    let mut nodes = [ids[a], ids[b]];
                    nodes.sort_unstable();
                    let index = self
                        .edges
                        .binary_search(&nodes)
                        .expect("contact tet edge is indexed");
                    // Both derivatives are parallel to the same normal, so
                    // this block is symmetric even when node order is reversed.
                    couplings[index].block += stiffness * outer(derivatives[a], derivatives[b]);
                }
            }
        }
        // Eliminate fixed degrees of freedom from the displacement solve.
        couplings.retain(|c| c.nodes.iter().all(|i| self.inverse_mass[*i] > 0.0));
        let inverse_diagonal = diagonal
            .iter_mut()
            .enumerate()
            .map(|(i, d)| {
                if self.inverse_mass[i] == 0.0 {
                    gradient[i] = DVec3::ZERO;
                    *d = DMat3::ZERO;
                    DMat3::ZERO
                } else {
                    d.inverse()
                }
            })
            .collect();
        let row_offsets=(0..=diagonal.len()).map(|i|couplings.partition_point(|c|c.nodes[0]<i)).collect();
        System {
            row_offsets,
            couplings,
            diagonal,
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
        contacts: &[PlaneContact],
    ) -> Result<(), TissueError> {
        let mut residuals=Vec::new();
        for _ in 0..self.material.iterations {
            let system = self.linearize(x, predicted, frame, dt, contacts);
            residuals.push(dot(&system.gradient,&system.gradient).sqrt());
            if dot(&system.gradient, &system.gradient) < 1e-16 {
                break;
            }
            let delta = system.direction();
            let slope = dot(&system.gradient, &delta);
            *x = self.descend(x, &delta, slope, |x| {
                self.objective(x, predicted, frame, dt)
                    + contacts
                        .iter()
                        .map(|c| c.energy(x, &self.tets))
                        .sum::<f64>()
            })?;
        }
        NONLINEAR.lock().unwrap().push(residuals);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_contact_gradient_and_hessian_match_finite_differences() {
        let tissue = super::super::tests::cube(Frame::IDENTITY, true);
        let mut x = tissue.positions.clone();
        let mut contact = PlaneContact {
            point: crate::cc_render::soft_tissue::EmbeddedPoint {
                tetrahedron: 0,
                barycentric: [0.1, 0.2, 0.3, 0.4],
                influence: 0.65,
                offset: DVec3::new(0.02, 0.003, -0.01),
            },
            surface_point: DVec3::ZERO,
            normal: DVec3::new(0.4, 1.0, 0.1).normalize(),
            stiffness_n_per_m: 5000.0,
        };
        contact.surface_point = contact.point.position(&x, &tissue.tets) + 0.002 * contact.normal;
        let dt = 1.0 / 240.0;
        let system = tissue.linearize(&x, &tissue.positions, Frame::IDENTITY, dt, &[contact]);
        let no_contact = tissue.linearize(&x, &tissue.positions, Frame::IDENTITY, dt, &[]);
        let gradient: Vec<_> = system
            .gradient
            .iter()
            .zip(&no_contact.gradient)
            .map(|(a, b)| *a - *b)
            .collect();
        let direction: Vec<_> = tissue
            .inverse_mass
            .iter()
            .map(|w| {
                if *w > 0.0 {
                    DVec3::new(0.3, -0.2, 0.1)
                } else {
                    DVec3::ZERO
                }
            })
            .collect();
        let product: Vec<_> = system
            .multiply(&direction)
            .iter()
            .zip(no_contact.multiply(&direction))
            .map(|(a, b)| *a - b)
            .collect();
        for node in [1, 3, 5, 7] {
            for axis in 0..3 {
                let eps = 1e-7;
                x[node][axis] += eps;
                let plus = contact.energy(&x, &tissue.tets);
                x[node][axis] -= 2.0 * eps;
                let minus = contact.energy(&x, &tissue.tets);
                x[node][axis] += eps;
                assert!(((plus - minus) / (2.0 * eps) - gradient[node][axis]).abs() < 1e-7);
            }
        }
        let contact_gradient = |x: &[DVec3]| {
            let a = tissue.linearize(x, &tissue.positions, Frame::IDENTITY, dt, &[contact]);
            let b = tissue.linearize(x, &tissue.positions, Frame::IDENTITY, dt, &[]);
            a.gradient
                .iter()
                .zip(b.gradient)
                .map(|(a, b)| *a - b)
                .collect::<Vec<_>>()
        };
        let eps = 1e-6;
        let plus = contact_gradient(
            &x.iter()
                .zip(&direction)
                .map(|(x, d)| *x + eps * *d)
                .collect::<Vec<_>>(),
        );
        let minus = contact_gradient(
            &x.iter()
                .zip(&direction)
                .map(|(x, d)| *x - eps * *d)
                .collect::<Vec<_>>(),
        );
        for node in 0..x.len() {
            assert!(((plus[node] - minus[node]) / (2.0 * eps)).distance(product[node]) < 1e-7);
        }
        assert!(dot(&direction, &product) > 0.0);
    }

    #[test]
    fn objective_gradient_and_sparse_system_are_consistent() {
        let mut tissue = super::super::tests::cube(Frame::IDENTITY, true);
        tissue.attachments[7] = Attachment::Compliant {
            compliance_m_per_n: 0.001,
        };
        let mut x = tissue.positions.clone();
        x[5] += DVec3::new(0.001, -0.002, 0.0003);
        let predicted: Vec<_> = x.iter().map(|p| *p + DVec3::Y * 0.0001).collect();
        let dt = 1.0 / 240.0;
        let system = tissue.linearize(&x, &predicted, Frame::IDENTITY, dt, &[]);
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
        // Independent element-wise product checks sparse assembly and block
        // transposes, including the compliant node and fixed-node elimination.
        let mut element_product: Vec<_> = a
            .iter()
            .zip(&tissue.masses)
            .map(|(v, mass)| *v * (*mass / (dt * dt)))
            .collect();
        element_product[7] += a[7] / 0.001;
        for tet in &tissue.tets {
            let f = tet.deformation(&x);
            let shape = material::nodal_gradient(tet.inverse_rest.transpose());
            let cofactor = DMat3::from_cols(
                f.y_axis.cross(f.z_axis),
                f.z_axis.cross(f.x_axis),
                f.x_axis.cross(f.y_axis),
            );
            let volume = shape.map(|g| cofactor * g);
            let df: DMat3 = (0..4).map(|k| outer(a[tet.ids[k]], shape[k])).sum();
            let dj: f64 = (0..4).map(|k| a[tet.ids[k]].dot(volume[k])).sum();
            for k in 0..4 {
                element_product[tet.ids[k]] +=
                    2.0 * tissue.material.shear_pa * tet.volume * df * shape[k]
                        + tissue.material.lambda() * tet.volume * dj * volume[k];
            }
        }
        for i in 0..ha.len() {
            if tissue.inverse_mass[i] == 0.0 {
                element_product[i] = DVec3::ZERO;
            }
            assert!(ha[i].distance(element_product[i]) < 1e-10 * (1.0 + ha[i].length()));
        }
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
