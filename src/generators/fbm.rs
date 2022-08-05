use std::sync::mpsc::Sender;

use eframe::egui;
use noise::{Fbm, MultiFractal, NoiseFn, Seedable};
use serde::{Deserialize, Serialize};

use crate::ThreadMessage;

use super::report_progress;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct FbmConf {
    pub mulx: f32,
    pub muly: f32,
    pub addx: f32,
    pub addy: f32,
    pub octaves: f32,
    pub delta: f32,
    pub scale: f32,
}

impl Default for FbmConf {
    fn default() -> Self {
        Self {
            mulx: 2.20,
            muly: 2.20,
            addx: 0.0,
            addy: 0.0,
            octaves: 6.0,
            delta: 0.0,
            scale: 2.05,
        }
    }
}

pub fn render_fbm(ui: &mut egui::Ui, conf: &mut FbmConf) {
    ui.horizontal(|ui| {
        ui.label("scale x");
        ui.add(
            egui::DragValue::new(&mut conf.mulx)
                .speed(0.1)
                .clamp_range(0.0..=100.0),
        );
        ui.label("y");
        ui.add(
            egui::DragValue::new(&mut conf.muly)
                .speed(0.1)
                .clamp_range(0.0..=100.0),
        );
        ui.label("octaves");
        ui.add(
            egui::DragValue::new(&mut conf.octaves)
                .speed(0.5)
                .clamp_range(1.0..=Fbm::MAX_OCTAVES as f32),
        );
    });
    ui.horizontal(|ui| {
        ui.label("offset x");
        ui.add(
            egui::DragValue::new(&mut conf.addx)
                .speed(0.1)
                .clamp_range(0.0..=200.0),
        );
        ui.label("y");
        ui.add(
            egui::DragValue::new(&mut conf.addy)
                .speed(0.1)
                .clamp_range(0.0..=200.0),
        );
        ui.label("scale");
        ui.add(
            egui::DragValue::new(&mut conf.scale)
                .speed(0.01)
                .clamp_range(0.01..=10.0),
        );
    });
}

pub fn gen_fbm(
    seed: u64,
    size: (usize, usize),
    hmap: &mut Vec<f32>,
    conf: &FbmConf,
    export: bool,
    tx: Sender<ThreadMessage>,
    min_progress_step: f32,
) {
    let xcoef = conf.mulx / 400.0;
    let ycoef = conf.muly / 400.0;
    let mut progress = 0.0;
    let fbm = Fbm::new()
        .set_seed(seed as u32)
        .set_octaves(conf.octaves as usize);
    for x in 0..size.0 {
        let mut offset = x;
        let f0 = (x as f32 * 512.0 / size.0 as f32 + conf.addx) * xcoef;
        for y in 0..size.1 {
            let f1 = (y as f32 * 512.0 / size.1 as f32 + conf.addy) * ycoef;
            let value = conf.delta + fbm.get([f0 as f64, f1 as f64]) as f32 * conf.scale;
            hmap[offset] += value;
            offset += size.0;
        }
        let new_progress = (x + 1) as f32 / size.0 as f32;
        if new_progress - progress >= min_progress_step {
            progress = new_progress;
            report_progress(progress, export, tx.clone())
        }
    }
}
