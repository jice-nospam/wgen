use eframe::egui;

use crate::panel_export::TEXTEDIT_WIDTH;
pub struct PanelSaveLoad {
    pub save_dir: String,
    pub file_name: String,
}

pub enum SaveLoadAction {
    Save,
    Load,
}

impl Default for PanelSaveLoad {
    fn default() -> Self {
        Self {
            save_dir: std::env::current_dir().unwrap().display().to_string(),
            file_name: "my_terrain.wgen".to_owned(),
        }
    }
}

impl PanelSaveLoad {
    pub fn get_file_path(&self) -> String {
        format!("{}/{}", self.save_dir, self.file_name)
    }
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<SaveLoadAction> {
        ui.heading("Save/load project");
        ui.label("Save/Load directory");
        ui.add(
            egui::TextEdit::singleline(&mut self.save_dir)
                .hint_text("Directory")
                .desired_width(TEXTEDIT_WIDTH),
        );
        ui.label("File name");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.file_name)
                    .hint_text("File name")
                    .desired_width(TEXTEDIT_WIDTH),
            );
        });
        if ui.button("Load!").clicked() {
            return Some(SaveLoadAction::Load);
        }
        if ui.button("Save!").clicked() {
            return Some(SaveLoadAction::Save);
        }
        None
    }
}
