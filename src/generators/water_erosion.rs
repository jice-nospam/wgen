use std::sync::mpsc::Sender;

use eframe::egui;
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::ThreadMessage;

use super::report_progress;

// water erosion algorithm adapted from https://www.firespark.de/resources/downloads/implementation%20of%20a%20methode%20for%20hydraulic%20erosion.pdf
const MAX_PATH_LENGTH: usize = 40;
const DEFAULT_EVAPORATION: f32 = 0.05;
const DEFAULT_CAPACITY: f32 = 8.0;
const DEFAULT_MIN_SLOPE: f32 = 0.05;
const DEFAULT_DEPOSITION: f32 = 0.1;
const DEFAULT_INERTIA: f32 = 0.4;
const DEFAULT_DROP_AMOUNT: f32 = 0.5;
const DEFAULT_EROSION_STRENGTH: f32 = 0.1;
const DEFAULT_RADIUS: f32 = 4.0;

/// a drop of water
struct Drop {
    /// position on the grid
    pub pos: (f32, f32),
    /// water amount
    pub water: f32,
    /// movement direction
    pub dir: (f32, f32),
    /// maximum sediment capacity of the drop
    pub capacity: f32,
    /// amount of accumulated sediment
    pub sediment: f32,
    /// velocity
    pub speed: f32,
}

