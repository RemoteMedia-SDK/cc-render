//! Lagrangian shower parcels: gravity, drag, swept skin impacts, surface runoff,
//! detachment and drain absorption. Avatar motion is an immutable boundary.
use super::water_collision::{WaterSurfaces, RADIUS};
use crate::cc_render::scene_water::{WaterFixture, WaterKind};
use bevy::math::DVec3;

const GRAVITY: DVec3 = DVec3::new(0., -9.81, 0.);

#[derive(Clone)]
struct Film {
    topology_revision: u64,
    face: usize,
    weights: [f64; 3],
    normal: DVec3,
    age: f64,
}
#[derive(Clone, Default)]
pub(super) struct Parcel {
    pub position: DVec3,
    pub velocity: DVec3,
    pub active: bool,
    age: f64,
    bounces: u8,
    film: Option<Film>,
}
impl Parcel {
    pub fn runoff(&self) -> bool {
        self.active && self.film.is_some()
    }
}

pub(super) struct WaterSimulation {
    pub particles: Vec<Parcel>,
    pending: f64,
    sequence: u64,
    pub body_impacts: u64,
    pub drain_impacts: u64,
    pub dropped_emissions: u64,
    pub unresolved_collisions: u64,
    pub elapsed_ms: f64,
}

impl WaterSimulation {
    pub fn new(kind: WaterKind) -> Self {
        Self {
            particles: vec![Parcel::default(); if kind == WaterKind::Shower { 2048 } else { 512 }],
            pending: 0.,
            sequence: 0,
            body_impacts: 0,
            drain_impacts: 0,
            dropped_emissions: 0,
            unresolved_collisions: 0,
            elapsed_ms: 0.,
        }
    }
    pub fn advance(
        &mut self,
        fixture: &WaterFixture,
        flow: f32,
        dt: f64,
        surfaces: &WaterSurfaces,
    ) {
        let started = std::time::Instant::now();
        self.elapsed_ms = 0.;
        if dt <= 0. || surfaces.error.is_some() {
            return;
        }
        let steps = (dt * 120.).ceil().max(1.) as usize;
        let h = dt / steps as f64;
        for step in 0..steps {
            let from = step as f64 / steps as f64;
            let to = (step + 1) as f64 / steps as f64;
            self.emit(fixture, flow, h);
            let step = Step {
                dt: h,
                from,
                to,
                height: fixture.surface_y as f64,
                sequence: self.sequence,
            };
            let results = step_parallel(
                &mut self.particles,
                step,
                surfaces,
                bevy::tasks::ComputeTaskPool::try_get().map(|pool| &**pool),
            );
            for result in results {
                self.body_impacts += result.body;
                self.drain_impacts += result.drain;
                if self.unresolved_collisions == 0 {
                    if let Some(reason) = &result.error {
                        tracing::warn!(%reason,"water parcel query unresolved");
                    }
                }
                self.unresolved_collisions += result.unresolved;
            }
        }
        self.elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
    }
    fn emit(&mut self, fixture: &WaterFixture, flow: f32, dt: f64) {
        let rate = if fixture.kind == WaterKind::Shower {
            500.
        } else {
            160.
        };
        if flow == 0. {
            self.pending = 0.;
            return;
        }
        self.pending += dt * rate * flow as f64;
        let count = self.pending.floor() as usize;
        self.pending -= count as f64;
        let mut emitted = 0;
        for particle in self.particles.iter_mut().filter(|p| !p.active).take(count) {
            let nozzle = self.sequence % 37;
            let angle = nozzle as f64 * 2.399963;
            let radius = fixture.spray_radius_m as f64 * ((nozzle as f64 + 0.5) / 37.).sqrt();
            *particle = Parcel {
                active: true,
                position: DVec3::from_array(fixture.outlet.map(f64::from))
                    + DVec3::new(radius * angle.cos(), 0., radius * angle.sin()),
                velocity: DVec3::new(0., -0.6, 0.),
                ..Default::default()
            };
            self.sequence += 1;
            emitted += 1;
        }
        self.dropped_emissions += (count - emitted) as u64;
    }
}

