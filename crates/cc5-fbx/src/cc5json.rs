//! CC5's export sidecar JSON (next to the FBX): material tints and opacity
//! (bake_alpha.py's authoritative first pass) and the CC5 shader settings and
//! maps as `extras.cc5` (glb_cc5_materials.py):
//!
//! ```json
//! {"shader": "RLHair", "maps": {"Hair Flow Map": "character.cc5maps/..._flow.png", ...},
//!  "srgb": ["..."], "vars": {...}, "sss": {...}, "wrinkle": {"maps": {...}, "rules": [...], ...}}
//! ```

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

pub struct Cc5Json {
    pub path: PathBuf,
    /// Every material under Meshes (bake_alpha.py's `load_cc5_json`).
    specs: HashMap<String, Value>,
    /// Materials with a CC5 custom shader, anywhere in the file.
    shaders: HashMap<String, Value>,
}

/// Writes the maps `extras.cc5` names into one folder (TGA re-encoded as PNG).
pub struct Maps {
    base: PathBuf,
    dir: PathBuf,
    done: HashMap<PathBuf, String>,
    pub missing: Vec<PathBuf>,
}

impl Maps {
    /// Maps named relative to `json`'s folder, written into `dir`.
    pub fn new(json: &Path, dir: PathBuf) -> Self {
        Self { base: json.parent().unwrap_or(Path::new(".")).to_path_buf(), dir, done: HashMap::new(), missing: Vec::new() }
    }

    /// The map's path relative to `dir`'s parent, or None when the file is missing.
    fn get(&mut self, rel: &str) -> Result<Option<String>> {
        let path = normalize(&self.base.join(rel.replace('\\', "/")));
        if let Some(r) = self.done.get(&path) {
            return Ok(Some(r.clone()));
        }
        if !path.is_file() {
            self.missing.push(path);
            return Ok(None);
        }
        let (bytes, mime) = crate::glb::file_bytes(&path)?;
        let stem = path.file_stem().unwrap().to_string_lossy();
        let fname = format!("{stem}.{}", if mime == "image/png" { "png" } else { "jpg" });
        std::fs::create_dir_all(&self.dir)?;
        let out = self.dir.join(&fname);
        // shared between rebuilds: rewrite only when it differs
        if std::fs::metadata(&out).map(|m| m.len() as usize != bytes.len()).unwrap_or(true) {
            std::fs::write(&out, &bytes).with_context(|| format!("writing {}", out.display()))?;
        }
        let r = format!("{}/{fname}", self.dir.file_name().unwrap().to_string_lossy());
        self.done.insert(path, r.clone());
        Ok(Some(r))
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// Colour maps (sRGB); the rest are data.
fn is_srgb(slot: &str) -> bool {
    let l = slot.to_lowercase();
    l.contains("basecolor") || l.contains("diffuse") || l == "sclera" || l.contains("color")
}

fn clean(vars: Option<&Value>) -> Value {
    let mut m = Map::new();
    if let Some(o) = vars.and_then(Value::as_object) {
        for (k, v) in o {
            if !k.to_lowercase().starts_with("padding") {
                m.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(m)
}

impl Cc5Json {
    pub fn load(path: &Path) -> Result<Self> {
        let data: Value = serde_json::from_slice(&std::fs::read(path).with_context(|| format!("reading {}", path.display()))?)
            .with_context(|| format!("parsing {}", path.display()))?;
        let mut specs = HashMap::new();
        if let Some((rig, r)) = data.as_object().and_then(|o| o.iter().next()) {
            let obj = r.get("Object").and_then(Value::as_object);
            let inner = obj.and_then(|o| o.get(rig).or_else(|| o.values().next()));
            if let Some(meshes) = inner.and_then(|i| i.get("Meshes")).and_then(Value::as_object) {
                for m in meshes.values() {
                    if let Some(mats) = m.get("Materials").and_then(Value::as_object) {
                        for (name, mat) in mats {
                            if mat.is_object() {
                                specs.insert(name.clone(), mat.clone());
                            }
                        }
                    }
                }
            }
        }
        let mut shaders = HashMap::new();
        fn walk(v: &Value, out: &mut HashMap<String, Value>) {
            match v {
                Value::Object(o) => {
                    for (k, x) in o {
                        if x.get("Custom Shader").is_some() {
                            out.insert(k.clone(), x.clone());
                        } else {
                            walk(x, out);
                        }
                    }
                }
                Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        walk(&data, &mut shaders);
        Ok(Self { path: path.to_path_buf(), specs, shaders })
    }

    /// CC5's diffuse tint and opacity; transparent families (`*_Transparency`,
    /// Std_Eyelash) blend at full alpha (their maps carry it).
    pub fn apply(&self, name: &str, factor: &mut [f32; 4], blend: &mut bool) {
        if let Some(spec) = self.specs.get(name) {
            if let Some(dc) = spec.get("Diffuse Color").and_then(Value::as_array).filter(|a| a.len() >= 3) {
                for k in 0..3 {
                    factor[k] = (dc[k].as_f64().unwrap_or(255.0) as f32 / 255.0).clamp(0.0, 1.0);
                }
            }
            if let Some(o) = spec.get("Opacity").and_then(Value::as_f64) {
                factor[3] = (o as f32).clamp(0.0, 1.0);
            }
            if factor[3] < 0.999 {
                *blend = true;
            }
        }
        if name.ends_with("_Transparency") || name == "Std_Eyelash" {
            *blend = true;
            factor[3] = 1.0;
        }
    }

    /// `extras.cc5` for a material with a CC5 custom shader.
    pub fn extras(&self, name: &str, maps: &mut Maps) -> Result<Option<Value>> {
        let Some(cm) = self.shaders.get(name) else { return Ok(None) };
        let cs = &cm["Custom Shader"];
        let mut info = json!({"shader": cs.get("Shader Name").and_then(Value::as_str).unwrap_or(""), "maps": {}, "srgb": [],
                              "vars": clean(cs.get("Variable"))});
        if let Some(imgs) = cs.get("Image").and_then(Value::as_object) {
            for (slot, t) in imgs {
                let Some(p) = t.get("Texture Path").and_then(Value::as_str).filter(|p| !p.is_empty()) else { continue };
                if let Some(r) = maps.get(p)? {
                    info["maps"][slot] = json!(r);
                    if is_srgb(slot) {
                        info["srgb"].as_array_mut().unwrap().push(json!(slot));
                    }
                }
            }
        }
        if let Some(s) = cm.get("Subsurface Scatter").filter(|s| !s.is_null() && s.as_object().map_or(true, |o| !o.is_empty())) {
            info["sss"] = s.clone();
        }
        if let Some(w) = cm.get("Wrinkle") {
            if let Some(tex) = w.get("Textures").and_then(Value::as_object).filter(|t| !t.is_empty()) {
                let mut wm = Map::new();
                for (slot, t) in tex {
                    let Some(p) = t.get("Texture Path").and_then(Value::as_str).filter(|p| !p.is_empty()) else { continue };
                    if let Some(r) = maps.get(p)? {
                        wm.insert(slot.clone(), json!(r));
                    }
                }
                info["wrinkle"] = json!({"maps": wm, "rules": w.get("WrinkleRules").cloned().unwrap_or(json!([])),
                    "rule_weights": w.get("WrinkleRuleWeights").cloned().unwrap_or(json!([])),
                    "ease_strength": w.get("WrinkleEaseStrength").cloned().unwrap_or(Value::Null),
                    "overall_weight": w.get("WrinkleOverallWeight").cloned().unwrap_or(Value::Null)});
            }
        }
        Ok(Some(info))
    }
}
