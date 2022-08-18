use std::sync::mpsc::Sender;

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::ThreadMessage;

use super::{normalize, report_progress};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct LandMassConf {
    pub land_proportion: f32,
    pub water_level: f32,
}

impl Default for LandMassConf {
    fn default() -> Self {
        Self {
            land_proportion: 0.6,
            water_level: 0.12,
        }
    }
}

pub fn render_landmass(ui: &mut egui::Ui, conf: &mut LandMassConf) {
    ui.horizontal(|ui| {
        ui.label("land proportion");
        ui.add(
            egui::DragValue::new(&mut conf.land_proportion)
                .speed(0.01)
                .clamp_range(0.0..=1.0),
        );
        ui.label("water level");
        ui.add(
            egui::DragValue::new(&mut conf.water_level)
                .speed(0.01)
                .clamp_range(0.0..=1.0),
        );
    });
}

pub fn gen_landmass(
    size: (usize, usize),
    hmap: &mut Vec<f32>,
    conf: &LandMassConf,
    export: bool,
    tx: Sender<ThreadMessage>,
    min_progress_step: f32,
) {
    let mut height_count: [f32; 256] = [0.0; 256];
    let mut progress = 0.0;
    normalize(hmap, 0.0, 1.0);
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let h = hmap[x + yoff];
            let ih = (h * 255.0) as usize;
            height_count[ih] += 1.0;
        }
        let new_progress = 0.33 * y as f32 / size.1 as f32;
        if new_progress - progress >= min_progress_step {
            progress = new_progress;
            report_progress(progress, export, tx.clone());
        }
    }
    let mut water_level = 0;
    let mut water_cells = 0.0;
    let target_water_cells = (size.0 * size.1) as f32 * (1.0 - conf.land_proportion);
    while water_level < 256 && water_cells < target_water_cells {
        water_cells += height_count[water_level];
        water_level += 1;
    }
    let new_water_level = water_level as f32 / 255.0;
    let land_coef = (1.0 - conf.water_level) / (1.0 - new_water_level);
    let water_coef = conf.water_level / new_water_level;
    // water level should be raised/lowered to newWaterLevel
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let mut h = hmap[x + yoff];
            if h > new_water_level {
                h = conf.water_level + (h - new_water_level) * land_coef;
            } else {
                h = h * water_coef;
            }
            hmap[x + yoff] = h;
        }
        let new_progress = 0.33 + 0.33 * y as f32 / size.1 as f32;
        if new_progress - progress >= min_progress_step {
            progress = new_progress;
            report_progress(progress, export, tx.clone());
        }
    }
    // fix land/mountain ratio using x^3 curve above sea level
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let mut h = hmap[x + yoff];
            if h >= conf.water_level {
                let coef = (h - conf.water_level) / (1.0 - conf.water_level);
                h = conf.water_level + coef * coef * coef * (1.0 - conf.water_level);
                hmap[x + y * size.0] = h;
            }
        }
        let new_progress = 0.66 + 0.33 * y as f32 / size.1 as f32;
        if new_progress - progress >= min_progress_step {
            progress = new_progress;
            report_progress(progress, export, tx.clone());
        }
    }
}
