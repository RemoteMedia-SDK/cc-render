//! Scene-only weather requests and observed state. No avatar or motion channels.
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[derive(bevy::prelude::Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherPreset {
    #[default]
    Clear,
    Overcast,
    Rain,
    Fog,
    Snow,
    Night,
}

impl std::str::FromStr for WeatherPreset {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "clear" => Ok(Self::Clear), "overcast" => Ok(Self::Overcast),
            "rain" => Ok(Self::Rain), "fog" => Ok(Self::Fog),
            "snow" => Ok(Self::Snow), "night" => Ok(Self::Night),
            _ => anyhow::bail!("weather must be clear, overcast, rain, fog, snow or night"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WeatherRequest {
    pub id: u64,
    pub target: WeatherPreset,
    pub seconds: f32,
}

impl WeatherRequest {
    pub fn new(id: u64, target: WeatherPreset, seconds: f32) -> anyhow::Result<Self> {
        anyhow::ensure!(seconds.is_finite() && (0.0..=600.0).contains(&seconds),
                        "weather transition duration must be finite and within 0..600 seconds");
        Ok(Self { id, target, seconds })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct WeatherSnapshot {
    pub request_id: u64,
    pub target: WeatherPreset,
    pub progress: f32,
    pub elapsed_seconds: f64,
    pub cloud_cover: f32,
    pub night: f32,
    pub wind: f32,
    pub wind_time: f32,
    pub rain: f32,
    /// Decorative exterior water film, retained while surfaces dry after rain.
    pub surface_wetness: f32,
    pub snow: f32,
    pub sun_illuminance: f32,
    pub environment_intensity: f32,
    pub exposure_ev100: f32,
    pub fog_start: f32,
    pub fog_end: f32,
}

#[derive(bevy::prelude::Resource)]
pub(crate) struct WeatherBridge {
    pub requests: watch::Receiver<Option<WeatherRequest>>,
    pub snapshots: watch::Sender<Option<WeatherSnapshot>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weather_requests_reject_invalid_durations() {
        for seconds in [-1.0, f32::NAN, f32::INFINITY, 601.0] {
            assert!(WeatherRequest::new(1, WeatherPreset::Rain, seconds).is_err());
        }
        for seconds in [0.0, 2.0, 600.0] {
            assert!(WeatherRequest::new(1, WeatherPreset::Rain, seconds).is_ok());
        }
    }
}