#[derive(Clone, Copy)]
struct Step {
    dt: f64,
    from: f64,
    to: f64,
    height: f64,
    sequence: u64,
}

#[derive(Default)]
struct Counts {
    body: u64,
    drain: u64,
    unresolved: u64,
    error: Option<String>,
}

fn step_parallel(
    particles: &mut [Parcel],
    step: Step,
    surfaces: &WaterSurfaces,
    pool: Option<&bevy::tasks::TaskPool>,
) -> Vec<Counts> {
    let Some(pool) = pool.filter(|_| !surfaces.triangles.is_empty()) else {
        return vec![step_particles(particles, step, surfaces)];
    };
    pool.scope(|scope| {
        for batch in particles
            .chunks_mut(64)
            .filter(|b| b.iter().any(|p| p.active))
        {
            scope.spawn(async move { step_particles(batch, step, surfaces) });
        }
    })
}

fn step_particles(particles: &mut [Parcel], step: Step, surfaces: &WaterSurfaces) -> Counts {
    let mut count = Counts::default();
    for p in particles.iter_mut().filter(|p| p.active) {
        p.age += step.dt;
        if p.age > 3. {
            p.active = false;
            continue;
        }
        match move_parcel(p, step.dt, step.from, step.to, surfaces) {
            Ok(hit) => count.body += u64::from(hit),
            Err(error) => {
                p.active = false;
                count.unresolved += 1;
                if count.error.is_none() {
                    count.error = Some(format!("{error:#}"));
                }
                continue;
            }
        }
        count.drain += u64::from(receive(p, step.height, step.sequence));
    }
    count
}

fn receive(p: &mut Parcel, height: f64, sequence: u64) -> bool {
    if p.position.y > height + RADIUS {
        return false;
    }
    if p.bounces > 0 || p.velocity.length() < 0.3 {
        p.active = false;
        return true;
    }
    p.position.y = height + RADIUS;
    let angle = (sequence as f64 + p.age) * 2.399963;
    p.velocity = DVec3::new(
        angle.cos() * 0.13,
        (-p.velocity.y * 0.08).min(0.65),
        angle.sin() * 0.13,
    );
    p.film = None;
    p.bounces += 1;
    true
}

