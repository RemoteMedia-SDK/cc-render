//! A single content-time clock and atomic physics tick for all live cloth layers.
use super::*;
use crate::cc_render::cloth_solver::{ClothBatch, ClothLayerStep};

#[derive(Resource, Default)]
pub(super) struct ClothSimulation {
    batch: Option<ClothBatch>,
    ids: Vec<String>,
}

impl ClothSimulation {
    fn prepare(&mut self, layers: &[&mut LiveCloth]) -> Result<(), String> {
        let ids: Vec<_> = layers.iter().map(|l| l.id.clone()).collect();
        if self.batch.is_some() {
            if self.ids != ids {
                return Err("cloth layer set changed; reload required".into());
            }
        } else {
            if ids.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err("duplicate cloth layer id".into());
            }
            self.batch = Some(
                ClothBatch::new(&layers.iter().map(|l| &l.solver).collect::<Vec<_>>())
                    .map_err(|e| e.to_string())?,
            );
            self.ids = ids;
        }
        Ok(())
    }

    pub(super) fn advance(
        &mut self,
        layers: &mut [&mut LiveCloth],
        office: Option<&OfficeController>,
        config: Option<&RapierConfiguration>,
    ) -> Result<(), String> {
        if layers.is_empty() {
            return Ok(());
        }
        if let Some(error) = layers.iter().find_map(|l| l.error.as_ref()) {
            return Err(error.clone());
        }
        self.prepare(layers)?;
        if layers.iter().any(|l| l.clock != layers[0].clock) {
            return Err("cloth layer clocks diverged; reload required".into());
        }
        let (Some(office), Some(config)) = (office, config) else {
            return Ok(());
        };
        if office.motion_error.is_some() {
            return Ok(());
        }
        let Some(stream) = &office.observed.native_stream else {
            return Ok(());
        };
        let mut clock = layers[0].clock.clone();
        let target = clock.observe(&stream.session_id, office.observed.displayed_pts_ms)?;
        for layer in layers.iter_mut() {
            layer.clock = clock.clone();
        }
        self.catch_up(layers, target, config.gravity.as_dvec3())
    }

    fn catch_up(
        &mut self,
        layers: &mut [&mut LiveCloth],
        target: u64,
        gravity: DVec3,
    ) -> Result<(), String> {
        let batch = self.batch.as_mut().expect("prepared cloth batch");
        while layers[0].clock.ticks < target {
            let mut step: Vec<_> = layers
                .iter_mut()
                .map(|layer| ClothLayerStep {
                    solver: &mut layer.solver,
                    colliders: &layer.colliders,
                    grips: &mut [],
                })
                .collect();
            batch
                .step(&mut step, 1. / 60., gravity)
                .map_err(|e| e.to_string())?;
            for layer in layers.iter_mut() {
                layer.clock.ticks += 1;
            }
        }
        Ok(())
    }
}
