//! Test-only measurements of the unchanged batch solve. These are physical
//! energy estimates, not extra forces or a settling acceptance override.
use super::*;
use std::io::{BufWriter, Write};

pub(super) struct EnergyProbe {
    file: Option<BufWriter<std::fs::File>>,
    tick: usize,
    elapsed: f64,
    dt: f64,
    h: f64,
    gravity: DVec3,
}

impl EnergyProbe {
    pub(super) fn from_environment() -> Self {
        let file = std::env::var_os("CC_CLOTH_ENERGY_TRACE").map(|path| {
            BufWriter::new(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .expect("create a unique energy trace for one diagnostic batch"),
            )
        });
        Self {
            file,
            tick: 0,
            elapsed: 0.,
            dt: 0.,
            h: 0.,
            gravity: DVec3::ZERO,
        }
    }

    pub(super) fn begin_tick(&mut self, dt: f64, substeps: usize, gravity: DVec3) {
        self.tick += 1;
        self.dt = dt;
        self.h = dt / substeps as f64;
        self.gravity = gravity;
    }

    pub(super) fn record(
        &mut self,
        phase: (&str, usize, Option<usize>),
        layers: &[ClothLayerStep<'_>],
        positions: &[DVec3],
        velocity: impl Fn(usize) -> DVec3,
    ) {
        if self.file.is_none() {
            return;
        }
        let (stage, substep, iteration) = phase;
        let mut offset = 0;
        let mut values = Vec::new();
        for layer in layers {
            let solver = &layer.solver;
            let local = &positions[offset..offset + solver.positions.len()];
            let velocities: Vec<_> = (0..local.len()).map(|i| velocity(offset + i)).collect();
            values.push(measure(
                solver,
                layer.colliders,
                local,
                &velocities,
                self.gravity,
            ));
            offset += local.len();
        }
        writeln!(
            self.file.as_mut().unwrap(),
            "{}",
            serde_json::json!({
                "tick":self.tick,"substep":substep,"stage":stage,"iteration":iteration,
                "interval_start_s":self.elapsed + substep as f64 * self.h,
                "interval_end_s":self.elapsed + (substep+1) as f64 * self.h,"layers":values,
            })
        )
        .expect("write energy diagnostic");
    }

    pub(super) fn finish_tick(&mut self) {
        self.elapsed += self.dt;
        if let Some(file) = &mut self.file {
            file.flush().expect("flush energy diagnostic");
        }
    }
}

fn measure(
    solver: &ClothSolver,
    colliders: &[FixedClothCollider],
    positions: &[DVec3],
    velocities: &[DVec3],
    gravity: DVec3,
) -> serde_json::Value {
    let kinetic: f64 = velocities
        .iter()
        .zip(&solver.inverse_mass)
        .map(|(v, w)| 0.5 * v.length_squared() / w)
        .sum();
    let potential: f64 = positions
        .iter()
        .zip(&solver.inverse_mass)
        .map(|(p, w)| -gravity.dot(*p) / w)
        .sum();
    let mut stretch = Some(0.);
    let mut bend = Some(0.);
    let mut residual_max: f64 = 0.;
    for spring in &solver.springs {
        let residual =
            positions[spring.vertices[0]].distance(positions[spring.vertices[1]]) - spring.rest;
        residual_max = residual_max.max(residual.abs());
        let energy = if spring.stretch {
            &mut stretch
        } else {
            &mut bend
        };
        *energy = energy.and_then(|e| {
            (spring.compliance > 0.).then(|| e + 0.5 * residual * residual / spring.compliance)
        });
    }
    let (vertex, speed) = velocities
        .iter()
        .enumerate()
        .map(|(i, v)| (i, v.length()))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    let support_distance = colliders
        .iter()
        .map(|c| c.distance_and_normal(positions[vertex]).0)
        .reduce(f64::min);
    serde_json::json!({
        "kinetic_j":kinetic,"gravity_j":potential,"stretch_j":stretch,"bend_j":bend,
        "total_j":stretch.zip(bend).map(|(s,b)| kinetic + potential + s + b),
        "max_spring_residual_m":residual_max,"max_speed_m_s":speed,
        "max_speed_vertex":vertex,"max_speed_position":positions[vertex].to_array(),
        "max_speed_velocity":velocities[vertex].to_array(),"max_speed_support_distance_m":support_distance,
    })
}

#[test]
#[ignore = "diagnostic: select authored layers and an energy trace; does not certify full bedding"]
fn probe_authored_bedding_substep_energy() {
    let (mut solvers, colliders) = load_layers();
    let ticks: usize = std::env::var("CC_CLOTH_PROBE_TICKS")
        .unwrap_or_else(|_| "120".into())
        .parse()
        .unwrap();
    assert!((1..=600).contains(&ticks));
    let mut batch = ClothBatch::new(&solvers.iter().collect::<Vec<_>>()).unwrap();
    let gravity = bevy_rapier3d::prelude::RapierConfiguration::new(1.)
        .gravity
        .as_dvec3();
    eprintln!("ENERGY_PROBE_GRAVITY {:?}", gravity);
    let start = std::time::Instant::now();
    for tick in 1..=ticks {
        let mut layers: Vec<_> = solvers
            .iter_mut()
            .zip(&colliders)
            .map(|(solver, colliders)| ClothLayerStep {
                solver,
                colliders,
                grips: &mut [],
            })
            .collect();
        batch.step(&mut layers, 1. / 60., gravity).unwrap();
        if tick % 10 == 0 || tick == ticks {
            eprintln!(
                "ENERGY_PROBE_TICK {tick} wall_seconds={:.3} speeds={:?}",
                start.elapsed().as_secs_f64(),
                solvers
                    .iter()
                    .map(ClothSolver::max_speed)
                    .collect::<Vec<_>>()
            );
        }
    }
    let hashes: Vec<_> = solvers
        .iter()
        .map(|s| {
            let bytes: Vec<_> = s
                .surface()
                .positions()
                .iter()
                .flat_map(|p| p.to_array())
                .flat_map(f32::to_le_bytes)
                .collect();
            hmac_sha256::Hash::hash(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        })
        .collect();
    eprintln!("ENERGY_PROBE_FINAL_HASHES {}", serde_json::json!(hashes));
}

fn load_layers() -> (Vec<ClothSolver>, Vec<Vec<FixedClothCollider>>) {
    use crate::cc_render::scene_package::ScenePackage;
    let package = ScenePackage::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scenes/toronto_condo/scene-bedding.json"),
    )
    .unwrap();
    let selected =
        std::env::var("CC_CLOTH_PROBE_LAYERS").unwrap_or_else(|_| "bed_fitted_sheet".into());
    let mut definitions: Vec<_> = package
        .cloths
        .iter()
        .filter(|c| selected.split(',').any(|id| id == c.id))
        .collect();
    definitions.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        definitions.len(),
        selected.split(',').count(),
        "unknown or duplicate layer selection"
    );
    let solvers: Vec<_> = definitions.iter().map(|d| d.solver().unwrap()).collect();
    let colliders: Vec<_> = definitions
        .iter()
        .map(|d| d.colliders(&package).unwrap())
        .collect();
    eprintln!(
        "ENERGY_PROBE_ASSETS {}",
        serde_json::json!(definitions
            .iter()
            .map(|d| serde_json::json!({"id":d.id,"sha256":d.sha256,"parameters":d.parameters}))
            .collect::<Vec<_>>())
    );
    (solvers, colliders)
}
