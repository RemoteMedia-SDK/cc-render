"""Replay the exact failed matrix with the unchanged iterative solver."""
from pathlib import Path
import subprocess, tempfile, json
root=Path.cwd(); out=root/'artifacts/soft-tissue/avatar-contact'
code=(root/'src/cc_render/soft_tissue/implicit.rs').read_text().split('impl Tissue {')[0]
code=code.replace('use super::*;','use bevy::math::DVec3;\n#[derive(Debug)] enum TissueError { LinearSolveFailed(f64) }')
code=code.replace('use super::timestep::Dissipation;','')
code=code.replace('super::preconditioner','preconditioner')
code=code.replace('fn direction(&self)', 'fn direction(&self, budget: usize)')
code=code.replace('for _ in 0..256 {', 'for iteration in 0..budget {')
code=code.replace('return Ok(result);\n                }', 'eprintln!("converged iterations {}", iteration); return Ok(result);\n                }')
code+='''
mod preconditioner;
fn main() {
 let d: serde_json::Value=serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
 let matrix=|v:&serde_json::Value| DMat3::from_cols_array(&std::array::from_fn(|i|v[i].as_f64().unwrap()));
 let diagonal:Vec<_>=d["diagonal"].as_array().unwrap().iter().map(matrix).collect();
 let inverse_diagonal=diagonal.iter().map(|d|if *d==DMat3::ZERO {DMat3::ZERO} else {d.inverse()}).collect();
 let couplings:Vec<_>=d["couplings"].as_array().unwrap().iter().map(|c| Coupling {nodes:[c["nodes"][0].as_u64().unwrap() as usize,c["nodes"][1].as_u64().unwrap() as usize],block:matrix(&c["block"])}).collect();
 let gradient=d["gradient"].as_array().unwrap().iter().map(|v|DVec3::from_array(std::array::from_fn(|i|v[i].as_f64().unwrap()))).collect();
 let pattern=preconditioner::Pattern::new(diagonal.len(),couplings.iter().map(|c|c.nodes));
 let system=System {pattern:&pattern,diagonal,inverse_diagonal,couplings,gradient};
 let factor=preconditioner::Factor::new(&pattern,&system.diagonal,|i|system.couplings[i].block).unwrap();
 std::fs::write(std::env::args().nth(2).unwrap(),factor.diagnostic().to_string()).unwrap();
 for budget in [256,512,768,1024] {
  let start=std::time::Instant::now();let solved=system.direction(budget);
  let residual=match &solved { Ok(x)=>{let r=system.residual(x);(dot(&r,&r)/dot(&system.gradient,&system.gradient)).sqrt()},Err(TissueError::LinearSolveFailed(r))=>*r};
  println!("{}",serde_json::json!({"budget":budget,"success":solved.is_ok(),"relative_residual":residual,"seconds":start.elapsed().as_secs_f64()}));
 }
}
'''
(out/'probe-linear.rs').write_text(code)
with tempfile.TemporaryDirectory(prefix='tissue-linear-') as temp:
 temp=Path(temp); (temp/'probe.rs').write_text(code); (temp/'preconditioner.rs').write_text((root/'src/cc_render/soft_tissue/preconditioner.rs').read_text())
 command=['rustc','--edition=2021','-C','opt-level=1','-L',f'dependency={root}/target/debug/deps',str(temp/'probe.rs'),'-o',str(temp/'probe')]
 for name in ['bevy','serde_json','tracing']:
  libs=list((root/'target/debug/deps').glob(f'lib{name}-*.rlib')); assert len(libs)==1
  command+=['--extern',f'{name}={libs[0]}']
 with (out/'probe-linear-build.log').open('w') as log: subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,check=True)
 with (out/'probe-linear-results.ndjson').open('w') as log, (out/'probe-linear-iterations.log').open('w') as err: subprocess.run([str(temp/'probe'),str(out/'failed-linear-system.json'),str(out/'recomputed-factor.json')],stdout=log,stderr=err,check=True)
print((out/'probe-linear-results.ndjson').read_text()); print((out/'probe-linear-iterations.log').read_text())
