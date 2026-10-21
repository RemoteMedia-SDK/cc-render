//! `cc5-fbx <in.fbx> <out.glb> [--dump <dump.json>] [--cc5-json <sidecar.json>]`
//!
//! A CC5 FBX export as a GLB (`fbx_to_glb.py` without Blender). `--dump` also
//! writes the joints / weights / rest-position dump the pack builders read.
//! `--cc5-json` adds CC5's material tints and `extras.cc5` (maps written to
//! `<out>.cc5maps/`), as bake_alpha.py's first pass + glb_cc5_materials.py.

use std::path::PathBuf;

use anyhow::{bail, Result};
use cc5_fbx::{Character, GlbOptions};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut pos, mut dump, mut cc5) = (Vec::new(), None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dump" => dump = args.next().map(PathBuf::from),
            "--cc5-json" => cc5 = args.next().map(PathBuf::from),
            "--keep-morphs" => bail!("--keep-morphs: morph targets are not written (nothing uses them)"),
            _ if a.starts_with("--") => bail!("unknown option {a}"),
            _ => pos.push(PathBuf::from(a)),
        }
    }
    let [fbx, out] = <[PathBuf; 2]>::try_from(pos).map_err(|_| anyhow::anyhow!("usage: cc5-fbx <in.fbx> <out.glb> [--dump <dump.json>] [--cc5-json <json>]"))?;
    let t = std::time::Instant::now();
    let c = Character::load(&fbx)?;
    let t_load = t.elapsed().as_secs_f32();
    let maps_dir = cc5.as_ref().map(|_| out.with_extension("cc5maps"));
    let bytes = c.to_glb(&GlbOptions { cc5_json: cc5, maps_dir })?;
    std::fs::write(&out, &bytes)?;
    if let Some(d) = &dump {
        cc5_fbx::dump::write(&c, d)?;
    }
    let verts: usize = c.meshes.iter().map(|m| m.points.len()).sum();
    println!(
        "wrote {}: {} meshes, {} joints, {} materials, {} control points, {:.1} MB (load {:.2} s, total {:.2} s)",
        out.display(),
        c.meshes.len(),
        c.joints.len(),
        c.materials.len(),
        verts,
        bytes.len() as f64 / 1e6,
        t_load,
        t.elapsed().as_secs_f32()
    );
    Ok(())
}
