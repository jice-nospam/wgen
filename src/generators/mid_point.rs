use std::sync::mpsc::Sender;

use eframe::egui;
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::ThreadMessage;

use super::report_progress;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct MidPointConf {
    pub roughness: f32,
}

impl Default for MidPointConf {
    fn default() -> Self {
        Self { roughness: 0.7 }
    }
}

pub struct ProgressTracking {
    count: usize,
    progress: f32,
    min_progress_step: f32,
    export: bool,
}

pub fn render_mid_point(ui: &mut egui::Ui, conf: &mut MidPointConf) {
    ui.horizontal(|ui| {
        ui.label("roughness");
        ui.add(
            egui::DragValue::new(&mut conf.roughness)
                .speed(0.01)
                .clamp_range(0.01..=1.0),
        );
    });
}

pub fn gen_mid_point(
    seed: u64,
    size: (usize, usize),
    hmap: &mut Vec<f32>,
    conf: &MidPointConf,
    export: bool,
    tx: Sender<ThreadMessage>,
    min_progress_step: f32,
) {
    let mut rng = StdRng::seed_from_u64(seed);
    hmap[0] = rng.gen_range(0.0, 1.0);
    hmap[size.0 - 1] = rng.gen_range(0.0, 1.0);
    hmap[size.0 * (size.1 - 1)] = rng.gen_range(0.0, 1.0);
    hmap[size.0 * size.1 - 1] = rng.gen_range(0.0, 1.0);
    let mut track = ProgressTracking {
        count: size.0 * size.1 * 2,
        progress: 0.0,
        min_progress_step,
        export,
    };
    diamond_square(
        hmap,
        &mut rng,
        size,
        size.0 / 2,
        conf.roughness,
        &mut track,
        tx,
    );
}

fn check_progress(track: &mut ProgressTracking, size: (usize, usize), tx: Sender<ThreadMessage>) {
    let new_progress = 1.0 - track.count as f32 / (size.0 * size.1 * 2) as f32;
    if new_progress - track.progress >= track.min_progress_step {
        track.progress = new_progress;
        report_progress(track.progress, track.export, tx);
    }
}

pub fn diamond_square(
    hmap: &mut Vec<f32>,
    rng: &mut StdRng,
    size: (usize, usize),
    cur_size: usize,
    roughness: f32,
    track: &mut ProgressTracking,
    tx: Sender<ThreadMessage>,
) {
    let half = cur_size / 2;
    if half < 1 {
        return;
    }
    for y in (half..size.1).step_by(cur_size) {
        for x in (half..size.0).step_by(cur_size) {
            square_step(hmap, rng, x, y, size, half, roughness);
            track.count -= 1;
            check_progress(track, size, tx.clone());
        }
    }
    let mut col = 0;
    for x in (0..size.0).step_by(half) {
        col += 1;
        if col % 2 == 1 {
            for y in (half..size.1).step_by(cur_size) {
                diamond_step(hmap, rng, x, y, size, half, roughness);
                track.count -= 1;
                check_progress(track, size, tx.clone());
            }
        } else {
            for y in (0..size.1).step_by(cur_size) {
                diamond_step(hmap, rng, x, y, size, half, roughness);
                track.count -= 1;
                check_progress(track, size, tx.clone());
            }
        }
    }
    diamond_square(hmap, rng, size, cur_size / 2, roughness * 0.5, track, tx);
}

fn square_step(
    hmap: &mut [f32],
    rng: &mut StdRng,
    x: usize,
    y: usize,
    size: (usize, usize),
    reach: usize,
    roughness: f32,
) {
    let mut count = 0;
    let mut avg = 0.0;
    if x >= reach && y >= reach {
        avg += hmap[x - reach + (y - reach) * size.0];
        count += 1;
    }
    if x >= reach && y + reach < size.1 {
        avg += hmap[x - reach + (y + reach) * size.0];
        count += 1;
    }
    if x + reach < size.0 && y >= reach {
        avg += hmap[x + reach + (y - reach) * size.0];
        count += 1;
    }
    if x + reach < size.0 && y + reach < size.1 {
        avg += hmap[x + reach + (y + reach) * size.0];
        count += 1;
    }
    avg /= count as f32;
    avg += rng.gen_range(-roughness, roughness);
    hmap[x + y * size.0] = avg;
}

fn diamond_step(
    hmap: &mut [f32],
    rng: &mut StdRng,
    x: usize,
    y: usize,
    size: (usize, usize),
    reach: usize,
    roughness: f32,
) {
    let mut count = 0;
    let mut avg = 0.0;
    if x >= reach {
        avg += hmap[x - reach + y * size.0];
        count += 1;
    }
    if x + reach < size.0 {
        avg += hmap[x + reach + y * size.0];
        count += 1;
    }
    if y >= reach {
        avg += hmap[x + (y - reach) * size.0];
        count += 1;
    }
    if y + reach < size.1 {
        avg += hmap[x + (y + reach) * size.0];
        count += 1;
    }
    avg /= count as f32;
    avg += rng.gen_range(-roughness, roughness);
    hmap[x + y * size.0] = avg;
}
