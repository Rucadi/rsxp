//! Per-game settings editor, independent of a running session.
use rsxp_provision::settings::{Preferences, Settings};
use std::path::PathBuf;
pub struct Editor {
    pub key: String,
    pub title: String,
    pub root: PathBuf,
    pub prefs: Preferences,
    pub settings: Settings,
    pub patches: String,
    pub error: Option<String>,
}
pub enum Action { Save, Reset, Cover, ClearCover }
impl Editor {
    pub fn draw(&mut self, ctx: &egui::Context, open: &mut bool) -> Option<Action> {
        let mut action = None;
        egui::Window::new(format!("{} · Settings", self.title)).id(egui::Id::new("game-settings"))
            .open(open).default_width(480.0).resizable(true).show(ctx, |ui| {
                ui.label("Overrides apply only to this game, the next time it starts.");
                if let Some(error) = &self.error { ui.colored_label(egui::Color32::LIGHT_RED, error); }
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Choose cover image…").clicked() { action = Some(Action::Cover); }
                        if ui.button("Use repository cover").clicked() { action = Some(Action::ClearCover); }
                    });
                    ui.separator();
                    choice(ui, "Renderer", &mut self.settings.renderer, &["auto", "wgpu", "software"]);
                    choice(ui, "Ruby JIT (RPG Maker)", &mut self.settings.jit, &["yjit", "zjit", "off"]);
                    ui.horizontal(|ui| {
                        ui.label("RGSS version");
                        egui::ComboBox::from_id_salt("rgss-version").selected_text(self.settings.rgss.map(|n| n.to_string()).unwrap_or("Automatic".into())).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.rgss, None, "Automatic");
                            for n in 1..=3 { ui.selectable_value(&mut self.settings.rgss, Some(n), n.to_string()); }
                        });
                    });
                    ui.horizontal(|ui| {
                        let mut enabled = self.settings.fps.is_some();
                        if ui.checkbox(&mut enabled, "Override RPG Maker FPS").changed() { self.settings.fps = enabled.then_some(60); }
                        if let Some(fps) = &mut self.settings.fps { ui.add(egui::DragValue::new(fps).range(1..=240)); }
                    });
                    ui.checkbox(&mut self.settings.fast_forward, "Start with fast-forward enabled");
                    egui::CollapsingHeader::new("Keyboard controls").show(ui, |ui| {
                        ui.weak("Map game keys to physical keys. Function-key shortcuts stay unchanged.");
                        let keys: Vec<(i32, String)> = (4..=29).map(|n| (n, ((b'A' + (n - 4) as u8) as char).to_string()))
                            .chain([(40,"Enter"), (42,"Backspace"), (43,"Tab"), (44,"Space"), (79,"Right"), (80,"Left"), (81,"Down"), (82,"Up")].into_iter().map(|(n,s)| (n,s.into()))).collect();
                        for (target,label) in [(82,"Up"),(81,"Down"),(80,"Left"),(79,"Right"),(27,"X / cartridge A"),(29,"Z / cartridge B"),(22,"S / cartridge X"),(4,"A / cartridge Y"),(40,"Enter / Start"),(42,"Backspace / Select"),(20,"Q / L"),(26,"W / R"),(44,"Space")] {
                            let mut key = *self.settings.keyboard.get(&target).unwrap_or(&target);
                            ui.horizontal(|ui| {
                                ui.label(label);
                                egui::ComboBox::from_id_salt(("key",target)).selected_text(keys.iter().find(|k| k.0==key).map(|k| k.1.clone()).unwrap_or(key.to_string())).show_ui(ui, |ui| {
                                    for (n,name) in &keys { ui.selectable_value(&mut key,*n,name); }
                                });
                            });
                            if key == target { self.settings.keyboard.remove(&target); } else { self.settings.keyboard.insert(target,key); }
                        }
                    });
                    egui::CollapsingHeader::new("Gamepad controls").show(ui, |ui| {
                        let mut enabled = self.settings.gamepad.is_some();
                        if ui.checkbox(&mut enabled, "Use a game-specific mapping").changed() { self.settings.gamepad = enabled.then(|| "0.25;7,6,8,9,11,12,5,10,6,7,0,0,4,1,2,3,2,3,4,1".into()); }
                        if let Some(encoded) = &mut self.settings.gamepad {
                            let (zone, list) = encoded.split_once(';').unwrap();
                            let mut zone: f32 = zone.parse().unwrap();
                            let mut buttons: Vec<usize> = list.split(',').map(|s| s.parse().unwrap()).collect();
                            ui.add(egui::Slider::new(&mut zone,0.1..=0.9).text("Stick deadzone"));
                            let names = ["South","East","West","North","L shoulder","R shoulder","L trigger","R trigger","Select","Start","L stick click","R stick click","D-pad up","D-pad down","D-pad left","D-pad right","Stick left","Stick right","Stick up","Stick down"];
                            let actions = ["Unbound","Down","Left","Right","Up","A / Run","B / Cancel","C / Confirm","X","Y","Z","L","R"];
                            for (i, value) in buttons.iter_mut().enumerate() {
                                ui.horizontal(|ui| { ui.label(names[i]); egui::ComboBox::from_id_salt(("pad",i)).selected_text(actions[*value]).show_ui(ui, |ui| {
                                    for (n,name) in actions.iter().enumerate() { ui.selectable_value(value,n,*name); }
                                }); });
                            }
                            *encoded = format!("{};{}",zone,buttons.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(","));
                        }
                    });
                    egui::CollapsingHeader::new("Runtime patches (RPG Maker)").show(ui, |ui| {
                        ui.label("Enabled patch IDs, separated by commas");
                        ui.text_edit_singleline(&mut self.patches);
                        ui.weak("Use IDs from this game's patches folder / runtime patch panel. No patch code is downloaded here.");
                        for id in self.patches.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                            ui.horizontal(|ui| {
                                let mut enabled = self.settings.patch_values.contains_key(id);
                                if ui.checkbox(&mut enabled, format!("Parameter: {id}")).changed() {
                                    if enabled { self.settings.patch_values.insert(id.into(),1.0); } else { self.settings.patch_values.remove(id); }
                                }
                                if let Some(value) = self.settings.patch_values.get_mut(id) { ui.add(egui::DragValue::new(value)); }
                            });
                        }
                    });
                    egui::CollapsingHeader::new("Emulator options").show(ui, |ui| {
                        choice(ui,"Nintendo DS renderer",&mut self.settings.nds_renderer,&["wgpu","software"]);
                        for (label, path) in [("GBA BIOS",&mut self.settings.gba_bios),("DS BIOS 7",&mut self.settings.nds_bios7),("DS BIOS 9",&mut self.settings.nds_bios9),("DS firmware",&mut self.settings.nds_firmware)] {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                if ui.button("Choose…").clicked() { if let Some(file) = rfd::FileDialog::new().pick_file() { *path = Some(file.to_string_lossy().into_owned()); } }
                                if ui.small_button("Clear").clicked() { *path = None; }
                            });
                            if let Some(path) = path { ui.small(path.as_str()); }
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Save settings").clicked() { self.settings.patches = self.patches.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(); action = Some(Action::Save); }
                    if ui.button("Reset to repository defaults").clicked() { action = Some(Action::Reset); }
                });
            });
        action
    }
}
fn choice(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, values: &[&str]) {
    ui.horizontal(|ui| { ui.label(label); egui::ComboBox::from_id_salt(label).selected_text(value.as_deref().unwrap_or("Default")).show_ui(ui, |ui| {
        ui.selectable_value(value, None, "Default"); for option in values { ui.selectable_value(value, Some((*option).into()), *option); }
    }); });
}
