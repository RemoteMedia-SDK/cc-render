//! Timelapse: the shape changing over time, e.g. the character gaining weight.
//!
//! The sliders move from a start design to a target design; each slider weight is
//! interpolated linearly (`start + t * (target - start)`), and since a slider's
//! deltas are linear in its weight on each side of its base, every frame is
//! itself a valid slider mix (saveable as a recipe or GLB like any other).
//! While a timelapse plays it owns the sliders.

use super::*;

/// Built-in targets: sliders set on top of the start (by display name).
pub const PRESETS: [(&str, &[(&str, f32)]); 2] = [
    ("Heavy (female)", &[("Female Heavy Full-body", 1.0), ("Face Heavy", 0.5)]),
    ("Heavy (male)", &[("Male Heavy Full-body", 1.0), ("Face Heavy", 0.5)]),
];

pub struct Timelapse {
    pub start: Vec<f32>,
    pub target: Option<Vec<f32>>,
    pub target_label: String,
    /// 0 = start, 1 = target.
    pub t: f32,
    pub playing: bool,
    /// Seconds from start to target.
    pub seconds: f32,
    /// Play back and forth instead of stopping at the target.
    pub bounce: bool,
    dir: f32,
    /// `t` changed: the weights follow on the next update.
    moved: bool,
    pub path: String,
}

impl Timelapse {
    pub fn new(start: Vec<f32>) -> Self {
        Timelapse {
            start,
            target: None,
            target_label: String::new(),
            t: 0.0,
            playing: false,
            seconds: 8.0,
            bounce: false,
            dir: 1.0,
            moved: false,
            path: String::new(),
        }
    }

    pub fn set_target(&mut self, target: Vec<f32>, label: String) {
        self.target = Some(target);
        self.target_label = label;
        self.moved = true;
    }

    /// Show time `t`.
    pub fn seek(&mut self, t: f32) {
        self.t = t.clamp(0.0, 1.0);
        self.moved = true;
    }

    pub fn play(&mut self) {
        if self.t >= 1.0 && !self.bounce {
            self.t = 0.0;
        }
        self.dir = if self.t >= 1.0 { -1.0 } else { 1.0 };
        self.playing = true;
    }

    /// Weights at time `t`.
    pub fn weights_at(&self, t: f32) -> Option<Vec<f32>> {
        let target = self.target.as_ref()?;
        Some(self.start.iter().zip(target).map(|(a, b)| a + t * (b - a)).collect())
    }
}

/// `--timelapse` / a preset button: the target, by preset name or recipe file
/// (a recipe is applied as usual: on the captured character, not on the start).
pub fn resolve_target(pack: &ShapePack, start: &[f32], arg: &str) -> anyhow::Result<(Vec<f32>, String)> {
    if let Some((name, sliders)) = PRESETS.iter().find(|(n, _)| preset_key(n) == preset_key(arg)) {
        let mut w = start.to_vec();
        for (key, value) in *sliders {
            w[pack.find(key)?] = *value;
        }
        return Ok((w, name.to_string()));
    }
    let w = pack.weights_from_recipe(&serde_json::from_slice(&std::fs::read(arg)?)?)?;
    Ok((w, std::path::Path::new(arg).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()))
}

/// "Heavy (female)" matches `heavy-female` on the command line.
fn preset_key(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Advances a playing timelapse and puts its weights on the sliders.
pub fn timelapse(time: Res<Time>, mut editor: ResMut<Editor>) {
    let ed = &mut *editor;
    let lapse = &mut ed.lapse;
    if lapse.playing && lapse.target.is_some() {
        lapse.t += lapse.dir * time.delta_secs() / lapse.seconds.max(0.1);
        if lapse.t >= 1.0 {
            lapse.t = 1.0;
            lapse.dir = -1.0;
            lapse.playing = lapse.bounce;
        } else if lapse.t <= 0.0 {
            lapse.t = 0.0;
            lapse.dir = 1.0;
        }
        lapse.moved = true;
    }
    if std::mem::take(&mut lapse.moved) {
        if let Some(w) = lapse.weights_at(lapse.t) {
            ed.weights = w;
            ed.dirty = true;
        }
    }
}

pub fn timelapse_ui(ui: &mut egui::Ui, ed: &mut Editor) {
    egui::CollapsingHeader::new("Timelapse").id_salt("timelapse").default_open(ed.lapse.target.is_some()).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Start");
            if ui.button("Current sliders").on_hover_text("start from the sliders as they are now").clicked() {
                ed.lapse.playing = false;
                ed.lapse.start = ed.weights.clone();
                ed.lapse.seek(0.0);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Target");
            let mut pick = None;
            for (name, _) in PRESETS {
                if ui.button(name).clicked() {
                    pick = Some(name.to_string());
                }
            }
            if ui.button("Current sliders").on_hover_text("end at the sliders as they are now").clicked() {
                ed.lapse.playing = false;
                let w = ed.weights.clone();
                ed.lapse.set_target(w, "sliders".into());
                ed.lapse.t = 1.0;
            }
            if let Some(name) = pick {
                set_target(ed, &name);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Recipe");
            ui.text_edit_singleline(&mut ed.lapse.path);
            if ui.button("Load").clicked() {
                let path = ed.lapse.path.clone();
                set_target(ed, &path);
            }
        });
        let lapse = &mut ed.lapse;
        ui.add_enabled_ui(lapse.target.is_some(), |ui| {
            ui.label(if lapse.target.is_some() { format!("→ {}", lapse.target_label) } else { "no target".into() });
            let mut t = lapse.t;
            if ui.add(egui::Slider::new(&mut t, 0.0..=1.0).text("time")).changed() {
                lapse.playing = false;
                lapse.seek(t);
            }
            ui.horizontal(|ui| {
                if lapse.playing {
                    if ui.button("Pause").clicked() {
                        lapse.playing = false;
                    }
                } else if ui.button("Play").clicked() {
                    lapse.play();
                }
                ui.add(egui::DragValue::new(&mut lapse.seconds).speed(0.1).range(0.5..=120.0).suffix(" s"));
                ui.checkbox(&mut lapse.bounce, "Back and forth");
            });
        });
    });
}

fn set_target(ed: &mut Editor, arg: &str) {
    ed.status = match resolve_target(&ed.pack, &ed.lapse.start, arg) {
        Ok((w, label)) => {
            ed.lapse.set_target(w, label.clone());
            format!("timelapse target: {label}")
        }
        Err(e) => format!("timelapse target failed: {e:#}"),
    };
}
