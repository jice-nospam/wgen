use std::path::PathBuf;

use eframe::egui;

pub const TEXTEDIT_WIDTH: f32 = 240.0;

#[derive(Clone)]
pub enum ExportFileType {
    PNG,
    EXR,
}

impl std::fmt::Display for ExportFileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::PNG => "png",
                Self::EXR => "exr",
            }
        )
    }
}

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
    /// should we repeat the same pixel row on two adjacent tiles ?
    /// not needed for unreal engine which handles multi-textures heightmaps
    /// might be needed for other engines (for example godot heightmap terrain plugin)
    pub seamless: bool,
    /// format to export, either png or exr
    pub file_type: ExportFileType,
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
            seamless: false,
            file_type: ExportFileType::PNG,
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
                        } else if self.file_path.ends_with(".exr") {
                            self.file_path =
                                self.file_path.strip_suffix(".exr").unwrap().to_owned();
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
                ui.label("_x*_y*.");
                if ui
                    .button(&self.file_type.to_string())
                    .on_hover_text("change the exported file format")
                    .clicked()
                {
                    match self.file_type {
                        ExportFileType::PNG => self.file_type = ExportFileType::EXR,
                        ExportFileType::EXR => self.file_type = ExportFileType::PNG,
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.seamless, "seamless")
                    .on_hover_text("whether pixel values are repeated on two adjacent tiles");
                export = ui.button("Export!").clicked();
            });
        });
        export
    }
}
