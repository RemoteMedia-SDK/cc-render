//! Sparse implicit elasticity. The global solve retains the
//! coupling between neighbouring elements that local XPBD sweeps leave behind.
use super::*;
use bevy::math::DMat3;
use super::timestep::Dissipation;

#[cfg(test)]
#[path = "pressure_tests.rs"]
mod pressure_tests;

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

fn nonlinear_limits(dt: f64, integration: TimeIntegration) -> (f64, f64) {
    if integration == TimeIntegration::ImplicitEuler {
        // Retain the established first-order solver's convergence policy.
        return (1e-4, 1e-7);
    }
    // BDF2 uses this effective differentiation interval for both x and v.
    // Since a position error contributes error/dt to velocity, scale its
    // budget as dt^4 (velocity dt^3). The force budget scales as dt^2.
    // Cap at the 120 Hz BDF2 budgets; numerical floors avoid demanding
    // arbitrarily small residuals during fractional/retried steps.
    let scale = (dt * 180.0).min(1.0);
    ((1e-3 * scale.powi(2)).max(1e-8), (1e-6 * scale.powi(4)).max(1e-12))
}

struct System<'a> {
    pattern: &'a super::preconditioner::Pattern,
    couplings: Vec<Coupling>,
    diagonal: Vec<DMat3>,
    inverse_diagonal: Vec<DMat3>,
    gradient: Vec<DVec3>,
}

impl<'a> System<'a> {
    fn add_element(&mut self, tet: &Tet, force: [DVec3; 4], blocks: [[DMat3; 4]; 4]) {
        for a in 0..4 {
            self.gradient[tet.ids[a]] += force[a];
            self.diagonal[tet.ids[a]] += blocks[a][a];
        }
        for (index, [a, b]) in tet.edge_indices.iter().zip(material::EDGE_PAIRS) {
            self.couplings[*index].block += if tet.ids[a] > tet.ids[b] {
                blocks[a][b].transpose()
            } else {
                blocks[a][b]
            };
        }
    }

    fn eliminate_fixed(&mut self, inverse_mass: &[f64]) {
        self.couplings.retain(|c| c.nodes.iter().all(|i| inverse_mass[*i] > 0.0));
        let gradient = &mut self.gradient;
        self.inverse_diagonal = self.diagonal.iter_mut().enumerate().map(|(i, d)| {
            if inverse_mass[i] == 0.0 {
                gradient[i] = DVec3::ZERO;
                *d = DMat3::ZERO;
                DMat3::ZERO
            } else {
                d.inverse()
            }
        }).collect();
    }

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

    fn direction(&self) -> Result<Vec<DVec3>, TissueError> {
        self.direction_cached(&mut None)
    }

