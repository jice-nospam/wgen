use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{normalize, Progress};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct LandMassConf {
    /// what proportion of the map should be above water 0.0-1.0
    pub land_proportion: f32,
    /// height of the water plane
    pub water_level: f32,
    /// apply h^plain_factor above sea level for sharper mountains and flatter plains
    pub plain_factor: f32,
    /// lower everything under water level by this value to avoid z fighting between land and water plane near shores
    pub shore_height: f32,
}

impl Default for LandMassConf {
    fn default() -> Self {
        Self {
            land_proportion: 0.6,
            water_level: 0.12,
            plain_factor: 2.5,
            shore_height: 0.05,
        }
    }
}

pub fn render_landmass(ui: &mut egui::Ui, conf: &mut LandMassConf) {
    ui.horizontal(|ui| {
        ui.label("land proportion")
            .on_hover_text("Share of the map that ends up above water");
        ui.add(
            egui::DragValue::new(&mut conf.land_proportion)
                .speed(0.01)
                .range(0.0..=1.0),
        );
        ui.label("water level").on_hover_text("Height of the sea");
        ui.add(
            egui::DragValue::new(&mut conf.water_level)
                .speed(0.01)
                .range(0.0..=1.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("plain factor")
            .on_hover_text("Higher = flatter plains and sharper mountains");
        ui.add(
            egui::DragValue::new(&mut conf.plain_factor)
                .speed(0.01)
                .range(1.0..=4.0),
        );
        ui.label("shore height")
            .on_hover_text("How far the land under the sea is pushed down");
        ui.add(
            egui::DragValue::new(&mut conf.shore_height)
                .speed(0.01)
                .range(0.0..=0.1),
        );
    });
}

pub fn gen_landmass(
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &LandMassConf,
    progress: &mut Progress,
) {
    let mut height_count: [f32; 256] = [0.0; 256];
    normalize(hmap, 0.0, 1.0);
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let h = hmap[x + yoff];
            let ih = (h * 255.0) as usize;
            height_count[ih] += 1.0;
        }
        if !progress.report(0.33 * y as f32 / size.1 as f32) {
            return;
        }
    }
    let mut water_level = 0;
    let mut water_cells = 0.0;
    let target_water_cells = (size.0 * size.1) as f32 * (1.0 - conf.land_proportion);
    while water_level < 256 && water_cells < target_water_cells {
        water_cells += height_count[water_level];
        water_level += 1;
    }
    // keep both coefficients finite when every cell ends up on one side of the water level
    let new_water_level = (water_level as f32 / 255.0).clamp(1.0 / 255.0, 254.0 / 255.0);
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
                h = h * water_coef - conf.shore_height;
            }
            hmap[x + yoff] = h;
        }
        if !progress.report(0.33 + 0.33 * y as f32 / size.1 as f32) {
            return;
        }
    }
    // fix land/mountain ratio using h^plain_factor curve above sea level
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let mut h = hmap[x + yoff];
            if h >= conf.water_level {
                let coef = (h - conf.water_level) / (1.0 - conf.water_level);
                let coef = coef.powf(conf.plain_factor);
                h = conf.water_level + coef * (1.0 - conf.water_level);
                hmap[x + y * size.0] = h;
            }
        }
        if !progress.report(0.66 + 0.33 * y as f32 / size.1 as f32) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landmass_stays_finite_at_extreme_land_proportions() {
        for land_proportion in [0.0, 1.0] {
            let conf = LandMassConf {
                land_proportion,
                ..Default::default()
            };
            let mut h: Vec<f32> = (0..64).map(|i| i as f32 / 63.0).collect();
            gen_landmass((8, 8), &mut h, &conf, &mut Progress::headless());
            assert!(
                h.iter().all(|v| v.is_finite()),
                "NaN at {}",
                land_proportion
            );
        }
    }
}
