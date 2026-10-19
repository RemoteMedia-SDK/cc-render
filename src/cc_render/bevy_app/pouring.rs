//! Open-container gravity dispensing from observed rigid-body orientation.
//! Bounded parcel approximation; does not model shampoo viscosity or squeeze pressure.
use bevy::prelude::*;
use crate::cc_render::scene_water::WaterFixture;

pub(super) fn update(fixture: &mut WaterFixture, pose: Option<&GlobalTransform>,
    used_liters: f64, dt: f64) -> f32 {
    let Some(container) = &fixture.container else { return 0.; };
    let Some(pose) = pose else { return 0.; };
    if dt <= 0. || !dt.is_finite() { return 0.; }
    fixture.outlet = pose.transform_point(Vec3::from(container.mouth)).to_array();
    let up = pose.compute_transform().rotation * Vec3::Y;
    let tilt_flow = (-up.y - 0.1).clamp(0., 1.);
    let remaining = (container.capacity_liters - used_liters).max(0.);
    let capacity_flow = remaining * 60. / (f64::from(fixture.max_liters_per_minute) * dt);
    tilt_flow.min(capacity_flow as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pouring_follows_physical_mouth_and_stops_upright_missing_or_empty() {
        let mut f: WaterFixture = serde_json::from_value(serde_json::json!({
            "id":"soap","kind":"pour","outlet":[0,1,0],"surface_y":0,
            "spray_radius_m":0,"max_liters_per_minute":0.3,
            "container":{"prop":"bottle","mouth":[0,0.1,0],"capacity_liters":0.01}})).unwrap();
        assert_eq!(update(&mut f,None,0.,0.1),0.);
        let upright = GlobalTransform::from_translation(Vec3::new(1.,2.,3.));
        assert_eq!(update(&mut f,Some(&upright),0.,0.1),0.);
        let tilted = GlobalTransform::from(Transform::from_xyz(1.,2.,3.)
            .with_rotation(Quat::from_rotation_z(std::f32::consts::PI)));
        assert!(update(&mut f,Some(&tilted),0.,0.1) > 0.8);
        assert!(Vec3::from(f.outlet).distance(Vec3::new(1.,1.9,3.)) < 1e-5);
        assert_eq!(update(&mut f,Some(&tilted),0.01,0.1),0.);
        let flow = update(&mut f,Some(&tilted),0.0099,1.);
        assert!((f64::from(flow*f.max_liters_per_minute)/60. - 0.0001).abs() < 1e-9);
    }
}
