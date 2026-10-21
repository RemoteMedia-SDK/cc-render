//! `cc5-hair`: build hair library parts for cc-render's avatar editor.
//!
//!   cc5-hair bake <spec.json> --head <head.glb> --lib <dir> [--settings <json>]
//!   cc5-hair smart-hair --src <smart_hair dir> --head <head.glb> --lib <dir>
//!                       --config <smart_hair.json> [--only <group>[,<group>...]]
//!
//! `bake`: one part from a spec (e.g. specs/cc5_converted.json, the character's
//! own hair). `smart-hair`: every CC5 Smart Hair group export (`<group>/character.glb`)
//! with the per-kind / per-group settings of smart_hair.json; catalog.json and
//! styles.json ("CC5 <group>") are updated.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use cc5_hair::head::Body;
use cc5_hair::settings::Spec;
use cc5_hair::source::Source;
use cc5_hair::{glb_out, part};
use serde_json::{json, Value};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn need(args: &[String], name: &str) -> Result<PathBuf> {
    arg(args, name).map(PathBuf::from).ok_or_else(|| anyhow!("missing {name}"))
}

fn read_json(p: &Path) -> Result<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?).with_context(|| format!("parsing {}", p.display()))
}

/// Merge `over` into `base` (objects: key by key, one level).
fn merge(base: &mut Value, over: &Value) {
    if let (Some(b), Some(o)) = (base.as_object_mut(), over.as_object()) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
}

struct Library {
    dir: PathBuf,
    catalog: Value,
    styles: Value,
}

impl Library {
    fn open(dir: &Path) -> Result<Self> {
        let cat = dir.join("catalog.json");
        let sty = dir.join("styles.json");
        Ok(Self {
            dir: dir.to_path_buf(),
            catalog: if cat.exists() { read_json(&cat)? } else { json!({"parts": []}) },
            styles: if sty.exists() { read_json(&sty)? } else { json!({"styles": []}) },
        })
    }

    fn put_part(&mut self, entry: Value) {
        let parts = self.catalog["parts"].as_array_mut().unwrap();
        parts.retain(|p| p["id"] != entry["id"]);
        parts.push(entry);
    }

    fn put_style(&mut self, name: &str, parts: Vec<String>) {
        let styles = self.styles["styles"].as_array_mut().unwrap();
        styles.retain(|s| s["name"] != name);
        styles.push(json!({"name": name, "parts": parts}));
    }

    fn save(&self) -> Result<()> {
        std::fs::write(self.dir.join("catalog.json"), serde_json::to_string_pretty(&self.catalog)?)?;
        std::fs::write(self.dir.join("styles.json"), serde_json::to_string_pretty(&self.styles)?)?;
        Ok(())
    }
}

fn bake_one(spec: &Spec, head: &Source, body: &Body, lib: &mut Library) -> Result<()> {
    let t = Instant::now();
    let built = part::build(spec, head, body, &lib.dir)?;
    let t_build = t.elapsed().as_secs_f32();
    glb_out::write(&built, head, &lib.dir.join(format!("{}.glb", spec.id)))?;
    glb_out::copy_files(&built, &lib.dir)?;
    let size = std::fs::metadata(lib.dir.join(format!("{}.glb", spec.id)))?.len();
    println!(
        "{}: {} strands/cards, {} verts, {:.0} MB  (build {:.1} s, total {:.1} s){}",
        spec.id,
        built.entry["cards"],
        built.entry["verts"],
        size as f64 / 1048576.0,
        t_build,
        t.elapsed().as_secs_f32(),
        built.entry.get("follow").map(|f| format!("  follow {f}")).unwrap_or_default()
    );
    lib.put_part(built.entry);
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().cloned().unwrap_or_default();
    let head_path = need(&args, "--head")?;
    let lib_dir = need(&args, "--lib")?;
    std::fs::create_dir_all(&lib_dir)?;
    let t = Instant::now();
    let head = Source::load(&head_path)?;
    let body = Body::from_source(&head)?;
    println!("head {} loaded in {:.1} s", head_path.display(), t.elapsed().as_secs_f32());
    let mut lib = Library::open(&lib_dir)?;
    match cmd.as_str() {
        "bake" => {
            let spec_path = PathBuf::from(args.get(1).ok_or_else(|| anyhow!("bake <spec.json>"))?);
            let mut v = read_json(&spec_path)?;
            if let Some(s) = arg(&args, "--settings") {
                let over = read_json(Path::new(&s))?;
                merge(&mut v["convert"], &over);
            }
            let spec: Spec = serde_json::from_value(v)?;
            bake_one(&spec, &head, &body, &mut lib)?;
        }
        "smart-hair" => {
            let src = need(&args, "--src")?;
            let cfg = read_json(&need(&args, "--config")?)?;
            let only: Option<Vec<String>> = arg(&args, "--only").map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
            let beards: Vec<String> = serde_json::from_value(cfg["beards"].clone())?;
            let mut groups: Vec<_> = std::fs::read_dir(&src)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("character.glb").exists()).collect();
            groups.sort();
            for gdir in groups {
                let g = gdir.file_name().unwrap().to_string_lossy().to_string();
                if only.as_ref().is_some_and(|o| !o.contains(&g)) {
                    continue;
                }
                let kind = if beards.contains(&g) { "beard" } else if g.starts_with("Male_") || g.starts_with("Female_") { "brows" } else { "hair" };
                let k = &cfg["kinds"][kind];
                let mut conv = k["convert"].clone();
                conv["source_glb"] = json!(gdir.join("character.glb"));
                if gdir.join("shape_base.obj").exists() {
                    conv["shape_dir"] = json!(gdir); // which layers CC5 moves with the head box / a bone
                }
                if let Some(o) = cfg["groups"].get(&g) {
                    merge(&mut conv, o);
                }
                let mut slug = String::new();
                for ch in g.to_lowercase().chars() {
                    if ch.is_ascii_alphanumeric() {
                        slug.push(ch);
                    } else if !slug.ends_with('_') {
                        slug.push('_');
                    }
                }
                let pid = format!("sh_{}", slug.trim_matches('_'));
                let spec = Spec {
                    id: pid.clone(),
                    slot: k["slot"].as_str().unwrap_or("Base").into(),
                    kind: k["kind"].as_str().unwrap_or("scalp").into(),
                    atlas: "strands".into(),
                    seed: cfg["seed"].as_u64().unwrap_or(101),
                    vars: Default::default(),
                    convert: conv,
                };
                bake_one(&spec, &head, &body, &mut lib).with_context(|| g.clone())?;
                // beards / brows on the converted own hair, as in CC5
                let parts = if kind == "hair" { vec![pid] } else { vec!["cc5_converted".to_string(), pid] };
                lib.put_style(&format!("CC5 {g}"), parts);
            }
        }
        _ => bail!("usage: cc5-hair bake <spec.json> | smart-hair --src <dir> --config <json>   (--head <glb> --lib <dir>)"),
    }
    lib.save()?;
    println!("done in {:.1} s", t.elapsed().as_secs_f32());
    Ok(())
}
