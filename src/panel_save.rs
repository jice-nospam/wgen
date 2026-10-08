use std::path::PathBuf;

use crate::panel_export::TEXTEDIT_WIDTH;
pub struct PanelSaveLoad {
    /// the name of the file to load or save
    pub file_path: String,
    /// the program's current directory
    cur_dir: PathBuf,
    /// shows "File saved" next to the buttons until the next load or save
    saved: bool,
}

pub enum SaveLoadAction {
    Save,
    Load,
}

impl Default for PanelSaveLoad {
    fn default() -> Self {
        let cur_dir = std::env::current_dir().unwrap();
        let file_path = format!("{}/my_terrain.wgen", cur_dir.display());
        Self {
            file_path,
            cur_dir,
            saved: false,
        }
    }
}

impl PanelSaveLoad {
    /// called once a save has succeeded
    pub fn set_saved(&mut self) {
        self.saved = true;
    }
    pub fn get_file_path(&self) -> &str {
        &self.file_path
    }
    /// `saving` disables the buttons and shows an animated progress bar next to them
    pub fn render(&mut self, ui: &mut egui::Ui, saving: bool) -> Option<SaveLoadAction> {
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
            ui.add_enabled_ui(!saving, |ui| {
                if ui.button("Load!").clicked() {
                    action = Some(SaveLoadAction::Load);
                }
                if ui.button("Save!").clicked() {
                    action = Some(SaveLoadAction::Save);
                }
            });
            if action.is_some() {
                self.saved = false;
            }
            if saving {
                ui.add(egui::ProgressBar::new(0.0).animate(true).text("Saving..."));
            } else if self.saved {
                ui.label("File saved");
            }
        });
        action
    }
}
