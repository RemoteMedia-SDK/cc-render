//! Copy physics observations into scene geometry; never write physical/body poses.
use bevy::prelude::*;
use super::controller::OfficeController;
use crate::cc_render::renderer::PropSnapshot;

impl OfficeController {
    /// Returns the existing legacy free-prop replan signal. Articulated updates
    /// only change observed geometry and its waypoint digest, never legacy motion.
    pub(super) fn update_prop_observations(&mut self, props: &[PropSnapshot]) -> bool {
        let mut moved = false;
        for prop in props {
            let Some(object) = self.objects.iter_mut().find(|o| o.id == prop.id) else { continue; };
            let pose = Transform::from_matrix(Mat4::from_cols_array_2d(&prop.transform));
            let carried = self.held.iter().any(|(_, id)| id == &prop.id);
            moved |= !carried && !self.constrained_objects.contains(&prop.id)
                && Vec3::from(object.position).distance(pose.translation) > 0.03;
            object.position = pose.translation.to_array();
            object.rotation_xyzw = Some(pose.rotation.to_array());
            object.yaw_deg = pose.rotation.to_euler(EulerRot::YXZ).0.to_degrees();
            match prop.articulation.as_ref().map(|a| &a.state) {
                Some(crate::cc_render::renderer::ArticulationCoordinate::Prismatic { travel_m, .. }) => {
                    self.articulation.insert(prop.id.clone(), *travel_m);
                }
                _ => { self.articulation.remove(&prop.id); }
            }
        }
        moved
    }
}
