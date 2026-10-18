#[path="/tmp/cc-tissue-reference-source/mod.rs"] mod reference;
#[path="/home/acidhax/dev/personal/remotemedia/cc-render-soft-tissue/src/cc_render/soft_tissue/mod.rs"] mod candidate;
use bevy::math::{DVec3,DQuat};
fn main()->anyhow::Result<()> {
 let bytes=std::fs::read("assets/tissue/beautiful_assistant.fitted-prototype.json")?;
 let old=reference::TissueAsset::from_json(&bytes)?;
 let new=candidate::TissueAsset::from_json(&bytes)?;
 for (mut a,mut b) in old.regions.into_iter().zip(new.regions) {
  a.material.iterations=2; b.material.iterations=2;
  let mut sa=a.solver(reference::Frame::IDENTITY)?;
  let mut sb=b.solver(candidate::Frame::IDENTITY)?;
  let mut maximum=0.0_f64; let mut wa=std::time::Duration::ZERO;let mut wb=wa;
  for step in 1..=720 {
   let t=(step as f64/120.0-3.0).max(0.0);
   let translation=DVec3::new(0.01*(1.0-(t*2.0).cos()),0.005*(1.0-t.cos()),0.0);
   let rotation=DQuat::from_rotation_z(0.05*(1.0-t.cos()));
   let start=std::time::Instant::now();sa.step(1.0/120.0,reference::Frame{translation,rotation},DVec3::NEG_Y*9.81)?;wa+=start.elapsed();
   let start=std::time::Instant::now();sb.step(1.0/120.0,candidate::Frame{translation,rotation},DVec3::NEG_Y*9.81)?;wb+=start.elapsed();
   for (pa,pb) in sa.positions().iter().zip(sb.positions()) { maximum=maximum.max(pa.distance(*pb)); }
  }
  println!("{}",serde_json::json!({"region":a.name,"max_node_difference_m":maximum,"reference_wall_seconds":wa.as_secs_f64(),"sparse_wall_seconds":wb.as_secs_f64(),"reference_metrics":sa.metrics(),"sparse_metrics":sb.metrics()}));
  anyhow::ensure!(maximum<1e-5,"reference difference exceeds 0.01mm");
 }
 Ok(())
}
