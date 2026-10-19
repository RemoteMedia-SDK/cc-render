//! Authored plumbing outlets and scene-only controls. No avatar motion channels.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WaterKind {
    Faucet,
    Shower,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaterFixture {
    pub id: String,
    pub kind: WaterKind,
    /// Outlet centre in package world metres, Y up.
    pub outlet: [f32; 3],
    /// Authored receiving surface, not the conservative navigation collider.
    pub surface_y: f32,
    pub spray_radius_m: f32,
    pub max_liters_per_minute: f32,
}

pub(crate) fn validate(fixtures: &[WaterFixture]) -> Result<()> {
    ensure!(fixtures.len() <= 16, "water fixture capacity exceeded");
    let mut ids = HashSet::new();
    for fixture in fixtures {
        ensure!(
            !fixture.id.is_empty() && ids.insert(&fixture.id),
            "duplicate/empty water fixture id"
        );
        ensure!(
            fixture.outlet.iter().all(|v| v.is_finite())
                && fixture.surface_y.is_finite()
                && (0.05..=3.0).contains(&(fixture.outlet[1] - fixture.surface_y)),
            "invalid water fall height"
        );
        ensure!(
            fixture.spray_radius_m.is_finite()
                && (0.0..=0.3).contains(&fixture.spray_radius_m)
                && fixture.max_liters_per_minute.is_finite()
                && (0.1..=30.0).contains(&fixture.max_liters_per_minute),
            "invalid water flow/spray"
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WaterSetting {
    pub revision: u64,
    pub flow: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct WaterSnapshot {
    pub id: String,
    pub revision: u64,
    /// Applied valve opening, 0 = off, 1 = full flow.
    pub flow: f32,
    pub liters_per_minute: f32,
    pub total_liters: f64,
    pub visible_particles: usize,
    pub runoff_particles: usize,
    pub body_impacts: u64,
    pub drain_impacts: u64,
    pub dropped_emissions: u64,
    pub unresolved_collisions: u64,
    pub collision_meshes: usize,
    pub collision_triangles: usize,
    pub collision_revision: u64,
    pub collision_error: Option<String>,
    pub collision_update_ms: f64,
    pub simulation_ms: f64,
    pub degenerate_triangles: usize,
}

pub(crate) struct WaterControl {
    requests: watch::Sender<BTreeMap<String, WaterSetting>>,
    snapshots: watch::Receiver<Vec<WaterSnapshot>>,
}

#[derive(bevy::prelude::Resource)]
pub(crate) struct WaterBridge {
    pub requests: watch::Receiver<BTreeMap<String, WaterSetting>>,
    pub snapshots: watch::Sender<Vec<WaterSnapshot>>,
}

impl WaterControl {
    pub fn new(fixtures: &[WaterFixture]) -> (Self, WaterBridge) {
        let (requests, rx) = watch::channel(
            fixtures
                .iter()
                .map(|fixture| (fixture.id.clone(), WaterSetting::default()))
                .collect(),
        );
        let (tx, snapshots) = watch::channel(Vec::new());
        (
            Self {
                requests,
                snapshots,
            },
            WaterBridge {
                requests: rx,
                snapshots: tx,
            },
        )
    }

    pub fn set(&self, id: &str, flow: f32) -> Result<u64> {
        ensure!(
            flow.is_finite() && (0.0..=1.0).contains(&flow),
            "water flow must be finite and within 0..1"
        );
        ensure!(!self.requests.is_closed(), "water renderer stopped");
        let mut revision = None;
        // Keep all outlets in one bounded mailbox. Two taps changed between
        // render frames must not overwrite one another. Revisions are per tap.
        self.requests.send_if_modified(|settings| {
            let Some(setting) = settings.get_mut(id) else {
                return false;
            };
            setting.revision += 1;
            setting.flow = flow;
            revision = Some(setting.revision);
            true
        });
        revision.ok_or_else(|| anyhow::anyhow!("unknown water fixture: {id}"))
    }

    pub fn state(&self) -> Vec<WaterSnapshot> {
        self.snapshots.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixtures() -> Vec<WaterFixture> {
        serde_json::from_str::<super::super::scene_package::ScenePackage>(include_str!(
            "../../scenes/toronto_condo/scene.json"
        ))
        .unwrap()
        .water_fixtures
    }
    #[test]
    fn water_controls_are_independent_bounded_and_reject_invalid_requests() {
        let (control, bridge) = WaterControl::new(&fixtures());
        assert_eq!(control.set("shower", 1.0).unwrap(), 1);
        control.set("bathroom_faucet", 0.4).unwrap();
        control.set("kitchen_faucet", 0.7).unwrap();
        assert_eq!(bridge.requests.borrow()["shower"].flow, 1.0);
        assert_eq!(bridge.requests.borrow()["bathroom_faucet"].flow, 0.4);
        assert_eq!(bridge.requests.borrow()["kitchen_faucet"].flow, 0.7);
        assert_eq!(control.set("shower", 0.0).unwrap(), 2);
        for flow in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            assert!(control.set("shower", flow).is_err());
        }
        assert!(control.set("missing", 1.0).is_err());
        assert_eq!(bridge.requests.borrow().len(), 3);
        assert_eq!(bridge.requests.borrow()["shower"].flow, 0.0);
        drop(bridge);
        assert!(control.set("shower", 1.0).is_err());
    }
    #[test]
    fn water_manifest_rejects_invalid_geometry_duplicates_and_unbounded_pools() {
        let original = fixtures();
        assert_eq!(original.len(), 3);
        validate(&original).unwrap();
        let mut bad = original.clone();
        bad.push(bad[0].clone());
        assert!(validate(&bad).is_err());
        for height in [f32::NAN, f32::INFINITY, original[0].outlet[1], -100.0] {
            bad = original.clone();
            bad[0].surface_y = height;
            assert!(validate(&bad).is_err());
        }
        bad = original.clone();
        bad[0].max_liters_per_minute = -1.0;
        assert!(validate(&bad).is_err());
        bad = (0..17)
            .map(|index| {
                let mut fixture = original[0].clone();
                fixture.id = format!("tap_{index}");
                fixture
            })
            .collect();
        assert!(validate(&bad).is_err());
        assert!(validate(&[]).is_ok(), "older packages have no plumbing");
        bad = original;
        bad[0].spray_radius_m = f32::NAN;
        assert!(validate(&bad).is_err());
    }
}
