//! Sparse implicit elasticity. The global solve retains the
//! coupling between neighbouring elements that local XPBD sweeps leave behind.
use bevy::math::DVec3;
#[derive(Debug)] enum TissueError { LinearSolveFailed(f64) }
use bevy::math::DMat3;


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

struct System<'a> {
    pattern: &'a preconditioner::Pattern,
    couplings: Vec<Coupling>,
    diagonal: Vec<DMat3>,
    inverse_diagonal: Vec<DMat3>,
    gradient: Vec<DVec3>,
}

impl System<'_> {
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

    fn direction(&self, budget: usize) -> Result<Vec<DVec3>, TissueError> {
        let mut result = vec![DVec3::ZERO; self.gradient.len()];
        let mut residual: Vec<_> = self.gradient.iter().map(|g| -*g).collect();
        let norm_squared = dot(&residual, &residual);
        if norm_squared == 0.0 {
            return Ok(result);
        }
        let factor = preconditioner::Factor::new(self.pattern, &self.diagonal, |i| self.couplings[i].block);
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
        for iteration in 0..budget {
            iterations += 1;
            if dot(&residual, &residual) <= threshold {
                // Recursive residuals can drift. Confirm with the unchanged
                // physical matrix; restart from the true residual if needed.
                residual = self.residual(&result);
                if dot(&residual, &residual) <= threshold {
                    eprintln!("converged iterations {}", iteration); return Ok(result);
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
                    "diagonal": self.diagonal.iter().map(DMat3::to_cols_array).collect::<Vec<_>>(),
                    "couplings": self.couplings.iter().map(|c| serde_json::json!({
                        "nodes": c.nodes, "block": c.block.to_cols_array()
                    })).collect::<Vec<_>>(),
                    "gradient": self.gradient.iter().map(|v| v.to_array()).collect::<Vec<_>>(),
                    "solution": result.iter().map(|v| v.to_array()).collect::<Vec<_>>(),
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


mod preconditioner;
fn main() {
 let d: serde_json::Value=serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
 let matrix=|v:&serde_json::Value| DMat3::from_cols_array(&std::array::from_fn(|i|f64::from_bits(v[i].as_u64().unwrap())));
 let diagonal:Vec<_>=d["diagonal"].as_array().unwrap().iter().map(matrix).collect();
 let inverse_diagonal=diagonal.iter().map(|d|if *d==DMat3::ZERO {DMat3::ZERO} else {d.inverse()}).collect();
 let couplings:Vec<_>=d["couplings"].as_array().unwrap().iter().map(|c| Coupling {nodes:[c["nodes"][0].as_u64().unwrap() as usize,c["nodes"][1].as_u64().unwrap() as usize],block:matrix(&c["block"])}).collect();
 let gradient=d["gradient"].as_array().unwrap().iter().map(|v|DVec3::from_array(std::array::from_fn(|i|f64::from_bits(v[i].as_u64().unwrap())))).collect();
 let pattern=preconditioner::Pattern::new(diagonal.len(),couplings.iter().map(|c|c.nodes));
 let system=System {pattern:&pattern,diagonal,inverse_diagonal,couplings,gradient};
 let factor=preconditioner::Factor::new(&pattern,&system.diagonal,|i|system.couplings[i].block).unwrap();

 for budget in [256,512,768,1024] {
  let start=std::time::Instant::now();let solved=system.direction(budget);
  let residual=match &solved { Ok(x)=>{let r=system.residual(x);(dot(&r,&r)/dot(&system.gradient,&system.gradient)).sqrt()},Err(TissueError::LinearSolveFailed(r))=>*r};
  println!("{}",serde_json::json!({"budget":budget,"success":solved.is_ok(),"relative_residual":residual,"seconds":start.elapsed().as_secs_f64()}));
 }
}
