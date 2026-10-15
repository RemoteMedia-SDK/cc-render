//! Declared interaction phases, judged only on the rendered avatar.
//!
//! Each phase of an object's declared interaction is complete when its
//! body-to-zone relations are observed (wrists, ankles and pelvis after FK),
//! then its coupling transition is queued for the controller. Nothing here
//! authors a pose; a phase change still needs a new model revision.
use super::controller::OfficeSnapshot;
use super::interaction::{hand, observed_joint, resolve_zone, zone_distance, CouplingCommand, Interaction,
                         InteractionPhase, Relation, Require, Transition};
use bevy::prelude::*;
use serde::Serialize;

/// Hold: share of window frames inside; pair: share in range; support: consecutive frames.
pub const HOLD_SHARE: f32 = 0.9;
pub const PAIR_SHARE: f32 = 0.8;
pub const SUPPORT_FRAMES: u32 = 10;
/// Phases without relations finish once the pelvis is this close to `stand_at` and still.
pub const STAND_RADIUS_M: f32 = 0.35;
/// ...and is facing the stand facing (declared, else towards the object) within this.
pub const STAND_FACING_DEG: f32 = 30.;
const NO_RELATION_SETTLE_MS: u64 = 1000;

#[derive(Clone, Debug, Default, Serialize)]
pub struct RelationProgress {
    pub kind: &'static str,
    pub zone: String,
    pub met: bool,
    /// Frames observed inside the relation window.
    pub frames: u32,
    /// Frames satisfying the relation (touch: per joint minimum; support: current run).
    pub inside: u32,
    pub best_distance_m: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PhaseResult {
    pub phase: String,
    pub status: &'static str,
    pub elapsed_s: f32,
    pub relations: Vec<RelationProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeclaredState {
    pub interaction: String,
    pub index: usize,
    pub phase: String,
    pub results: Vec<PhaseResult>,
    pub progress: Vec<RelationProgress>,
    #[serde(skip)] declaration: Interaction,
    #[serde(skip)] object: String,
    #[serde(skip)] started: Option<u64>,
    #[serde(skip)] last_pts: Option<u64>,
    #[serde(skip)] touch_counts: Vec<Vec<u32>>,
    #[serde(skip)] hold_inside: Vec<u32>,
    #[serde(skip)] stand_since: Option<u64>,
    #[serde(skip)] pub(crate) couplings: Vec<CouplingCommand>,
}

pub enum Step { Running, PhaseDone, Done, Failed(String) }

impl DeclaredState {
    pub fn new(object: &super::OfficeObject, interaction: &str) -> Result<Self, String> {
        let declaration = object.interactions.iter().find(|i| i.id == interaction).cloned()
            .ok_or_else(|| format!("unsupported_capability: {} declares no interaction {interaction}", object.id))?;
        let mut state = Self { interaction: interaction.into(), index: 0, phase: String::new(), results: Vec::new(),
            progress: Vec::new(), declaration, object: object.id.clone(), started: None, last_pts: None,
            touch_counts: Vec::new(), hold_inside: Vec::new(), stand_since: None, couplings: Vec::new() };
        state.enter(0);
        Ok(state)
    }

    pub fn current(&self) -> &InteractionPhase { &self.declaration.phases[self.index] }
    pub fn prompt(&self) -> &str { &self.current().prompt }
    pub fn has_next(&self) -> bool { self.index + 1 < self.declaration.phases.len() }

    /// Start phase `index`. `support_on` applies here, before the body moves onto the support.
    pub fn enter(&mut self, index: usize) {
        self.index = index;
        let phase = self.declaration.phases[index].clone();
        self.phase = phase.id.clone();
        self.started = None;
        self.last_pts = None;
        self.stand_since = None;
        self.progress = phase.relations.iter().map(|r| RelationProgress {
            kind: match r { Relation::Touch { .. } => "touch", Relation::Hold { .. } => "hold",
                            Relation::Support { .. } => "support", Relation::Pair { .. } => "pair" },
            zone: r.zone().into(), ..Default::default() }).collect();
        self.touch_counts = phase.relations.iter().map(|r| match r {
            Relation::Touch { joints, .. } => vec![0; joints.len()], _ => Vec::new() }).collect();
        self.hold_inside = vec![0; phase.relations.len()];
        if let Some(Transition::SupportOn { .. }) = &phase.transition {
            self.couplings.push(CouplingCommand::SupportOn { object: self.object.clone() });
        }
    }

    fn finish(&mut self, status: &'static str, elapsed_s: f32, detail: Option<String>) {
        self.results.push(PhaseResult { phase: self.phase.clone(), status, elapsed_s,
            relations: self.progress.clone(), detail });
    }

    /// Evaluate one displayed frame of the current phase's revision.
    pub fn observe(&mut self, office: &OfficeSnapshot) -> Step {
        let observation = &office.avatar;
        let pts = observation.displayed_pts_ms;
        if self.last_pts.is_some_and(|old| pts <= old) { return Step::Running; }
        self.last_pts = Some(pts);
        let start = *self.started.get_or_insert(pts);
        let elapsed = pts.saturating_sub(start) as f32 / 1000.;
        let phase = self.current().clone();
        let Some(owner) = office.objects.iter().find(|o| o.id == self.object) else {
            return self.fail(elapsed, "interaction object disappeared".into());
        };
        let joint = |j| observed_joint(j, &observation.wrists, &observation.feet, observation.pelvis);
        let distance = |zone: &str, at: Vec3| resolve_zone(owner, zone, &office.objects, &office.homes)
            .map(|(z, t)| zone_distance(&z, &t, at));
        for (i, relation) in phase.relations.iter().enumerate() {
            let [t0, t1] = relation.window_s();
            if elapsed < t0 || self.progress[i].met { continue; }
            let window_closed = elapsed > t1;
            let progress = &mut self.progress[i];
            if !window_closed { progress.frames += 1; }
            match relation {
                Relation::Touch { joints, zone, min_frames, tolerance_m, .. } => {
                    if window_closed { continue; }
                    for (k, j) in joints.iter().enumerate() {
                        let Some(d) = distance(zone, joint(*j)) else { continue };
                        progress.best_distance_m = Some(progress.best_distance_m.map_or(d, |b| b.min(d)));
                        if d <= *tolerance_m { self.touch_counts[i][k] += 1; }
                    }
                    progress.inside = self.touch_counts[i].iter().copied().min().unwrap_or(0);
                    progress.met = progress.inside >= *min_frames;
                }
                Relation::Hold { joints, require, zone, tolerance_m, .. } => {
                    if !window_closed {
                        let inside: Vec<bool> = joints.iter().map(|j| distance(zone, joint(*j)).is_some_and(|d| {
                            progress.best_distance_m = Some(progress.best_distance_m.map_or(d, |b| b.min(d)));
                            d <= *tolerance_m })).collect();
                        let ok = match require { Require::All => inside.iter().all(|x| *x), Require::Any => inside.iter().any(|x| *x) };
                        if ok { self.hold_inside[i] += 1; }
                        progress.inside = self.hold_inside[i];
                    } else {
                        progress.met = progress.frames > 0 && progress.inside as f32 >= HOLD_SHARE * progress.frames as f32;
                    }
                }
                Relation::Support { body, zone, tolerance_m, .. } => {
                    if window_closed { continue; }
                    let joints: &[super::interaction::Joint] = match body {
                        super::interaction::Body::Feet => &[super::interaction::Joint::LeftFoot, super::interaction::Joint::RightFoot],
                        super::interaction::Body::Pelvis => &[super::interaction::Joint::Hips],
                    };
                    // Ankles sit above the sole: a foot is supported within the zone's thickness band.
                    let worst = joints.iter().filter_map(|j| distance(zone, joint(*j))).fold(0f32, f32::max);
                    progress.best_distance_m = Some(progress.best_distance_m.map_or(worst, |b| b.min(worst)));
                    progress.inside = if worst <= *tolerance_m { progress.inside + 1 } else { 0 };
                    progress.met = progress.inside >= SUPPORT_FRAMES;
                }
                Relation::Pair { joints, distance_m: [lo, hi], .. } => {
                    if !window_closed {
                        let d = joint(joints[0]).distance(joint(joints[1]));
                        progress.best_distance_m = Some(d);
                        if (*lo..=*hi).contains(&d) { progress.inside += 1; }
                    } else {
                        progress.met = progress.frames > 0 && progress.inside as f32 >= PAIR_SHARE * progress.frames as f32;
                    }
                }
            }
            if window_closed && !self.progress[i].met {
                let p = &self.progress[i];
                return self.fail(elapsed, format!("{} relation on {} not met ({} of {} frames)",
                    p.kind, if p.zone.is_empty() { "hands" } else { &p.zone }, p.inside, p.frames));
            }
        }
        let done = if phase.relations.is_empty() {
            match &phase.stand_at {
                Some(stand) => {
                    let near = resolve_zone(owner, &stand.zone, &office.objects, &office.homes).is_some_and(|(z, t)| {
                        let local = Vec3::from(z.center.unwrap_or_else(|| {
                            let (a, b) = (Vec3::from(z.a.unwrap_or_default()), Vec3::from(z.b.unwrap_or_default()));
                            ((a + b) / 2.).to_array()
                        }));
                        let point = t * local;
                        let close = Vec2::new(point.x - observation.pelvis[0], point.z - observation.pelvis[2]).length()
                            <= STAND_RADIUS_M;
                        close && stand_facing(stand.facing_deg, &t, point)
                            .is_none_or(|facing| angle_between(facing, observation.heading) <= STAND_FACING_DEG.to_radians())
                    });
                    if !near { self.stand_since = None; }
                    near && pts.saturating_sub(*self.stand_since.get_or_insert(pts)) >= NO_RELATION_SETTLE_MS / 2
                }
                None => pts.saturating_sub(start) >= NO_RELATION_SETTLE_MS,
            }
        } else {
            self.progress.iter().all(|p| p.met)
        };
        if !done {
            if elapsed > phase.timeout_s {
                let pending: Vec<String> = self.progress.iter().filter(|p| !p.met)
                    .map(|p| format!("{} {} ({} of {} frames, best {:.3} m)", p.kind, p.zone, p.inside, p.frames,
                                     p.best_distance_m.unwrap_or(f32::NAN))).collect();
                return self.fail(elapsed, format!("phase {} timed out after {:.1} s: {}", self.phase, phase.timeout_s,
                    if pending.is_empty() { "stand_at not reached".into() } else { pending.join("; ") }));
            }
            return Step::Running;
        }
        if let Some(command) = self.transition(&phase, owner, office) {
            self.couplings.push(command);
        }
        self.finish("passed", elapsed, None);
        if self.has_next() { Step::PhaseDone } else { Step::Done }
    }

    fn fail(&mut self, elapsed: f32, detail: String) -> Step {
        self.finish("failed", elapsed, Some(detail.clone()));
        Step::Failed(format!("interaction {}/{}: {detail}", self.interaction, self.phase))
    }

    /// Coupling applied when the phase's relations are met (`support_on` applies at phase start).
    fn transition(&self, phase: &InteractionPhase, owner: &super::OfficeObject, office: &OfficeSnapshot) -> Option<CouplingCommand> {
        let object = self.object.clone();
        match phase.transition.as_ref()? {
            Transition::Attach { anchors, slide_on } => {
                let transform = owner.transform();
                let anchors = anchors.iter().filter_map(|a| {
                    let (zone, _) = resolve_zone(owner, &a.zone, &office.objects, &office.homes)?;
                    let wrist = observed_joint(a.joint, &office.avatar.wrists, &office.avatar.feet, office.avatar.pelvis);
                    let local = transform.rotation.inverse() * (wrist - transform.translation);
                    Some((hand(a.joint)?, zone.nearest_local(local).to_array(), zone.grip))
                }).collect();
                Some(CouplingCommand::Attach { object, anchors, slide_on: slide_on.clone() })
            }
            Transition::Release { onto } => Some(CouplingCommand::Release { object, onto: onto.clone() }),
            Transition::SupportOff => Some(CouplingCommand::SupportOff { object }),
            Transition::SupportOn { .. } => None,
        }
    }
}

/// Heading (0 faces +Z) a stand point asks for: the declared object-local facing,
/// else towards the object's origin; `None` when standing on the origin itself.
pub fn stand_facing(facing_deg: Option<f32>, frame: &Transform, point: Vec3) -> Option<f32> {
    let ahead = match facing_deg {
        Some(f) => frame.rotation * Vec3::new(f.to_radians().sin(), 0., f.to_radians().cos()),
        None => frame.translation - point,
    };
    (ahead.x * ahead.x + ahead.z * ahead.z > 1e-6).then(|| ahead.x.atan2(ahead.z))
}

fn angle_between(a: f32, b: f32) -> f32 {
    let d = a - b;
    d.sin().atan2(d.cos()).abs()
}
