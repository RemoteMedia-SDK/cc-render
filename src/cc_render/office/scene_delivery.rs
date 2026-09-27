//! Transport-only scene handoffs. Boundaries refer to existing model samples;
//! this module cannot construct or edit a pose.
use super::{controller::OfficeSnapshot, motion_intent::MotionGoal};

#[derive(Default)]
pub(crate) struct SceneDelivery {
    pub pending: Option<(MotionGoal, bool)>,
    reservation: Option<(String, u64, u64)>,
}
impl SceneDelivery {
    pub(crate) fn anchor(office: &OfficeSnapshot) -> Option<u64> {
        let displayed = office.avatar.displayed_pts_ms;
        let end = office.avatar.buffered_until_pts_ms?;
        let latest = end.min(displayed.checked_add(2400)?);
        let anchor = end.checked_sub((end-latest).div_ceil(200)*200)?;
        (anchor >= displayed.checked_add(2200)?).then_some(anchor)
    }

    pub(crate) fn reserve(&mut self, goal: &MotionGoal, anchor: u64) {
        self.reservation = Some((goal.session_id().into(), goal.revision(), anchor));
    }

    /// Called before forwarding even the FIRST new frame to the renderer.
    /// Failure rejects delivery; it must not skip a prefix and resume halfway.
    pub(crate) fn first_frame(&mut self, session: &str, revision: u64, pts: u64, displayed: u64) -> Result<(), String> {
        let Some((expected_session, expected_revision, anchor)) = self.reservation.as_ref() else { return Ok(()) };
        if expected_session != session || *expected_revision != revision { return Ok(()); }
        if pts != anchor + 50 || displayed > *anchor {
            return Err(format!("scene delivery missed reserved native handoff: anchor={anchor} first={pts} displayed={displayed}"));
        }
        self.reservation = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::controller::OfficeController;
    fn goal() -> MotionGoal {
        serde_json::from_value(serde_json::json!({"kind":"motion_goal","version":3,
            "session_id":"test","generation_revision":2,"pts_ms":1500,"seed":42,
            "goal":{"operation":"navigate","target_id":"chair"}})).unwrap()
    }
    #[test]
    fn same_native_token_boundary_as_worker_and_insufficient_lead_is_not_ready() {
        let mut office = OfficeController::default().snapshot();
        office.avatar.displayed_pts_ms = 1500;
        office.avatar.buffered_until_pts_ms = Some(4000);
        assert_eq!(SceneDelivery::anchor(&office), Some(3800));
        office.avatar.buffered_until_pts_ms = Some(3500);
        assert_eq!(SceneDelivery::anchor(&office), None);
    }
    #[test]
    fn first_frame_must_arrive_before_reserved_prefix_finishes() {
        let mut delivery = SceneDelivery::default();
        delivery.reserve(&goal(), 3800);
        assert!(delivery.first_frame("test", 2, 3850, 3750).is_ok());
        delivery.reserve(&goal(), 3800);
        assert!(delivery.first_frame("test", 2, 3850, 3801).is_err());
        assert!(delivery.first_frame("test", 2, 3900, 3750).is_err());
    }
}
