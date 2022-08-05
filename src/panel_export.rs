use eframe::egui;

pub const TEXTEDIT_WIDTH: f32 = 180.0;

#[derive(Clone)]
pub struct PanelExport {
    pub export_width: f32,
    pub export_height: f32,
    pub tiles_h: f32,
    pub tiles_v: f32,
    pub export_dir: String,
    pub file_pattern: String,
    pub enabled: bool,
}

impl Default for PanelExport {
    fn default() -> Self {
        Self {
            export_width: 1024.0,
            export_height: 1024.0,
            tiles_h: 1.0,
            tiles_v: 1.0,
            export_dir: std::env::current_dir().unwrap().display().to_string(),
            file_pattern: "wgen".to_owned(),
            enabled: true,
        }
    }
}

impl PanelExport {
    pub fn render(&mut self, ui: &mut egui::Ui, progress: f32, progress_text: &str) -> bool {
        let mut export = false;
        ui.horizontal(|ui| {
            ui.heading("Export heightmaps");
            if !self.enabled {
                ui.add(egui::Spinner::new());
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
            ui.label("Export directory");
            ui.add(
                egui::TextEdit::singleline(&mut self.export_dir)
                    .hint_text("Directory")
                    .desired_width(TEXTEDIT_WIDTH),
            );
            ui.label("Export file name");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.file_pattern)
                        .hint_text("File pattern")
                        .desired_width(TEXTEDIT_WIDTH),
                );
                ui.label("_x*_y*.png");
            });
            export = ui.button("Export!").clicked();
        });
        export
    }
}
