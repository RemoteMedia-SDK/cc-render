//! Test client: submit each action only when observed state permits it.
//! The controller/ARDY runtime never receives the rest of this script in advance.
use cc_render_plugin::cc_render::office::controller::{
    OfficeCommand, OfficeSnapshot, Status, Verb,
};
use cc_render_plugin::cc_render::renderer::Hand;

pub struct Choreography {
    step: usize,
    sent: bool,
    hold_until: Option<u64>,
    hold_ms: u64,
    walkthrough: bool,
    surface_jump: bool,
    surface_up_only: bool,
}
impl Choreography {
    pub fn new() -> Self {
        Self {
            surface_jump: std::env::var("OFFICE_ACCEPTANCE_SURFACE_JUMP").as_deref() == Ok("1"),
            surface_up_only: std::env::var("OFFICE_ACCEPTANCE_SURFACE_UP_ONLY").as_deref() == Ok("1"),
            step: 0,
            sent: false,
            hold_until: None,
            hold_ms: std::env::var("OFFICE_ACCEPTANCE_HOLD_SEC")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(3)
                .clamp(3, 120)
                * 1000,
            walkthrough: std::env::var("OFFICE_ACCEPTANCE_WALKTHROUGH").as_deref() == Ok("1"),
        }
    }
    fn is_hold(&self) -> bool {
        (self.surface_jump && matches!(self.step, 1 | 3))
            || (!self.surface_jump && ((!self.walkthrough && matches!(self.step, 1 | 9)) || (self.walkthrough && self.step == 6)))
    }
    pub fn tick(&mut self, state: &OfficeSnapshot) -> (Option<OfficeCommand>, bool) {
        // Wait on the playback clock, including when inference stalls.
        if self.is_hold()
        {
            let end = *self
                .hold_until
                .get_or_insert(state.avatar.displayed_pts_ms + self.hold_ms);
            if state.avatar.displayed_pts_ms < end {
                return (None, false);
            }
            self.step += 1;
            self.hold_until = None;
        }
        let action = if self.surface_jump {
            match self.step {
                0 => ("surface-up", Verb::JumpOnto, Some("living_sofa"), None),
                2 if !self.surface_up_only => ("surface-down", Verb::JumpDown, None, None),
                _ => return (None, true),
            }
        } else if self.walkthrough {
            match self.step {
                0 => ("tour-shelf", Verb::Walk, Some("shelf"), None),
                1 => ("tour-sofa", Verb::Walk, Some("sofa"), None),
                2 => ("tour-desk", Verb::Walk, Some("desk"), None),
                3 => ("mug-grab", Verb::Grab, Some("mug"), Some(Hand::Right)),
                4 => ("carry-to-chair", Verb::Walk, Some("chair"), None),
                5 => ("sit-with-mug", Verb::Sit, Some("chair"), None),
                _ => return (None, true),
            }
        } else {
            match self.step {
                0 => ("initial-sit", Verb::Sit, Some("chair"), None),
                2 => ("seated-wave", Verb::Wave, None, Some(Hand::Right)),
                3 => ("stand", Verb::StandUp, None, None),
                4 => ("table", Verb::Walk, Some("desk"), None),
                5 => ("bend-return", Verb::BendAndReturn, Some("desk"), None),
                6 => ("mug-grab", Verb::Grab, Some("mug"), Some(Hand::Right)),
                7 => ("carry-to-chair", Verb::Walk, Some("chair"), None),
                8 => ("sit-with-mug", Verb::Sit, Some("chair"), None),
                _ => return (None, true),
            }
        };
        if !self.sent {
            self.sent = true;
            return (
                Some(super::action(action.0, action.1, action.2, action.3)),
                false,
            );
        }
        if state
            .events
            .iter()
            .any(|e| e.action_id == action.0 && e.status == Status::Completed)
        {
            self.step += 1;
            self.sent = false;
            // Dispatch the newly eligible action now. Waiting another client
            // tick would submit a transient idle motion request between actions.
            return self.tick(state);
        }
        (None, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_render_plugin::cc_render::office::controller::{OfficeController, ActionEvent};

    #[test]
    fn completed_wave_dispatches_next_action_without_an_idle_tick() {
        let mut script = Choreography::new();
        script.step = 2;
        script.sent = true;
        script.walkthrough = false;
        let mut state = OfficeController::default().snapshot();
        state.events.push(ActionEvent {
            sequence: 1, action_id: "seated-wave".into(), status: Status::Completed, detail: String::new(),
        });
        let (command, done) = script.tick(&state);
        assert!(!done);
        assert!(matches!(command, Some(OfficeCommand::PerformAction { request }) if request.verb == Verb::StandUp));
        assert!(script.tick(&state).0.is_none(), "do not duplicate the submitted action");
    }
}
