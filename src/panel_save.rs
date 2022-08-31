use std::path::PathBuf;

use eframe::egui;

use crate::panel_export::TEXTEDIT_WIDTH;
pub struct PanelSaveLoad {
    pub file_path: String,
    cur_dir: PathBuf,
}

pub enum SaveLoadAction {
    Save,
    Load,
}

impl Default for PanelSaveLoad {
    fn default() -> Self {
        let cur_dir = std::env::current_dir().unwrap();
        let file_path = format!("{}/my_terrain.wgen", cur_dir.display());
        Self { file_path, cur_dir }
    }
}

impl PanelSaveLoad {
    pub fn get_file_path(&self) -> &str {
        &self.file_path
    }
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<SaveLoadAction> {
        let mut action = None;
        ui.heading("Save/load project");
        ui.horizontal(|ui| {
            ui.label("File path");
            if ui.button("Pick...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.cur_dir)
                    .pick_file()
                {
                    self.file_path = path.display().to_string();
                    self.cur_dir = if path.is_file() {
                        path.parent().unwrap().to_path_buf()
                    } else {
                        path
                    };
                }
            }
        });
        ui.add(egui::TextEdit::singleline(&mut self.file_path).desired_width(TEXTEDIT_WIDTH));
        ui.horizontal(|ui| {
            if ui.button("Load!").clicked() {
                action = Some(SaveLoadAction::Load);
            }
            if ui.button("Save!").clicked() {
                action = Some(SaveLoadAction::Save);
            }
        });
        action
    }
}
