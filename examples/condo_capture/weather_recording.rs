//! Capture-relative, reproducible weather commands; never supplies avatar poses.
use cc_render_plugin::cc_render::{CcRenderer, scene_weather::WeatherPreset};
use serde::Deserialize;
use std::{io::Write, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    at_seconds: f32,
    weather: WeatherPreset,
    transition_seconds: f32,
}

fn validate(events: &[Event], duration: f32) -> anyhow::Result<()> {
    let mut previous = -1.0;
    for event in events {
        anyhow::ensure!(event.at_seconds.is_finite() && event.at_seconds >= 0.0 && event.at_seconds > previous,
                        "weather events must have finite, nonnegative, strictly increasing start times");
        anyhow::ensure!(event.transition_seconds.is_finite() && (0.0..=600.0).contains(&event.transition_seconds),
                        "weather transition duration outside 0..600 seconds");
        anyhow::ensure!(event.at_seconds + event.transition_seconds <= duration,
                        "weather transition extends beyond the capture");
        previous = event.at_seconds;
    }
    Ok(())
}

pub(super) struct WeatherCapture {
    events: Vec<Event>,
    next: usize,
    requests: std::fs::File,
    states: std::fs::File,
}

impl WeatherCapture {
    pub(super) fn new(out: &Path, frames: usize) -> anyhow::Result<Self> {
        let events = if let Some(path) = std::env::var_os("CC_CONDO_WEATHER_SEQUENCE") {
            let bytes = std::fs::read(path)?;
            let events: Vec<Event> = serde_json::from_slice(&bytes)?;
            validate(&events, frames as f32 / 30.0)?;
            std::fs::write(out.join("weather-sequence.json"), bytes)?;
            events
        } else { Vec::new() };
        Ok(Self { events, next: 0,
            requests: std::fs::File::create(out.join("weather-requests.ndjson"))?,
            states: std::fs::File::create(out.join("weather-states.ndjson"))? })
    }

    pub(super) fn advance(&mut self, renderer: &CcRenderer, frame: usize) -> anyhow::Result<()> {
        while let Some(event) = self.events.get(self.next).filter(|event| event.at_seconds <= frame as f32 / 30.0) {
            let id = renderer.transition_weather(event.weather, event.transition_seconds)?;
            writeln!(self.requests, "{}", serde_json::json!({"request_id": id, "encoded_frame": frame,
                "weather": event.weather, "transition_seconds": event.transition_seconds,
                "at_seconds": event.at_seconds}))?;
            self.next += 1;
        }
        Ok(())
    }

    pub(super) fn observe(&mut self, renderer: &CcRenderer, frame: usize) -> anyhow::Result<()> {
        writeln!(self.states, "{}", serde_json::json!({"encoded_frame": frame, "weather": renderer.weather_state()}))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequences_allow_supersession_but_reject_invalid_or_unrecorded_events() {
        let mut events = vec![Event { at_seconds: 2.0, weather: WeatherPreset::Rain, transition_seconds: 10.0 },
            Event { at_seconds: 6.0, weather: WeatherPreset::Night, transition_seconds: 3.0 }];
        assert!(validate(&events, 20.0).is_ok());
        events[1].at_seconds = 1.0;
        assert!(validate(&events, 20.0).is_err());
        events[1].at_seconds = 19.0;
        assert!(validate(&events, 20.0).is_err());
        events[1].at_seconds = f32::NAN;
        assert!(validate(&events, 20.0).is_err());
        assert!(serde_json::from_str::<Event>(r#"{"at_seconds":2,"weather":"rain","transition_seconds":3,"root":0}"#).is_err());
    }
}
