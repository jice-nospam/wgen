use std::sync::mpsc::Sender;

use eframe::egui;
use rand::{prelude::*, rngs::StdRng};
use serde::{Deserialize, Serialize};

use crate::ThreadMessage;

use super::report_progress;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct HillsConf {
    pub nb_hill: usize,
    pub base_radius: f32,
    pub radius_var: f32,
    pub height: f32,
}

impl Default for HillsConf {
    fn default() -> Self {
        Self {
            nb_hill: 600,
            base_radius: 16.0,
            radius_var: 0.7,
            height: 0.3,
        }
    }
}

pub fn render_hills(ui: &mut egui::Ui, conf: &mut HillsConf) {
    ui.horizontal(|ui| {
        ui.label("count");
        ui.add(
            egui::DragValue::new(&mut conf.nb_hill)
                .speed(1.0)
                .clamp_range(1.0..=5000.0),
        );
        ui.label("radius");
        ui.add(
            egui::DragValue::new(&mut conf.base_radius)
                .speed(1.0)
                .clamp_range(1.0..=255.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("radius variation");
        ui.add(
            egui::DragValue::new(&mut conf.radius_var)
                .speed(0.01)
                .clamp_range(0.0..=1.0),
        );
    });
}

pub fn gen_hills(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &HillsConf,
    export: bool,
    tx: Sender<ThreadMessage>,
    min_progress_step: f32,
) {
    let mut rng = StdRng::seed_from_u64(seed);
    let real_radius = conf.base_radius * size.0 as f32 / 200.0;
    let hill_min_radius = real_radius * (1.0 - conf.radius_var);
    let hill_max_radius = real_radius * (1.0 + conf.radius_var);
    let mut progress = 0.0;
    for i in 0..conf.nb_hill {
        let radius: f32 = if conf.radius_var == 0.0 {
            hill_min_radius
        } else {
            rng.random_range(hill_min_radius..hill_max_radius)
        };
        let xh: f32 = rng.random_range(0.0..size.0 as f32);
        let yh: f32 = rng.random_range(0.0..size.1 as f32);
        let radius2 = radius * radius;
        let coef = conf.height / radius2;
        let minx = (xh - radius).max(0.0) as usize;
        let maxx = (xh + radius).min(size.0 as f32) as usize;
        let miny = (yh - radius).max(0.0) as usize;
        let maxy = (yh + radius).min(size.1 as f32) as usize;
        for px in minx..maxx {
            let xdist = (px as f32 - xh).powi(2);
            for py in miny..maxy {
                let z = radius2 - xdist - (py as f32 - yh).powi(2);
                if z > 0.0 {
                    hmap[px + py * size.0] += z * coef;
                }
            }
        }
        let new_progress = i as f32 / conf.nb_hill as f32;
        if new_progress - progress >= min_progress_step {
            progress = new_progress;
            report_progress(progress, export, tx.clone());
        }
    }
}
