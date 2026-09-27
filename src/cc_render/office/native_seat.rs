//! Candidate-identity-scoped geometric seat admission. No motion is produced here.
use super::{avatar_geometry::AvatarGeometry, OfficeObject};
use crate::cc_render::renderer::SkeletalPose;
use serde::Serialize;
use std::{collections::VecDeque, sync::Arc};

/// Fixed measurement tolerance, not a target height or a pose correction.
const PENETRATION_TOLERANCE_M: f32 = 0.001;
const MAX_SCOPES: usize = 8;


#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeatScope {
    pub session_id: String,
    pub generation_revision: u64,
    pub target_id: String,
}
impl SeatScope {
    fn matches(&self, pose: &SkeletalPose) -> bool {
        pose.stream.as_ref().is_some_and(|s| {
            s.valid()
                && s.session_id == self.session_id
                && s.generation_revision == self.generation_revision
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SeatObservation {
    pub session_id: String,
    pub generation_revision: u64,
    pub target_id: String,
    pub pts_ms: u64,
    pub patch_distances_m: [f32; 2],
    pub maximum_penetration_m: f32,
    /// Diagnostic overlap admitted under the default warning policy.
    pub collision_warning: bool,
}

#[derive(Debug)]
pub(crate) struct NativeSeatAdmission {
    geometry: Arc<AvatarGeometry>,
    scopes: VecDeque<SeatScope>,
    strict_collisions: bool,
}
impl NativeSeatAdmission {
    pub(crate) fn profile_identity(&self) -> &str { self.geometry.profile_identity() }
    pub(crate) fn new(geometry: Arc<AvatarGeometry>) -> Self {
        Self {
            geometry,
            scopes: VecDeque::new(),
            strict_collisions: std::env::var("CC_RENDER_STRICT_SEAT_COLLISIONS").as_deref() == Ok("1"),
        }
    }
    pub(crate) fn authorize(&mut self, scope: SeatScope) -> Result<(), String> {
        if scope.session_id.is_empty() || scope.target_id.is_empty() {
            return Err("empty native seat identity".into());
        }
        if let Some(old) = self.scopes.iter().find(|old| {
            old.session_id == scope.session_id
                && old.generation_revision == scope.generation_revision
        }) {
            return if old == &scope {
                Ok(())
            } else {
                Err("native seat scope replacement rejected".into())
            };
        }
        self.scopes.push_back(scope);
        while self.scopes.len() > MAX_SCOPES {
            self.scopes.pop_front();
        }
        Ok(())
    }
    /// None means ordinary root admission still applies. Err is never an exemption.
    pub(crate) fn measure(
        &self,
        pose: &SkeletalPose,
        previous: Option<&SkeletalPose>,
        objects: &[OfficeObject],
    ) -> Option<Result<SeatObservation, String>> {
        let scope = self.scopes.iter().find(|scope| scope.matches(pose))?;
        Some(self.measure_scoped(pose, previous, objects, scope))
    }
    fn measure_scoped(
        &self,
        pose: &SkeletalPose,
        previous: Option<&SkeletalPose>,
        objects: &[OfficeObject],
        scope: &SeatScope,
    ) -> Result<SeatObservation, String> {
        let object = objects
            .iter()
            .find(|object| object.id == scope.target_id)
            .ok_or("native seat target disappeared")?;
        if object.dynamic {
            return Err("native seat admission requires static support".into());
        }
        let previous = previous
            .filter(|p| {
                p.pts_ms <= pose.pts_ms
                    && p.stream
                        .as_ref()
                        .is_some_and(|s| s.valid() && s.session_id == scope.session_id)
            })
            .ok_or("native seat admission requires displayed native history")?;
        let world = self.geometry.world(pose)?;
        let evidence = self.geometry.measure_seat(&world, object)?;
        let limit = PENETRATION_TOLERANCE_M;
        let mut collision_warning = evidence.maximum_penetration_m > limit;
        if self.strict_collisions && collision_warning {
            return Err(format!(
                "native seat mesh penetration {:.6}m exceeds {:.6}m",
                evidence.maximum_penetration_m, limit
            ));
        }
        let previous_world = self.geometry.world(previous)?;
        let boxes = super::support::boxes(std::slice::from_ref(object));
        if boxes
            .iter()
            .any(|b| b.half.min_element() <= limit)
        {
            return Err("seat solid thinner than measurement tolerance".into());
        }
        // Retain a swept check for the target instead of exempting its footprint.
        // This measures vertex segments; it does not synthesize intermediate poses.
        for mesh in &self.geometry.meshes {
            if mesh
                .points(&previous_world)
                .zip(mesh.points(&world))
                .any(|(a, b)| {
                    boxes
                        .iter()
                        .any(|solid| solid.intersects(a, b, -limit))
                })
            {
                if self.strict_collisions {
                    return Err("native seat swept vertex collision".into());
                }
                collision_warning = true;
            }
        }
        if collision_warning {
            tracing::warn!(session_id = %scope.session_id, revision = scope.generation_revision,
                pts_ms = pose.pts_ms, penetration_m = evidence.maximum_penetration_m,
                "seat overlap admitted with warning; motion unchanged");
        }
        Ok(SeatObservation {
            session_id: scope.session_id.clone(),
            generation_revision: scope.generation_revision,
            target_id: scope.target_id.clone(),
            pts_ms: pose.pts_ms,
            patch_distances_m: evidence.patch_distances_m,
            maximum_penetration_m: evidence.maximum_penetration_m,
            collision_warning,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::timeline::StreamIdentity;
    #[test]
    fn rejects_swept_collision_and_never_replaces_authorized_target() {
        let mut admission = NativeSeatAdmission::new(Arc::new(AvatarGeometry::test_fixture()));
        admission.strict_collisions = true; // Independent of launcher environment.
        let scope = SeatScope {
            session_id: "owned".into(),
            generation_revision: 7,
            target_id: "chair".into(),
        };
        let mut chair = super::super::starter_objects()
            .into_iter()
            .find(|o| o.id == "chair")
            .unwrap();
        chair.position = [0.; 3];
        chair.parts.truncate(1);
        chair.parts[0].position = [0.; 3];
        chair.parts[0].half_extents = [2., 0.5, 0.5];
        let objects = [chair];
        let mut previous = SkeletalPose::rest();
        previous.stream = Some(StreamIdentity {
            session_id: "owned".into(),
            generation_revision: 7,
            skeleton_id: "smpl22".into(),
            root_mode: "world".into(),
        });
        previous.root_pos = [0., 0., -1.];
        let mut pose = previous.clone();
        pose.root_pos[2] = 1.;
        assert!(admission
            .measure(&pose, Some(&previous), &objects)
            .is_none());
        admission.authorize(scope.clone()).unwrap();
        assert!(admission
            .measure(&pose, Some(&previous), &objects)
            .unwrap()
            .unwrap_err()
            .contains("swept"));
        pose.root_pos[2] = 0.;
        assert!(admission
            .measure(&pose, Some(&previous), &objects)
            .unwrap()
            .unwrap_err()
            .contains("penetration"));
        pose.root_pos[2] = -0.498; // 2mm endpoint overlap: strict fails, explicit development warns.
        assert!(admission.measure(&pose, Some(&previous), &objects).unwrap().is_err());
        admission.strict_collisions = false;
        let warning = admission.measure(&pose, Some(&previous), &objects).unwrap().unwrap();
        assert!(warning.collision_warning);
        assert!(warning.maximum_penetration_m > PENETRATION_TOLERANCE_M);
        pose.root_pos[2] = 1.; // Swept overlap is diagnostic in warning mode too.
        assert!(admission.measure(&pose, Some(&previous), &objects).unwrap().unwrap().collision_warning);
        pose.root_pos[2] = -0.496; // No second hard failure threshold above 3mm.
        assert!(admission.measure(&pose, Some(&previous), &objects).unwrap().unwrap().collision_warning);
        assert!(admission.measure(&pose, None, &objects).unwrap().is_err());
        admission.strict_collisions = true;
        pose.root_pos[2] = -0.75;
        assert!(admission
            .measure(&pose, Some(&previous), &objects)
            .unwrap()
            .is_ok());
        assert!(admission.measure(&pose, None, &objects).unwrap().is_err());
        assert!(admission
            .authorize(SeatScope {
                target_id: "desk".into(),
                ..scope
            })
            .is_err());
        assert!(admission
            .measure(&pose, Some(&previous), &objects)
            .unwrap()
            .is_ok());
    }

    #[test]
    fn seat_scope_cannot_authorize_foreign_or_adjacent_generations() {
        let scope = SeatScope {
            session_id: "owned".into(),
            generation_revision: 7,
            target_id: "chair".into(),
        };
        let mut pose = SkeletalPose::rest();
        assert!(!scope.matches(&pose));
        pose.stream = Some(StreamIdentity {
            session_id: "owned".into(),
            generation_revision: 7,
            skeleton_id: "smpl22".into(),
            root_mode: "world".into(),
        });
        assert!(scope.matches(&pose));
        pose.stream.as_mut().unwrap().generation_revision = 8;
        assert!(!scope.matches(&pose));
        pose.stream.as_mut().unwrap().generation_revision = 7;
        pose.stream.as_mut().unwrap().session_id = "foreign".into();
        assert!(!scope.matches(&pose));
    }
}
