//! One transactional physics tick across independently authored cloth layers.
use super::self_collision::{ContactLayer, SelfCollisionTopology};
use super::*;
use std::ops::Range;

pub(crate) struct ClothLayerStep<'a> {
    pub solver: &'a mut ClothSolver,
    pub colliders: &'a [FixedClothCollider],
    pub grips: &'a mut [ClothGrip],
}

struct Layout {
    range: Range<usize>,
    triangles: Vec<[usize; 3]>,
}

pub(crate) struct ClothBatch {
    layout: Vec<Layout>,
    triangles: Vec<[usize; 3]>,
    contacts: SelfCollisionTopology,
}

impl ClothBatch {
    pub(crate) fn new(solvers: &[&ClothSolver]) -> Result<Self, ClothError> {
        if solvers.is_empty() {
            return Err(ClothError::InvalidInput);
        }
        let mut layout = Vec::new();
        let mut triangles = Vec::new();
        let mut offset = 0;
        for solver in solvers {
            triangles.extend(solver.triangles.iter().map(|t| t.map(|i| i + offset)));
            let end = offset + solver.positions.len();
            layout.push(Layout {
                range: offset..end,
                triangles: solver.triangles.clone(),
            });
            offset = end;
        }
        Ok(Self {
            contacts: SelfCollisionTopology::new(&triangles),
            layout,
            triangles,
        })
    }

    fn validate(
        &self,
        layers: &[ClothLayerStep<'_>],
        dt: f64,
        gravity: DVec3,
    ) -> Result<(), ClothError> {
        if !dt.is_finite()
            || !(0.0..=1. / 30.).contains(&dt)
            || dt == 0.
            || !gravity.is_finite()
            || layers.len() != self.layout.len()
        {
            return Err(ClothError::InvalidInput);
        }
        for (layer, layout) in layers.iter().zip(&self.layout) {
            if layer.solver.positions.len() != layout.range.len()
                || layer.solver.triangles != layout.triangles
            {
                return Err(ClothError::InvalidInput);
            }
        }
        Ok(())
    }

    pub(crate) fn step(
        &mut self,
        layers: &mut [ClothLayerStep<'_>],
        dt: f64,
        gravity: DVec3,
    ) -> Result<(), ClothError> {
        self.validate(layers, dt, gravity)?;
        // Every material gets at least its requested integration resolution.
        let substeps = layers
            .iter()
            .map(|l| l.solver.parameters.substeps)
            .max()
            .unwrap();
        let h = dt / substeps as f64;
        let mut positions: Vec<_> = layers
            .iter()
            .flat_map(|l| l.solver.positions.iter().copied())
            .collect();
        let mut velocities: Vec<_> = layers
            .iter()
            .flat_map(|l| l.solver.velocities.iter().copied())
            .collect();
        let inverse_mass: Vec<_> = layers
            .iter()
            .flat_map(|l| l.solver.inverse_mass.iter().copied())
            .collect();
        let mut previous = positions.clone();
        let mut grips: Vec<_> = layers.iter().map(|l| l.grips.to_vec()).collect();
        for (layer, pending) in layers.iter().zip(&mut grips) {
            for grip in pending {
                grip.prepare(&layer.solver.surface)?;
            }
        }
        let contacts: Vec<_> = layers
            .iter()
            .zip(&self.layout)
            .map(|(layer, layout)| ContactLayer {
                range: layout.range.clone(),
                triangles: &layer.solver.triangles,
                radius: layer.solver.parameters.thickness_m as f64 * 0.5,
                fixed: layer.colliders,
            })
            .collect();
        for substep in 0..substeps {
            previous.copy_from_slice(&positions);
            let interval = [substep as f64, (substep + 1) as f64].map(|s| s / substeps as f64);
            for ((layer, layout), pending) in layers.iter().zip(&self.layout).zip(&mut grips) {
                let r = layout.range.clone();
                let drag = (-layer.solver.parameters.air_drag_per_second * h).exp();
                for (p, v) in positions[r.clone()]
                    .iter_mut()
                    .zip(&mut velocities[r.clone()])
                {
                    *v = *v * drag + gravity * h;
                    *p += *v * h;
                }
                layer.solver.project_constraints(
                    &mut GripParticles {
                        positions: &mut positions[r.clone()],
                        previous: &previous[r.clone()],
                        inverse_mass: &inverse_mass[r],
                        h,
                    },
                    interval,
                    pending,
                )?;
            }
            self.contacts.resolve_layers(
                &mut GripParticles {
                    positions: &mut positions,
                    previous: &previous,
                    inverse_mass: &inverse_mass,
                    h,
                },
                &self.triangles,
                &contacts,
            )?;
            reconstruct_velocities(&positions, &previous, &mut velocities, h)?;
        }
        // Validate every render surface and revision before publishing any one.
        let surfaces = layers
            .iter()
            .zip(&self.layout)
            .map(|(layer, layout)| {
                layer.solver.surface.updated(
                    positions[layout.range.clone()]
                        .iter()
                        .map(|p| p.as_vec3())
                        .collect(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (((layer, layout), surface), pending) in layers
            .iter_mut()
            .zip(&self.layout)
            .zip(surfaces)
            .zip(&mut grips)
        {
            layer
                .solver
                .positions
                .copy_from_slice(&positions[layout.range.clone()]);
            layer
                .solver
                .velocities
                .copy_from_slice(&velocities[layout.range.clone()]);
            layer.solver.surface = surface;
            for grip in pending.iter_mut() {
                grip.finish();
            }
            layer.grips.clone_from_slice(pending);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "cloth_batch_tests.rs"]
mod tests;
