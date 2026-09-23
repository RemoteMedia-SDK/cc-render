//! Query a loaded home scene without starting a motion worker.
//! Usage: generate_waypoint avatar.glb arkit-map.json avatar bed path.json
//! Endpoints may also be JSON XYZ arrays, e.g. '[0,0,0]'.
use cc_render_plugin::cc_render::{
    office::navigation::NavigationTarget, renderer::PhysicsConfig, CcRenderer, RendererConfig,
};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 6,
        "avatar.glb arkit-map.json from to output.json required"
    );
    let endpoint = |value: &str| {
        serde_json::from_str::<NavigationTarget>(value)
            .unwrap_or_else(|_| NavigationTarget::Name(value.into()))
    };
    std::env::set_var("CC_OFFICE", "1");
    let renderer = CcRenderer::spawn(RendererConfig {
        glb_path: args[1].clone().into(),
        arkit_map_path: args[2].clone().into(),
        width: 320,
        height: 240,
        fps: 15,
        physics: Some(PhysicsConfig::default()),
        ..Default::default()
    })?;
    let started = Instant::now();
    let result = loop {
        let state = renderer.scene_state();
        if renderer.is_ready()
            && state
                .office
                .as_ref()
                .is_some_and(|s| s.avatar.position_valid)
        {
            break state
                .generate_waypoint(endpoint(&args[3]), endpoint(&args[4]))
                .map_err(anyhow::Error::from);
        }
        if started.elapsed() > Duration::from_secs(60) {
            break Err(anyhow::anyhow!("timed out waiting for scene observation"));
        }
        renderer.drain_frames(&mut Vec::new());
        std::thread::sleep(Duration::from_millis(50));
    };
    renderer.shutdown()?;
    let path = result?;
    std::fs::write(&args[5], serde_json::to_vec_pretty(&path)?)?;
    println!(
        "Saved {} waypoints ({:.2} m) to {}",
        path.waypoints.len(),
        path.distance_m,
        args[5]
    );
    Ok(())
}