fn move_parcel(
    p: &mut Parcel,
    dt: f64,
    from: f64,
    to: f64,
    surfaces: &WaterSurfaces,
) -> anyhow::Result<bool> {
    if let Some(film) = p.film.take() {
        if film.topology_revision == surfaces.topology_revision
            && film.face < surfaces.triangles.len()
            && film.age < 1.5
        {
            let old = surfaces.point(film.face, film.weights, from);
            let new = surfaces.point(film.face, film.weights, to);
            let relative = p.velocity + (GRAVITY - film.normal * GRAVITY.dot(film.normal)) * dt;
            let target = new + relative * dt;
            if GRAVITY.dot(film.normal) < 3.0 {
                if let Some(hit) = surfaces.project(film.face, target, film.normal, to) {
                    p.position =
                        surfaces.point(hit.face, hit.weights, to) + hit.normal * (RADIUS + 0.0001);
                    p.velocity =
                        (relative - hit.normal * relative.dot(hit.normal)) * (-2. * dt).exp();
                    p.film = Some(Film {
                        topology_revision: surfaces.topology_revision,
                        face: hit.face,
                        weights: hit.weights,
                        normal: hit.normal,
                        age: film.age + dt,
                    });
                    return Ok(false);
                }
            }
            p.position = new + film.normal * (RADIUS + 0.0002);
            p.velocity = relative + (new - old) / dt;
        }
    }
    let velocity = (p.velocity + GRAVITY * dt) / (1. + 0.12 * p.velocity.length() * dt);
    let end = p.position + velocity * dt;
    if let Some(hit) = surfaces.sweep(p.position, end, from, to)? {
        let at = from + (to - from) * hit.fraction;
        let old = surfaces.point(hit.face, hit.weights, from);
        let new = surfaces.point(hit.face, hit.weights, to);
        let surface_velocity = (new - old) / dt;
        let relative = velocity - surface_velocity;
        p.position = surfaces.point(hit.face, hit.weights, at) + hit.normal * (RADIUS + 0.0001);
        // Normal momentum is absorbed by the kinematic surface; tangential
        // momentum and gravity drive the liquid film. No force is sent to bones.
        p.velocity = (relative - hit.normal * relative.dot(hit.normal)) * 0.35;
        p.film = Some(Film {
            topology_revision: surfaces.topology_revision,
            face: hit.face,
            weights: hit.weights,
            normal: hit.normal,
            age: 0.,
        });
        p.position = new + hit.normal * (RADIUS + 0.0001);
        Ok(true)
    } else {
        p.position = end;
        p.velocity = velocity;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::super::water_collision::bounds;
    use super::*;
    fn plane(x: f64) -> (Vec<DVec3>, Vec<[usize; 3]>) {
        (
            vec![
                DVec3::new(x, -1., -1.),
                DVec3::new(x, 2., -1.),
                DVec3::new(x, 2., 1.),
                DVec3::new(x, -1., 1.),
            ],
            vec![[0, 1, 2], [0, 2, 3]],
        )
    }
    fn region() -> parry_ccd::bounding_volume::Aabb {
        bounds([DVec3::splat(-10.), DVec3::splat(10.)], 0.)
    }
    #[test]
    fn water_parallel_batches_match_serial_contacts_and_runoff() {
        let mut surfaces = WaterSurfaces::default();
        let (p, t) = plane(0.);
        surfaces.update(p, t, &[region()]).unwrap();
        let mut serial = vec![
            Parcel {
                active: true,
                position: DVec3::new(-0.01, 1., 0.),
                velocity: DVec3::X * 2.,
                ..Default::default()
            };
            256
        ];
        let mut parallel = serial.clone();
        let pool = bevy::tasks::TaskPoolBuilder::new().num_threads(4).build();
        for _ in 0..120 {
            let step = Step {
                dt: 1. / 120.,
                from: 0.,
                to: 1.,
                height: -2.,
                sequence: 1,
            };
            let a = step_parallel(&mut serial, step, &surfaces, None);
            let b = step_parallel(&mut parallel, step, &surfaces, Some(&pool));
            let totals = |items: &[Counts]| {
                items.iter().fold([0, 0, 0], |sum, c| {
                    [sum[0] + c.body, sum[1] + c.drain, sum[2] + c.unresolved]
                })
            };
            assert_eq!(totals(&a), totals(&b));
            for (a, b) in serial.iter().zip(&parallel) {
                assert_eq!(a.position, b.position);
                assert_eq!(a.velocity, b.velocity);
                assert_eq!(a.runoff(), b.runoff());
                assert_eq!(a.active, b.active);
            }
        }
    }
    #[test]
    fn water_separating_contact_does_not_reattach_to_the_same_surface() {
        let mut surfaces = WaterSurfaces::default();
        let (p, t) = plane(0.);
        surfaces.update(p, t, &[region()]).unwrap();
        let start = DVec3::new(-RADIUS * 0.5, 1., 0.);
        assert!(surfaces
            .sweep(start, start - DVec3::X * 0.1, 0., 1.)
            .unwrap()
            .is_none());
        assert!(surfaces
            .sweep(start, start + DVec3::X * 0.1, 0., 1.)
            .unwrap()
            .is_some());
    }
    #[test]
    fn water_degenerate_faces_do_not_hide_real_surface_impacts() {
        let mut surfaces = WaterSurfaces::default();
        let (p, mut t) = plane(0.);
        t.push([0, 0, 0]);
        surfaces.update(p, t, &[region()]).unwrap();
        assert_eq!(surfaces.degenerate_triangles, 1);
        assert!(surfaces
            .sweep(DVec3::new(-1., 0., 0.), DVec3::new(1., 0., 0.), 0., 1.)
            .unwrap()
            .is_some());
    }
    #[test]
    fn water_runoff_does_not_jump_to_a_disconnected_nearby_surface() {
        let (mut p, mut t) = plane(0.);
        let (other, faces) = plane(0.001);
        let offset = p.len();
        p.extend(other);
        t.extend(faces.into_iter().map(|f| f.map(|i| i + offset)));
        let mut surfaces = WaterSurfaces::default();
        surfaces.update(p, t, &[region()]).unwrap();
        let hit = surfaces
            .project(0, DVec3::new(0.001, 1., 0.), DVec3::X, 1.)
            .unwrap();
        assert!(hit.face < 2);
    }
    #[test]
    fn water_grazing_past_an_edge_is_resolved_without_a_false_impact() {
        let (p, t) = plane(0.);
        let mut surfaces = WaterSurfaces::default();
        surfaces.update(p, t, &[region()]).unwrap();
        assert!(surfaces
            .sweep(
                DVec3::new(-1., 2.0016, 0.),
                DVec3::new(1., 2.0016, 0.),
                0.,
                1.
            )
            .unwrap()
            .is_none());
    }
    #[test]
    fn water_sweeps_fast_particle_and_moving_body_with_clear_endpoints() {
        let mut s = WaterSurfaces::default();
        let (p, t) = plane(0.);
        s.update(p.clone(), t.clone(), &[region()]).unwrap();
        let hit = s
            .sweep(DVec3::new(-1., 0., 0.), DVec3::new(1., 0., 0.), 0., 1.)
            .unwrap()
            .unwrap();
        assert!(hit.fraction > 0.49 && hit.fraction < 0.5);
        let (p, _) = plane(-1.);
        s.update(p, t.clone(), &[region()]).unwrap();
        let (p, _) = plane(1.);
        s.update(p, t, &[region()]).unwrap();
        assert!(s.sweep(DVec3::ZERO, DVec3::ZERO, 0., 1.).unwrap().is_some());
    }
    #[test]
    fn water_runoff_follows_moving_surface_and_gravity_without_changing_it() {
        let mut s = WaterSurfaces::default();
        let (p, t) = plane(0.);
        s.update(p.clone(), t.clone(), &[region()]).unwrap();
        let mut drop = Parcel {
            active: true,
            position: DVec3::new(-0.01, 1., 0.),
            velocity: DVec3::X * 2.,
            ..Default::default()
        };
        assert!(move_parcel(&mut drop, 0.01, 0., 1., &s).unwrap());
        assert!(drop.runoff());
        let y = drop.position.y;
        let (p, _) = plane(0.2);
        s.update(p.clone(), t, &[region()]).unwrap();
        move_parcel(&mut drop, 0.05, 0., 1., &s).unwrap();
        assert!((drop.position.x - (0.2 - RADIUS - 0.0001)).abs() < 1e-6);
        assert!(drop.position.y < y);
        assert_eq!(s.current, p);
    }
    #[test]
    fn water_empty_shower_drains_and_shutoff_emits_nothing() {
        let fixture = WaterFixture {
            id: "test".into(),
            kind: WaterKind::Shower,
            outlet: [0., 2.2, 0.],
            surface_y: 0.,
            spray_radius_m: 0.1,
            max_liters_per_minute: 9.,
        };
        let mut sim = WaterSimulation::new(WaterKind::Shower);
        let s = WaterSurfaces::default();
        for _ in 0..600 {
            sim.advance(&fixture, 1., 1. / 60., &s);
        }
        assert!(sim.drain_impacts > 1000);
        assert_eq!(sim.body_impacts, 0);
        assert_eq!(sim.dropped_emissions, 0);
        for _ in 0..240 {
            sim.advance(&fixture, 0., 1. / 60., &s);
        }
        assert!(sim.particles.iter().all(|p| !p.active));
        assert_eq!(sim.particles.len(), 2048);
    }
}
