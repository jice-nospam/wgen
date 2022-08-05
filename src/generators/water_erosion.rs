use std::sync::mpsc::Sender;

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::ThreadMessage;

use super::{
    compute_flow_and_slopes, compute_precipitations, report_progress, DIRX, DIRY, OPPOSITE_DIR,
};

const MAX_STEP_PER_DROP: usize = 20;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct WaterErosionConf {
    iterations: f32,
    water_level: f32,
    strength: f32,
}

impl Default for WaterErosionConf {
    fn default() -> Self {
        Self {
            iterations: 5.0,
            water_level: 0.12,
            strength: 0.1,
        }
    }
}

pub fn render_water_erosion(ui: &mut egui::Ui, conf: &mut WaterErosionConf) {
    ui.horizontal(|ui| {
        ui.label("iterations");
        ui.add(
            egui::DragValue::new(&mut conf.iterations)
                .speed(0.5)
                .clamp_range(1.0..=10.0),
        );
        ui.label("water level");
        ui.add(
            egui::DragValue::new(&mut conf.water_level)
                .speed(0.01)
                .clamp_range(0.0..=1.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("strength");
        ui.add(
            egui::DragValue::new(&mut conf.strength)
                .speed(0.01)
                .clamp_range(0.0..=1.0),
        );
    });
}

pub fn gen_water_erosion(
    size: (usize, usize),
    hmap: &mut Vec<f32>,
    conf: &WaterErosionConf,
    export: bool,
    tx: Sender<ThreadMessage>,
    min_progress_step: f32,
) {
    let prec = compute_precipitations(size, hmap);
    for i in 0..conf.iterations as usize {
        let (flow, slopes) = compute_flow_and_slopes(size, hmap);
        erode(
            size,
            hmap,
            &prec,
            &flow,
            &slopes,
            i,
            conf,
            export,
            tx.clone(),
            min_progress_step,
        );
    }
}

fn erode(
    size: (usize, usize),
    hmap: &mut Vec<f32>,
    prec: &Vec<f32>,
    flow: &Vec<usize>,
    slopes: &Vec<f32>,
    iteration: usize,
    conf: &WaterErosionConf,
    export: bool,
    tx: Sender<ThreadMessage>,
    min_progress_step: f32,
) {
    let strength = conf.strength * 0.1;
    let mut progress = 0.0;
    for y in 0..size.1 {
        for x in 0..size.0 {
            let mut ix = x;
            let mut iy = y;
            let mut off = ix + iy * size.0;
            let mut old_flow = flow[off];
            let mut count = 0;
            while count < MAX_STEP_PER_DROP {
                let mut h = hmap[off];
                if h < conf.water_level - 0.01 {
                    break;
                };
                if flow[off] == OPPOSITE_DIR[old_flow] {
                    break;
                } else {
                    // erode => decrease h (slope <0)
                    let slope = slopes[off];
                    h += prec[off] * strength * slope;
                    h = h.max(conf.water_level);
                    hmap[off] = h;
                    old_flow = flow[off];
                    ix = (ix as i32 + DIRX[old_flow]) as usize;
                    iy = (iy as i32 + DIRY[old_flow]) as usize;
                }
                if ix >= size.0 || iy >= size.1 {
                    break;
                }
                off = ix + iy * size.0;
                count += 1;
            }
        }
        let new_progress = iteration as f32 / conf.iterations as f32
            + (y as f32 / size.1 as f32) / conf.iterations as f32;
        if new_progress - progress >= min_progress_step {
            progress = new_progress;
            report_progress(progress, export, tx.clone());
        }
    }
}
