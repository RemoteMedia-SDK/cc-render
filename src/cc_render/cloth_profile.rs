//! Wall-clock attribution of the cloth solve by phase, for release benches.
//! Compiled only into test builds; production code paths are unchanged.
use std::{cell::RefCell, collections::BTreeMap, time::Duration, time::Instant};

#[derive(Default, Clone, Debug)]
pub(crate) struct Phase {
    pub seconds: f64,
    pub calls: u64,
    pub items: u64,
}

thread_local! {
    static PROFILE: RefCell<Option<BTreeMap<&'static str, Phase>>> = const { RefCell::new(None) };
}

pub(crate) fn enable() {
    PROFILE.with(|p| *p.borrow_mut() = Some(BTreeMap::new()));
}

pub(crate) fn take() -> BTreeMap<&'static str, Phase> {
    PROFILE.with(|p| p.borrow_mut().replace(BTreeMap::new()).unwrap_or_default())
}

pub(crate) fn add(name: &'static str, elapsed: Duration, items: u64) {
    PROFILE.with(|p| {
        if let Some(map) = p.borrow_mut().as_mut() {
            let phase = map.entry(name).or_default();
            phase.seconds += elapsed.as_secs_f64();
            phase.calls += 1;
            phase.items += items;
        }
    });
}

pub(crate) fn count(name: &'static str, items: u64) {
    add(name, Duration::ZERO, items);
}

pub(crate) struct Timer(&'static str, Instant);

impl Timer {
    pub(crate) fn start(name: &'static str) -> Self {
        Self(name, Instant::now())
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        add(self.0, self.1.elapsed(), 0);
    }
}

/// Per-substep position snapshots at solver stage boundaries, for locating
/// where velocity is created. Recorded only while explicitly enabled.
pub(crate) type StageTrace = Vec<(&'static str, usize, Vec<bevy::math::DVec3>)>;

thread_local! {
    static STAGES: RefCell<Option<StageTrace>> = const { RefCell::new(None) };
}

pub(crate) fn trace_stages(enabled: bool) {
    STAGES.with(|s| *s.borrow_mut() = enabled.then(Vec::new));
}

pub(crate) fn take_stages() -> StageTrace {
    STAGES.with(|s| {
        s.borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    })
}

pub(crate) fn stage(name: &'static str, substep: usize, positions: &[bevy::math::DVec3]) {
    STAGES.with(|s| {
        if let Some(trace) = s.borrow_mut().as_mut() {
            trace.push((name, substep, positions.to_vec()));
        }
    });
}
