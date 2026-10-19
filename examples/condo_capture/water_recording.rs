//! Repeatable valve commands and applied state, independent of avatar playback.
use cc_render_plugin::cc_render::CcRenderer;
use serde::Deserialize;
use std::{io::Write, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    at_seconds: f32,
    fixture: String,
    flow: f32,
}

pub(super) struct WaterCapture {
    events: Vec<Event>,
    next: usize,
    requests: std::fs::File,
    states: std::fs::File,
}

impl WaterCapture {
    pub(super) fn new(out: &Path, frames: usize, renderer: &CcRenderer) -> anyhow::Result<Self> {
        let events = if let Some(path) = std::env::var_os("CC_CONDO_WATER_SEQUENCE") {
            let bytes = std::fs::read(path)?;
            let events: Vec<Event> = serde_json::from_slice(&bytes)?;
            let ids: Vec<_> = renderer.water_state().into_iter().map(|s| s.id).collect();
            let mut previous = 0.0;
            for event in &events {
                anyhow::ensure!(
                    event.at_seconds.is_finite()
                        && event.at_seconds >= previous
                        && event.at_seconds < frames as f32 / 30.0,
                    "water events must be ordered and within capture"
                );
                anyhow::ensure!(
                    event.flow.is_finite() && (0.0..=1.0).contains(&event.flow),
                    "invalid water flow"
                );
                anyhow::ensure!(
                    ids.contains(&event.fixture),
                    "unknown water fixture {}",
                    event.fixture
                );
                previous = event.at_seconds;
            }
            std::fs::write(out.join("water-sequence.json"), bytes)?;
            events
        } else {
            Vec::new()
        };
        Ok(Self {
            events,
            next: 0,
            requests: std::fs::File::create(out.join("water-requests.ndjson"))?,
            states: std::fs::File::create(out.join("water-states.ndjson"))?,
        })
    }
    pub(super) fn advance(&mut self, renderer: &CcRenderer, frame: usize) -> anyhow::Result<()> {
        while let Some(event) = self
            .events
            .get(self.next)
            .filter(|e| e.at_seconds <= frame as f32 / 30.0)
        {
            let revision = renderer.set_water_flow(&event.fixture, event.flow)?;
            writeln!(
                self.requests,
                "{}",
                serde_json::json!({"fixture":event.fixture,"flow":event.flow,
                "revision":revision,"encoded_frame":frame,"at_seconds":event.at_seconds})
            )?;
            self.next += 1;
        }
        Ok(())
    }
    pub(super) fn observe(&mut self, renderer: &CcRenderer, frame: usize) -> anyhow::Result<()> {
        writeln!(
            self.states,
            "{}",
            serde_json::json!({"encoded_frame":frame,"water":renderer.water_state()})
        )?;
        Ok(())
    }
}
