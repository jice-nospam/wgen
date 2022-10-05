use std::path::PathBuf;

use eframe::egui;

pub const TEXTEDIT_WIDTH: f32 = 240.0;

#[derive(Clone)]
pub struct PanelExport {
    /// width of each image in pixels
    pub export_width: f32,
    /// height of each image in pixels
    pub export_height: f32,
    /// number of horizontal tiles
    pub tiles_h: f32,
    /// number of vertical tiles
    pub tiles_v: f32,
    /// image filename prefix
    pub file_path: String,
    /// to disable the exporter ui during export
    pub enabled: bool,
    /// program's current directory
    cur_dir: PathBuf,
}

impl Default for PanelExport {
    fn default() -> Self {
        let cur_dir = std::env::current_dir().unwrap();
        let file_path = format!("{}/wgen", cur_dir.display());
        Self {
            export_width: 1024.0,
            export_height: 1024.0,
            tiles_h: 1.0,
            tiles_v: 1.0,
            file_path,
            enabled: true,
            cur_dir,
        }
    }
}

impl PanelExport {
    pub fn render(&mut self, ui: &mut egui::Ui, progress: f32, progress_text: &str) -> bool {
        let mut export = false;
        ui.horizontal(|ui| {
            ui.heading("Export heightmaps");
            if !self.enabled {
                ui.spinner();
            }
        });
        ui.add(egui::ProgressBar::new(progress).text(progress_text));
        ui.add_enabled_ui(self.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Tile size");
                ui.add(egui::DragValue::new(&mut self.export_width).speed(1.0));
                ui.label(" x ");
                ui.add(egui::DragValue::new(&mut self.export_height).speed(1.0));
            });
            ui.horizontal(|ui| {
                ui.label("Tiles");
                ui.add(egui::DragValue::new(&mut self.tiles_h).speed(1.0));
                ui.label(" x ");
                ui.add(egui::DragValue::new(&mut self.tiles_v).speed(1.0));
            });
            ui.horizontal(|ui| {
                ui.label("Export file path");
                if ui.button("Pick...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_directory(&self.cur_dir)
                        .pick_file()
                    {
                        self.file_path = path.display().to_string();
                        if self.file_path.ends_with(".png") {
                            self.file_path =
                                self.file_path.strip_suffix(".png").unwrap().to_owned();
                        }
                        self.cur_dir = if path.is_file() {
                            path.parent().unwrap().to_path_buf()
                        } else {
                            path
                        };
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.file_path)
                        .desired_width(TEXTEDIT_WIDTH - 80.0),
                );
                ui.label("_x*_y*.png");
            });
            export = ui.button("Export!").clicked();
        });
        export
    }
}
