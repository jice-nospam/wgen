use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{get_min_max, Progress};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct IslandConf {
    pub coast_range: f32,
}

impl Default for IslandConf {
    fn default() -> Self {
        Self { coast_range: 50.0 }
    }
}

pub fn render_island(ui: &mut egui::Ui, conf: &mut IslandConf) {
    ui.horizontal(|ui| {
        ui.label("coast range %");
        ui.add(
            egui::DragValue::new(&mut conf.coast_range)
                .speed(0.1)
                .range(0.1..=50.0),
        );
    });
}

pub fn gen_island(
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &IslandConf,
    progress: &mut Progress,
) {
    let coast_h_dist = size.0 as f32 * conf.coast_range / 100.0;
    let coast_v_dist = size.1 as f32 * conf.coast_range / 100.0;
    let (min, _) = get_min_max(hmap);
    for x in 0..size.0 {
        for y in 0..coast_v_dist as usize {
            let h_coef = y as f32 / coast_v_dist as f32;
            let h = hmap[x + y * size.0];
            hmap[x + y * size.0] = (h - min) * h_coef + min;
            let h = hmap[x + (size.1 - 1 - y) * size.0];
            hmap[x + (size.1 - 1 - y) * size.0] = (h - min) * h_coef + min;
        }
        if !progress.report(0.5 * x as f32 / size.0 as f32) {
            return;
        }
    }
    for y in 0..size.1 {
        for x in 0..coast_h_dist as usize {
            let h_coef = x as f32 / coast_h_dist as f32;
            let h = hmap[x + y * size.0];
            hmap[x + y * size.0] = (h - min) * h_coef + min;
            let h = hmap[(size.0 - 1 - x) + y * size.0];
            hmap[(size.0 - 1 - x) + y * size.0] = (h - min) * h_coef + min;
        }
        if !progress.report(0.5 + 0.5 * y as f32 / size.1 as f32) {
            return;
        }
    }
}
