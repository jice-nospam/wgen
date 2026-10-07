use serde::{Deserialize, Serialize};

use crate::height_range::drag_meters;
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
        drag_meters(ui, &mut conf.min, 1.0);
        conf.min = conf.min.min(conf.max);
        ui.label("max")
            .on_hover_text("Height of the highest point after the step");
        drag_meters(ui, &mut conf.max, 1.0);
        conf.max = conf.max.max(conf.min);
    });
}
