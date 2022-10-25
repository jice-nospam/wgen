use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{vec_get_safe, DIRX, DIRY};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct MudSlideConf {
    iterations: f32,
    max_erosion_alt: f32,
    strength: f32,
    water_level: f32,
}

impl Default for MudSlideConf {
    fn default() -> Self {
        Self {
            iterations: 5.0,
            max_erosion_alt: 0.9,
            strength: 0.4,
            water_level: 0.12,
        }
    }
}

pub fn render_mudslide(ui: &mut egui::Ui, conf: &mut MudSlideConf) {
    ui.horizontal(|ui| {
        ui.label("iterations");
        ui.add(
            egui::DragValue::new(&mut conf.iterations)
                .speed(0.5)
                .clamp_range(1.0..=10.0),
        );
        ui.label("max altitude");
        ui.add(
            egui::DragValue::new(&mut conf.max_erosion_alt)
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
        ui.label("water level");
        ui.add(
            egui::DragValue::new(&mut conf.water_level)
                .speed(0.01)
                .clamp_range(0.0..=1.0),
        );
    });
}

pub fn gen_mudslide(size: (usize, usize), hmap: &mut Vec<f32>, conf: &MudSlideConf) {
    for _ in 0..conf.iterations as usize {
        mudslide(size, hmap, conf);
    }
}

fn mudslide(size: (usize, usize), hmap: &mut Vec<f32>, conf: &MudSlideConf) {
    let sand_coef = 1.0 / (1.0 - conf.water_level);
    let mut new_hmap = vec![0.0; size.0 * size.1];
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let h = vec_get_safe(hmap, x + yoff);
            if h < conf.water_level - 0.01 || h >= conf.max_erosion_alt {
                new_hmap[x + y * size.0] = h;
                continue;
            }
            let mut sum_delta1 = 0.0;
            let mut sum_delta2 = 0.0;
            let mut nb1 = 1.0;
            let mut nb2 = 1.0;
            for i in 1..9 {
                let ix = (x as i32 + DIRX[i]) as usize;
                let iy = (y as i32 + DIRY[i]) as usize;
                if ix < size.0 && iy < size.1 {
                    let ih = vec_get_safe(hmap, ix + iy * size.0);
                    if ih < h {
                        if i == 1 || i == 3 || i == 6 || i == 8 {
                            // diagonal neighbour
                            sum_delta1 += (ih - h) * 0.4;
                            nb1 += 1.0;
                        } else {
                            // adjacent neighbour
                            sum_delta2 += (ih - h) * 1.6;
                            nb2 += 1.0;
                        }
                    }
                }
            }
            // average height difference with lower neighbours
            let mut dh = sum_delta1 / nb1 + sum_delta2 / nb2;
            dh *= conf.strength;
            let hcoef = (h - conf.water_level) * sand_coef;
            dh *= 1.0 - hcoef * hcoef * hcoef; // less smoothing at high altitudes
            new_hmap[x + y * size.0] = h + dh;
        }
    }
    *hmap = new_hmap;
}
