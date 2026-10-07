use serde::{Deserialize, Serialize};

use super::normalize;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct NormalizeConf {
    pub min: f32,
    pub max: f32,
}

impl Default for NormalizeConf {
    fn default() -> Self {
        Self { min: 0.0, max: 1.0 }
    }
}

pub fn gen_normalize(hmap: &mut [f32], conf: &NormalizeConf) {
    normalize(hmap, conf.min, conf.max);
}

pub fn render_normalize(ui: &mut egui::Ui, conf: &mut NormalizeConf) {
    ui.horizontal(|ui| {
        ui.label("min")
            .on_hover_text("Height of the lowest point after the step");
        ui.add(
            egui::DragValue::new(&mut conf.min)
                .speed(0.01)
                .range(f32::MIN..=conf.max),
        );
        ui.label("max")
            .on_hover_text("Height of the highest point after the step");
        ui.add(
            egui::DragValue::new(&mut conf.max)
                .speed(0.01)
                .range(conf.min..=f32::MAX),
        );
    });
}
