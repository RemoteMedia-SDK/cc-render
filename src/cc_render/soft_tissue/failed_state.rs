//! Diagnostic capture of a failed nonlinear tissue solve. Opt-in through
//! `CC_TISSUE_FAILED_STATE_DIR`; recording never changes solver behaviour.
//! Values are stored as IEEE754 f64 bits so an offline probe replays the
//! identical step instead of a decimal approximation of it.
use super::*;
use serde_json::{json, Value};

fn bits(v: DVec3) -> [u64; 3] {
    v.to_array().map(f64::to_bits)
}

fn frame_bits(frame: Frame) -> Value {
    json!({"translation": bits(frame.translation), "rotation": frame.rotation.to_array().map(f64::to_bits)})
}

pub(super) fn record(
    tissue: &Tissue,
    interval: f64,
    frame: Frame,
    gravity: DVec3,
    contacts: &[PlaneContact],
    iterate: &[DVec3],
    error: &TissueError,
) {
    let Ok(directory) = std::env::var("CC_TISSUE_FAILED_STATE_DIR") else {
        return;
    };
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let index = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let state = json!({
        "float_encoding": "IEEE754 f64 bits",
        "error": error.to_string(),
        "node_count": tissue.rest.len(),
        "rest_0": bits(tissue.rest[0]),
        "interval_s": interval.to_bits(),
        "end_frame": frame_bits(frame),
        "gravity": bits(gravity),
        "positions": tissue.positions.iter().map(|p| bits(*p)).collect::<Vec<_>>(),
        "velocities": tissue.velocities.iter().map(|v| bits(*v)).collect::<Vec<_>>(),
        "frame": frame_bits(tissue.frame),
        "previous": tissue.previous.as_ref().map(|old| json!({
            "positions": old.positions.iter().map(|p| bits(*p)).collect::<Vec<_>>(),
            "velocities": old.velocities.iter().map(|v| bits(*v)).collect::<Vec<_>>(),
            "dt": old.dt.to_bits(),
        })),
        "contacts": contacts.iter().map(|c| json!({
            "tetrahedron": c.point.tetrahedron,
            "barycentric": c.point.barycentric.map(f64::to_bits),
            "influence": c.point.influence.to_bits(),
            "offset": bits(c.point.offset),
            "surface_point": bits(c.surface_point),
            "normal": bits(c.normal),
            "stiffness_n_per_m": c.stiffness_n_per_m.to_bits(),
            "max_force_n": c.max_force_n.to_bits(),
        })).collect::<Vec<_>>(),
        "iterate": iterate.iter().map(|p| bits(*p)).collect::<Vec<_>>(),
    });
    let path = std::path::Path::new(&directory).join(format!("failed-state-{index:03}.json"));
    match std::fs::create_dir_all(&directory).and_then(|_| std::fs::write(&path, state.to_string())) {
        Ok(()) => tracing::info!(path = %path.display(), "recorded failed tissue solve state"),
        Err(error) => tracing::warn!(%error, "could not record failed tissue solve state"),
    }
}
