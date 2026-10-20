//! Apply CC5 shaping sliders to a character GLB without CC5.
//! Usage: shape_avatar character.glb pack_dir weights.json shaped.glb
//!
//! `weights.json` maps slider names (or ids) to values, e.g.
//! `{"Character Height": 0.5, "Jaw Width": -0.3}`; unlisted sliders keep the
//! character's captured design value.
use cc_render_plugin::cc_render::{shape::ShapePack, shape_glb::shape_glb};
use std::collections::HashMap;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 5, "character.glb pack_dir weights.json shaped.glb required");
    let pack = ShapePack::load(Path::new(&args[2]))?;
    let wanted: HashMap<String, f32> = serde_json::from_slice(&std::fs::read(&args[3])?)?;
    let weights = pack.weights_from(wanted.iter().map(|(k, &v)| (k.as_str(), v)))?;
    let delta = pack.evaluate(&weights);
    let out = shape_glb(&std::fs::read(&args[1])?, &pack, &delta)?;
    std::fs::write(&args[4], out)?;
    println!("wrote {} ({} slider(s) set)", args[4], wanted.len());
    Ok(())
}