    fn factor(&self) -> Option<super::preconditioner::Factor<'a>> {
        super::preconditioner::Factor::new(self.pattern, &self.diagonal, |i| self.couplings[i].block)
    }

    fn direction_cached(
        &self,
        cache: &mut Option<super::preconditioner::Factor<'a>>,
    ) -> Result<Vec<DVec3>, TissueError> {
        let reused = cache.is_some();
        if !reused {
            *cache = self.factor();
        }
        match self.direction_preconditioned(cache.as_ref()) {
            // A changing tangent can make an older factor ineffective. Retry
            // with the current matrix before reporting a linear failure.
            Err(TissueError::LinearSolveFailed(_)) if reused => {
                *cache = self.factor();
                self.direction_preconditioned(cache.as_ref())
            }
            result => result,
        }
    }

    fn direction_preconditioned(
        &self,
        factor: Option<&super::preconditioner::Factor<'_>>,
    ) -> Result<Vec<DVec3>, TissueError> {
        let mut result = vec![DVec3::ZERO; self.gradient.len()];
        let mut residual: Vec<_> = self.gradient.iter().map(|g| -*g).collect();
        let norm_squared = dot(&residual, &residual);
        if norm_squared == 0.0 {
            return Ok(result);
        }
        let precondition = |r: &[DVec3]| -> Vec<_> {
            if let Some(factor) = &factor {
                return factor.apply(r);
            }
            r.iter()
                .zip(&self.inverse_diagonal)
                .map(|(r, d)| *d * *r)
                .collect()
        };
        let mut z = precondition(&residual);
        let mut direction = z.clone();
        let mut rz = dot(&residual, &z);
        // A tighter linear tolerance preserves rigid free fall over repeated
        // velocity updates; the nonlinear force/correction limits are separate.
        let threshold = norm_squared * 1e-18;
        let mut iterations = 0;
        for _ in 0..256 {
            iterations += 1;
            if dot(&residual, &residual) <= threshold {
                // Recursive residuals can drift. Confirm with the unchanged
                // physical matrix; restart from the true residual if needed.
                residual = self.residual(&result);
                if dot(&residual, &residual) <= threshold {
                    return Ok(result);
                }
                z = precondition(&residual);
                direction = z.clone();
                rz = dot(&residual, &z);
            }
            let product = self.multiply(&direction);
            let curvature = dot(&direction, &product);
            if !rz.is_finite() || rz <= 0.0 || !curvature.is_finite() || curvature <= 0.0 {
                break;
            }
            let alpha = rz / curvature;
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
        let residual = self.residual(&result);
        let relative = (dot(&residual, &residual) / norm_squared).sqrt();
        if relative <= 1e-9 {
            Ok(result)
        } else {
            tracing::debug!(
                system = %serde_json::json!({
                    "float_encoding": "IEEE754 f64 bits, column-major 3x3 blocks",
                    "diagonal": self.diagonal.iter().map(|m| m.to_cols_array().map(f64::to_bits)).collect::<Vec<_>>(),
                    "couplings": self.couplings.iter().map(|c| serde_json::json!({
                        "nodes": c.nodes, "block": c.block.to_cols_array().map(f64::to_bits)
                    })).collect::<Vec<_>>(),
                    "gradient": self.gradient.iter().map(|v| v.to_array().map(f64::to_bits)).collect::<Vec<_>>(),
                    "solution": result.iter().map(|v| v.to_array().map(f64::to_bits)).collect::<Vec<_>>(),
                    "relative_residual": relative,
                    "incomplete_factor": factor.is_some(),
                    "iterations": iterations,
                    "rz": rz,
                }),
                "failed tissue linear system"
            );
            Err(TissueError::LinearSolveFailed(relative))
        }
    }

    fn residual(&self, solution: &[DVec3]) -> Vec<DVec3> {
        // The correction solves A*x = -gradient. A restart needs b - A*x,
        // just like the initial residual; reversing it amplifies the error.
        self.multiply(solution).iter().zip(&self.gradient).map(|(a, b)| -*a - *b).collect()
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
        let mut energy = objective(&x);
        for _ in 0..max_iterations {
            let mut system = self.linearize(&x, &self.positions, self.frame, f64::INFINITY, &[], &Dissipation::default());
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
                self.previous = None;
                return Ok(residual);
            }
            let delta = system.direction()?;
            (x, energy) = self.descend((&x, energy), &delta, dot(&system.gradient, &delta), &objective)?;
        }
        Err(TissueError::InvalidInput("equilibrium did not converge"))
    }

    fn descend(
        &self,
        current: (&[DVec3], f64),
        delta: &[DVec3],
        slope: f64,
        objective: impl Fn(&[DVec3]) -> f64,
    ) -> Result<(Vec<DVec3>, f64), TissueError> {
        let (x, energy) = current;
        let mut scale = 1.0;
        for _ in 0..20 {
            let candidate: Vec<_> = x.iter().zip(delta).map(|(x, d)| *x + scale * *d).collect();
            if self.validate_positions(&candidate).is_ok() {
                let candidate_energy = objective(&candidate);
                if candidate_energy <= energy + 1e-4 * scale * slope + 1e-15 {
                    return Ok((candidate, candidate_energy));
                }
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
        damping: &Dissipation,
    ) -> System<'_> {
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
        let diagonal: Vec<_> = diagonal_mass.iter().map(|m| DMat3::IDENTITY * *m).collect();
        let couplings: Vec<_> = self
            .edges
            .iter()
            .map(|nodes| Coupling {
                nodes: *nodes,
                block: DMat3::ZERO,
            })
            .collect();
        let mut system = System {
            pattern: &self.factor_pattern,
            couplings,
            diagonal,
            inverse_diagonal: Vec::new(),
            gradient,
        };
        for (index, tet) in self.tets.iter().enumerate() {
            let active = tet.ids.map(|i| self.inverse_mass[i] > 0.0);
            if !active.iter().any(|free| *free) { continue; }
            let (force, blocks) = self.elastic_differential(index, x, active);
            system.add_element(tet, force, blocks);
        }
        for (p, patch) in self.pressure_patches.iter().enumerate() {
            let evaluated = patch.evaluate(&self.tets, x);
            let damping = damping.pressures.get(p).copied().unwrap_or_default();
            let ratio = evaluated.ratio;
            let stress = patch.volume * (self.material.bulk_pa * (ratio - 1.)
                + damping.bulk * (ratio.ln() - damping.history_log_volume) / ratio);
            let stiffness = patch.volume * (self.material.bulk_pa + damping.bulk / (ratio * ratio));
            for (&i, &g) in patch.nodes.iter().zip(&evaluated.gradients) {
                system.gradient[i] += stress * g;
                system.diagonal[i] += stiffness * outer(g, g);
            }
            for &(index, a, b) in &patch.couplings {
                system.couplings[index].block += stiffness * outer(evaluated.gradients[a], evaluated.gradients[b]);
            }
        }
        for contact in contacts {
            let depth = contact.depth(x, &self.tets);
            if depth == 0.0 {
                continue;
            }
            let tet = &self.tets[contact.point.tetrahedron];
            let ids = tet.ids;
            let derivatives = contact
                .point
                .barycentric
                .map(|w| contact.normal * (contact.point.influence * w));
            let stiffness = contact.stiffness_n_per_m;
            for a in 0..4 {
                system.gradient[ids[a]] -= stiffness * depth * derivatives[a];
                system.diagonal[ids[a]] += stiffness * outer(derivatives[a], derivatives[a]);
            }
            for (index, [a, b]) in tet.edge_indices.iter().zip(material::EDGE_PAIRS) {
                // Both derivatives are parallel to the same normal, so
                // this block is symmetric even when node order is reversed.
                system.couplings[*index].block += stiffness * outer(derivatives[a], derivatives[b]);
            }
        }
        for edge in &damping.edges {
            let [a, b] = edge.nodes;
            let delta = x[b] - x[a];
            let length = delta.length();
            let n = delta / length;
            let g = edge.stiffness * (length - edge.target_length) * n;
            let axial = outer(n, n);
            // Exact gradient; retain a positive semidefinite tangent when an
            // edge shortens below its history-derived length target.
            let block = edge.stiffness * (axial + (1.0 - edge.target_length / length).max(0.0) * (DMat3::IDENTITY - axial));
            system.gradient[a] -= g;
            system.gradient[b] += g;
            system.diagonal[a] += block;
            system.diagonal[b] += block;
            system.couplings[self.edges.binary_search(&edge.nodes).expect("damping edge is indexed")].block -= block;
        }
        for element in &damping.elements {
            let tet = &self.tets[element.tet];
            let active = tet.ids.map(|i| self.inverse_mass[i] > 0.0);
            if !active.iter().any(|free| *free) { continue; }
            let (forces, blocks) = element.differential(tet, x, active);
            system.add_element(tet, forces, blocks);
        }
        // Eliminate fixed degrees of freedom from the displacement solve.
        system.eliminate_fixed(&self.inverse_mass);
        system
    }

    fn objective(&self, x: &[DVec3], predicted: &[DVec3], frame: Frame, dt: f64) -> f64 {
        let inertial: f64 = x
            .iter()
            .zip(predicted)
            .zip(&self.masses)
            .map(|((x, p), m)| 0.5 * m * x.distance_squared(*p) / (dt * dt))
            .sum();
        let elastic = self.elastic_energy(x);
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
        damping: &Dissipation,
    ) -> Result<(), TissueError> {
        let objective = |x: &[DVec3]| {
            self.objective(x, predicted, frame, dt)
                + contacts.iter().map(|c| c.energy(x, &self.tets)).sum::<f64>()
                + damping.energy(x, &self.tets, &self.pressure_patches)
        };
        // The objective is fixed for this physical solve. Retain the energy
        // already evaluated at each accepted trial, never across timesteps.
        let mut energy = objective(x);
        let (force_limit, correction_limit) = nonlinear_limits(dt, self.material.time_integration);
        // Check the final accepted descent too. Reaching the iteration budget
        // is not convergence; the caller may retry a smaller physical step.
        // Reuse only the SPD preconditioner within this physical step. Every
        // iteration still assembles the current physical matrix and verifies
        // its true residual; neither the force law nor tolerances change.
        let mut factor = None;
        for iteration in 0..=self.material.iterations {
            let system = self.linearize(x, predicted, frame, dt, contacts, damping);
            let force_squared = dot(&system.gradient, &system.gradient);
            if force_squared == 0.0 {
                return Ok(());
            }
            let delta = system.direction_cached(&mut factor).map_err(|error| {
                tracing::debug!(iteration, contacts = contacts.len(), dt, "tissue nonlinear solve context");
                error
            })?;
            // Both force balance and the remaining Newton correction must be
            // small. A force-only check is insufficient for compliant modes.
            // These are numerical stopping limits, not material damping.
            // Independent time-convergence and trajectory comparisons are
            // recorded in breast-soft-tissue-runtime.md.
            let correction_squared = delta.iter().map(|d| d.length_squared()).fold(0.0, f64::max);
            if force_squared <= force_limit * force_limit
                && correction_squared <= correction_limit * correction_limit
            {
                return Ok(());
            }
            if iteration == self.material.iterations {
                return Err(TissueError::NonlinearSolveFailed {
                    force_norm_n: force_squared.sqrt(),
                    correction_m: correction_squared.sqrt(),
                });
            }
            let slope = dot(&system.gradient, &delta);
            (*x, energy) = self.descend((x, energy), &delta, slope, &objective)?;
        }
        unreachable!("the inclusive final iteration returns a convergence result")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tissue_descent_returns_energy_of_accepted_backtrack_without_rechecking_current_state() {
        let tissue = super::super::tests::cube(Frame::IDENTITY, false);
        let x = tissue.positions.clone();
        let target: Vec<_> = x.iter().map(|p| *p + DVec3::X * 0.125).collect();
        let potential = |p: &[DVec3]| p.iter().zip(&target)
            .map(|(p, t)| p.distance_squared(*t)).sum::<f64>();
        let energy = potential(&x);
        let evaluations = std::cell::Cell::new(0);
        let objective = |p: &[DVec3]| {
            // The caller supplied this value; only new trials need evaluation.
            assert_ne!(p, x.as_slice());
            evaluations.set(evaluations.get() + 1);
            potential(p)
        };
        let direction = vec![DVec3::X * 0.5; x.len()];
        let slope = -0.125 * x.len() as f64;
        let (accepted, accepted_energy) = tissue.descend((&x, energy), &direction, slope, objective).unwrap();
        assert_eq!(accepted, target);
        assert_eq!(accepted_energy, potential(&accepted));
        assert!(accepted_energy < energy && evaluations.get() > 1);
        let ascent = vec![-DVec3::X * 0.125; x.len()];
        assert!(tissue.descend((&x, energy), &ascent, slope, objective).is_err());
        assert_eq!(x, tissue.positions);
        assert_eq!(energy, potential(&x));
    }

    #[test]
    fn restarted_linear_solve_converges_on_recorded_arm_fold_matrix() {
        // This exact matrix triggers residual replacement. Decimal parsing
        // can round by one ULP and hide the former wrong-sign restart bug.
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "fixtures/arm_fold_linear_system.json"
        )).unwrap();
        let vector = |v: &serde_json::Value| DVec3::from_array(
            std::array::from_fn(|i| f64::from_bits(v[i].as_u64().unwrap()))
        );
        let matrix = |v: &serde_json::Value| DMat3::from_cols_array(
            &std::array::from_fn(|i| f64::from_bits(v[i].as_u64().unwrap()))
        );
        let diagonal: Vec<_> = data["diagonal"].as_array().unwrap().iter().map(matrix).collect();
        let inverse_diagonal: Vec<_> = diagonal.iter().map(|d| {
            if *d == DMat3::ZERO { DMat3::ZERO } else { d.inverse() }
        }).collect();
        let couplings: Vec<_> = data["couplings"].as_array().unwrap().iter().map(|c| Coupling {
            nodes: std::array::from_fn(|i| c["nodes"][i].as_u64().unwrap() as usize),
            block: matrix(&c["block"]),
        }).collect();
        let gradient: Vec<_> = data["gradient"].as_array().unwrap().iter().map(vector).collect();
        // Preserve the IC(0) restart regression as well as checking the new
        // complete factor. A more accurate factor alone can mask that bug.
        for pattern in [
            super::super::preconditioner::Pattern::new(diagonal.len(), couplings.iter().map(|c| c.nodes)),
            super::super::preconditioner::Pattern::incomplete(diagonal.len(), couplings.iter().map(|c| c.nodes)),
        ] {
            let diagonal = diagonal.clone();
            let inverse_diagonal = inverse_diagonal.clone();
            let gradient = gradient.clone();
            let couplings = couplings.iter().map(|c| Coupling { nodes: c.nodes, block: c.block }).collect();
            let system = System { pattern: &pattern, diagonal, inverse_diagonal, couplings, gradient };
            let solution = system.direction().unwrap();
            let residual = system.residual(&solution);
            assert!((dot(&residual, &residual) / dot(&system.gradient, &system.gradient)).sqrt() <= 1e-9);
            // Also check the signed residual, independently of convergence luck.
            let probe = vec![DVec3::splat(1e-6); system.gradient.len()];
            for (r, (ax, g)) in system.residual(&probe).iter().zip(system.multiply(&probe).iter().zip(&system.gradient)) {
                assert_eq!(*r, -*g - *ax);
            }
        }
    }

    #[test]
    fn linear_solve_rejects_indefinite_system_instead_of_returning_a_capped_direction() {
        let pattern = super::super::preconditioner::Pattern::new(1, [].into_iter());
        let system = System {
            pattern: &pattern,
            couplings: vec![],
            diagonal: vec![-DMat3::IDENTITY],
            inverse_diagonal: vec![-DMat3::IDENTITY],
            gradient: vec![DVec3::X],
        };
        assert!(matches!(system.direction(), Err(TissueError::LinearSolveFailed(r)) if r == 1.0));
    }

    #[test]
    fn coupled_length_rate_damping_has_the_energy_gradient_and_psd_tangent() {
        let mut tissue = super::super::tests::cube(Frame::IDENTITY, false);
        tissue.material.time_integration = TimeIntegration::Bdf2;
        let dt = 1./240.;
        let step = IntegrationStep::new(&tissue, dt, DVec3::ZERO);
        for scale in [0.9, 1.1] {
            let mut x: Vec<_> = tissue.positions.iter().map(|p| *p*scale).collect();
            let full = tissue.linearize(&x, &step.predicted, Frame::IDENTITY, dt, &[], &step.damping);
            let base = tissue.linearize(&x, &step.predicted, Frame::IDENTITY, dt, &[], &Dissipation::default());
            let epsilon = 1e-7;
            for i in 0..x.len() {
                for axis in 0..3 {
                    x[i][axis] += epsilon;
                    let plus: f64 = step.damping.energy(&x, &tissue.tets, &tissue.pressure_patches);
                    x[i][axis] -= 2.*epsilon;
                    let minus: f64 = step.damping.energy(&x, &tissue.tets, &tissue.pressure_patches);
                    x[i][axis] += epsilon;
                    let expected = (plus-minus)/(2.*epsilon);
                    assert!((full.gradient[i][axis]-base.gradient[i][axis]-expected).abs() < 1e-6);
                }
            }
            let direction: Vec<_> = (0..x.len()).map(|i| DVec3::new((i as f64).sin(), (i as f64*2.).cos(), 0.3)).collect();
            let a = full.multiply(&direction);
            let b = base.multiply(&direction);
            assert!(direction.iter().zip(a.iter().zip(&b)).map(|(v,(a,b))| v.dot(*a-*b)).sum::<f64>() >= -1e-8);
            let rotation = DQuat::from_rotation_x(0.4);
            let moved: Vec<_> = x.iter().map(|p| rotation * *p + DVec3::splat(2.)).collect();
            let energy = |p: &[DVec3]| step.damping.energy(p, &tissue.tets, &tissue.pressure_patches);
            assert!((energy(&x)-energy(&moved)).abs() < 1e-10);
        }
    }

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
        let system = tissue.linearize(&x, &tissue.positions, Frame::IDENTITY, dt, &[contact], &Dissipation::default());
        let no_contact = tissue.linearize(&x, &tissue.positions, Frame::IDENTITY, dt, &[], &Dissipation::default());
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
            let a = tissue.linearize(x, &tissue.positions, Frame::IDENTITY, dt, &[contact], &Dissipation::default());
            let b = tissue.linearize(x, &tissue.positions, Frame::IDENTITY, dt, &[], &Dissipation::default());
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
        for stiffening in [0.0, 8.0] {
            check_objective_gradient_and_sparse_system(stiffening);
        }
    }

    fn check_objective_gradient_and_sparse_system(stiffening: f64) {
        let mut tissue = super::super::tests::cube(Frame::IDENTITY, true);
        tissue.material.strain_stiffening = stiffening;
        tissue.attachments[7] = Attachment::Compliant {
            compliance_m_per_n: 0.001,
        };
        let mut x = tissue.positions.clone();
        x[5] += DVec3::new(0.001, -0.002, 0.0003);
        let predicted: Vec<_> = x.iter().map(|p| *p + DVec3::Y * 0.0001).collect();
        let dt = 1.0 / 240.0;
        let system = tissue.linearize(&x, &predicted, Frame::IDENTITY, dt, &[], &Dissipation::default());
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
            let strain = f - material::polar_rotation(f);
            let strain_gradient = shape.map(|g| strain * g);
            let df: DMat3 = (0..4).map(|k| outer(a[tet.ids[k]], shape[k])).sum();
            let dj: f64 = (0..4).map(|k| a[tet.ids[k]].dot(volume[k])).sum();
            let dq: f64 = (0..4).map(|k| 2.0 * a[tet.ids[k]].dot(strain_gradient[k])).sum();
            for k in 0..4 {
                element_product[tet.ids[k]] +=
                    2.0 * tissue.material.shear_pa * tet.volume * df * shape[k]
                        * (1.0 + stiffening * material::squared_norm(strain))
                        + 2.0 * tissue.material.shear_pa * tet.volume * stiffening * dq * strain_gradient[k]
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
        let direction = system.direction().unwrap();
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
