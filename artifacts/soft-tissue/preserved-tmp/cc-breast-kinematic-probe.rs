
use rapier3d::prelude::*;
fn run(fps: usize, resample: bool) {
 let mut bodies=RigidBodySet::new(); let mut colliders=ColliderSet::new();
 let mut joints=ImpulseJointSet::new(); let mut multibody=MultibodyJointSet::new();
 let mut pipeline=PhysicsPipeline::new(); let mut broad=BroadPhaseMultiSap::new();
 let mut narrow=NarrowPhase::new(); let mut islands=IslandManager::new(); let mut ccd=CCDSolver::new();
 let anchor=bodies.insert(RigidBodyBuilder::kinematic_position_based());
 let body=bodies.insert(RigidBodyBuilder::dynamic().additional_mass(2.0).gravity_scale(0.0).can_sleep(false));
 colliders.insert_with_parent(ColliderBuilder::ball(0.04).collision_groups(InteractionGroups::none()),body,&mut bodies);
 let mut joint=GenericJointBuilder::new(JointAxesMask::ANG_AXES);
 for axis in [JointAxis::LinX,JointAxis::LinY,JointAxis::LinZ] { joint=joint.limits(axis,[-0.03,0.03]).motor_model(axis,MotorModel::ForceBased).motor_position(axis,0.0,1260.0,60.0); }
 joints.insert(anchor,body,joint,true);
 let params=IntegrationParameters {dt:1.0/480.0,..Default::default()};
 let steps=480/fps; let mut first=Vec::new(); let mut last_offset=0.0f32;
 for frame in 1..=fps*4 {
  if !resample { bodies[anchor].set_next_kinematic_position(Isometry::translation(0.5*frame as f32/fps as f32,0.,0.)); }
  for s in 0..steps {
   if resample {bodies[anchor].set_next_kinematic_position(Isometry::translation(0.5*((frame-1)*steps+s+1) as f32/480.0,0.,0.));}
   pipeline.step(&Vector::zeros(),&params,&mut islands,&mut broad,&mut narrow,&mut bodies,&mut colliders,&mut joints,&mut multibody,&mut ccd,None,&(),&());
   if frame==2 {first.push(bodies[anchor].linvel().x);}
  }
  last_offset=bodies[body].translation().x-bodies[anchor].translation().x;
 }
 println!("fps={fps} resample={resample} anchor_v={first:?} steady_end_offset_mm={:.3}",last_offset*1000.0);
}
fn main(){for fps in [30,60,120]{run(fps,false);run(fps,true);}}