impl Drop {
    pub fn grid_offset(&self, grid_width: usize) -> usize {
        self.pos.0.round() as usize + self.pos.1.round() as usize * grid_width
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct WaterErosionConf {
    drop_amount: f32,
    erosion_strength: f32,
    evaporation: f32,
    capacity: f32,
    min_slope: f32,
    deposition: f32,
    inertia: f32,
    radius: f32,
}

impl Default for WaterErosionConf {
    fn default() -> Self {
        Self {
            drop_amount: DEFAULT_DROP_AMOUNT,
            erosion_strength: DEFAULT_EROSION_STRENGTH,
            evaporation: DEFAULT_EVAPORATION,
            capacity: DEFAULT_CAPACITY,
            min_slope: DEFAULT_MIN_SLOPE,
            deposition: DEFAULT_DEPOSITION,
            inertia: DEFAULT_INERTIA,
            radius: DEFAULT_RADIUS,
        }
    }
}

pub fn render_water_erosion(ui: &mut egui::Ui, conf: &mut WaterErosionConf) {
    ui.horizontal(|ui| {
        ui.label("drop amount")
            .on_hover_text("Amount of drops simulated");
        ui.add(
            egui::DragValue::new(&mut conf.drop_amount)
                .speed(0.01)
                .clamp_range(0.1..=2.0),
        );
        ui.label("erosion strength")
            .on_hover_text("How much soil is eroded by the drop");
        ui.add(
            egui::DragValue::new(&mut conf.erosion_strength)
                .speed(0.01)
                .clamp_range(0.01..=1.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("drop capacity")
            .on_hover_text("How much sediment a drop can contain");
        ui.add(
            egui::DragValue::new(&mut conf.capacity)
                .speed(0.5)
                .clamp_range(2.0..=32.0),
        );
        ui.label("inertia")
            .on_hover_text("Inertia of the drop. Increase for smoother result");
        ui.add(
            egui::DragValue::new(&mut conf.inertia)
                .speed(0.01)
                .clamp_range(0.01..=0.5),
        );
    });
    ui.horizontal(|ui| {
        ui.label("deposition")
            .on_hover_text("Amount of sediment deposited");
        ui.add(
            egui::DragValue::new(&mut conf.deposition)
                .speed(0.01)
                .clamp_range(0.01..=1.0),
        );
        ui.label("evaporation")
            .on_hover_text("How fast the drop evaporate. Increase for smoother results");
        ui.add(
            egui::DragValue::new(&mut conf.evaporation)
                .speed(0.01)
                .clamp_range(0.01..=0.5),
        );
    });
    ui.horizontal(|ui| {
        ui.label("radius").on_hover_text("Erosion radius");
        ui.add(
            egui::DragValue::new(&mut conf.radius)
                .speed(0.1)
                .clamp_range(1.0..=10.0),
        );
        ui.label("minimum slope")
            .on_hover_text("Minimum height for the drop capacity calculation");
        ui.add(
            egui::DragValue::new(&mut conf.min_slope)
                .speed(0.001)
                .clamp_range(0.001..=0.1),
        );
    });
}

pub fn gen_water_erosion(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &WaterErosionConf,
    export: bool,
    tx: Sender<ThreadMessage>,
    min_progress_step: f32,
) {
    let mut progress = 0.0;
    let mut rng = StdRng::seed_from_u64(seed);
    // maximum drop count is 2 per cell
    let drop_count = ((size.1 * 2) as f32 * conf.drop_amount) as usize;
    // compute erosion weight depending on radius
    let mut erosion_weight = 0.0;
    for y in (-conf.radius).round() as i32..conf.radius.round() as i32 {
        for x in (-conf.radius).round() as i32..conf.radius.round() as i32 {
            let dist = ((x * x + y * y) as f32).sqrt();
            if dist < conf.radius {
                erosion_weight += conf.radius - dist;
            }
        }
    }
    // use a double loop to check progress every size.0 drops
    for y in 0..drop_count {
        for _ in 0..size.0 {
            let mut drop = Drop {
                pos: (
                    rng.gen_range(0, size.0 - 1) as f32,
                    rng.gen_range(0, size.1 - 1) as f32,
                ),
                dir: (0.0, 0.0),
                sediment: 0.0,
                water: 1.0,
                capacity: conf.capacity,
                speed: 0.0,
            };
            let mut off = drop.grid_offset(size.0);
            let mut count = 0;
            while count < MAX_PATH_LENGTH {
                let oldh = hmap[off];
                let old_off = off;
                // interpolate slope at old position
                let h00 = oldh;
                let h10 = hmap[off + 1];
                let h01 = hmap[off + size.0];
                let h11 = hmap[off + 1 + size.0];
                let old_u = drop.pos.0.fract();
                let old_v = drop.pos.1.fract();
                // weight for each cell surrounding the drop position
                let w00 = (1.0 - old_u) * (1.0 - old_v);
                let w10 = old_u * (1.0 - old_v);
                let w01 = (1.0 - old_u) * old_v;
                let w11 = old_u * old_v;
                // get slope direction
                let mut gx = (h00 - h10) * (1.0 - old_v) + (h01 - h11) * old_v;
                let mut gy = (h00 - h01) * (1.0 - old_u) + (h10 - h11) * old_u;
                (gx, gy) = normalize_dir(gx, gy, &mut rng);
                // interpolate between old direction and new one to account for inertia
                gx = (drop.dir.0 - gx) * conf.inertia + gx;
                gy = (drop.dir.1 - gy) * conf.inertia + gy;
                (drop.dir.0, drop.dir.1) = normalize_dir(gx, gy, &mut rng);
                let old_x = drop.pos.0;
                let old_y = drop.pos.1;
                // compute the droplet new position
                drop.pos.0 += drop.dir.0;
                drop.pos.1 += drop.dir.1;
                let ix = drop.pos.0.round() as usize;
                let iy = drop.pos.1.round() as usize;
                if ix >= size.0 - 1 || iy >= size.1 - 1 {
                    // out of the map
                    break;
                }
                off = drop.grid_offset(size.0);
                // interpolate height at new drop position
                let u = drop.pos.0.fract();
                let v = drop.pos.1.fract();
                let new_h00 = hmap[off];
                let new_h10 = hmap[off + 1];
                let new_h01 = hmap[off + size.0];
                let new_h11 = hmap[off + 1 + size.0];
                let newh = (new_h00 * (1.0 - u) + new_h10 * u) * (1.0 - v)
                    + (new_h01 * (1.0 - u) + new_h11 * u) * v;
                let hdif = newh - oldh;
                if hdif >= 0.0 {
                    // going uphill : deposit sediment at old position
                    let deposit = drop.sediment.min(hdif);
                    hmap[old_off] += deposit * w00;
                    hmap[old_off + 1] += deposit * w10;
                    hmap[old_off + size.0] += deposit * w01;
                    hmap[old_off + 1 + size.0] += deposit * w11;
                    drop.sediment -= deposit;
                    drop.speed = 0.0;
                    if drop.sediment <= 0.0 {
                        // no more sediment. stop the path
                        break;
                    }
                } else {
                    drop.capacity =
                        conf.min_slope.max(-hdif) * drop.water * conf.capacity * drop.speed;
                    if drop.sediment > drop.capacity {
                        // too much sediment in the drop. deposit
                        let deposit = (drop.sediment - drop.capacity) * conf.deposition;
                        hmap[old_off] += deposit * w00;
                        hmap[old_off + 1] += deposit * w10;
                        hmap[old_off + size.0] += deposit * w01;
                        hmap[old_off + 1 + size.0] += deposit * w11;
                        drop.sediment -= deposit;
                    } else {
                        // erode
                        let amount =
                            ((drop.capacity - drop.sediment) * conf.erosion_strength).min(-hdif);
                        for y in (old_y - conf.radius).round() as i32
                            ..(old_y + conf.radius).round() as i32
                        {
                            if y < 0 || y >= size.1 as i32 {
                                continue;
                            }
                            let dy = old_y - y as f32;
                            for x in (old_x - conf.radius).round() as i32
                                ..(old_x + conf.radius).round() as i32
                            {
                                if x < 0 || x >= size.0 as i32 {
                                    continue;
                                }
                                let dx = old_x - x as f32;
                                let dist = (dx * dx + dy * dy).sqrt();
                                if dist < conf.radius {
                                    let off = x as usize + y as usize * size.0;
                                    hmap[off] -= amount * (conf.radius - dist) / erosion_weight;
                                }
                            }
                        }
                        drop.sediment += amount;
                    }
                }
                drop.speed = (drop.speed * drop.speed + hdif.abs()).sqrt();
                drop.water *= 1.0 - conf.evaporation;
                count += 1;
            }
        }
        let new_progress = y as f32 / drop_count as f32;
        if new_progress - progress >= min_progress_step {
            progress = new_progress;
            report_progress(progress, export, tx.clone());
        }
    }
}

fn normalize_dir(dx: f32, dy: f32, rng: &mut StdRng) -> (f32, f32) {
    let len = (dx * dx + dy * dy).sqrt();
    if len < std::f32::EPSILON {
        // random direction
        let angle = rng.gen_range(0.0, std::f32::consts::PI * 2.0);
        (angle.cos(), angle.sin())
    } else {
        (dx / len, dy / len)
    }
}
