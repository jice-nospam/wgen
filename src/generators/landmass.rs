use serde::{Deserialize, Serialize};

use super::{normalize, par_rows, Progress};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct LandMassConf {
    /// what proportion of the map should be above water 0.0-1.0
    pub land_proportion: f32,
    /// height of the water plane
    pub water_level: f32,
    /// apply h^plain_factor above sea level for sharper mountains and flatter plains
    pub plain_factor: f32,
}

impl Default for LandMassConf {
    fn default() -> Self {
        Self {
            land_proportion: 0.6,
            water_level: 0.12,
            plain_factor: 2.5,
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
    });
}

pub fn gen_landmass(
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &LandMassConf,
    progress: &mut Progress,
) {
    let mut height_count: [usize; 256] = [0; 256];
    normalize(hmap, 0.0, 1.0);
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let h = hmap[x + yoff];
            let ih = (h * 255.0) as usize;
            height_count[ih] += 1;
        }
        if !progress.report(0.33 * y as f32 / size.1 as f32) {
            return;
        }
    }
    let mut water_level = 0;
    let mut water_cells = 0;
    let target_water_cells = (size.0 * size.1) as f32 * (1.0 - conf.land_proportion);
    while water_level < 256 && (water_cells as f32) < target_water_cells {
        water_cells += height_count[water_level];
        water_level += 1;
    }
    // keep both coefficients finite when every cell ends up on one side of the water level
    let new_water_level = (water_level as f32 / 255.0).clamp(1.0 / 255.0, 254.0 / 255.0);
    let land_coef = (1.0 - conf.water_level) / (1.0 - new_water_level);
    let water_coef = conf.water_level / new_water_level;
    // water level should be raised/lowered to newWaterLevel
    if !par_rows(size.0, hmap, progress, (0.33, 0.66), |_, row| {
        landmass_row_rescale(row, new_water_level, land_coef, water_coef, conf)
    }) {
        return;
    }
    // fix land/mountain ratio using h^plain_factor curve above sea level
    par_rows(size.0, hmap, progress, (0.66, 1.0), |_, row| {
        landmass_row_plain(row, conf)
    });
}

/// moves the found water level to `conf.water_level`, stretching land and sea separately;
/// both sides meet at `conf.water_level`, so the shoreline has no step
fn landmass_row_rescale(
    row: &mut [f32],
    new_water_level: f32,
    land_coef: f32,
    water_coef: f32,
    conf: &LandMassConf,
) {
    for h in row {
        if *h > new_water_level {
            *h = conf.water_level + (*h - new_water_level) * land_coef;
        } else {
            *h *= water_coef;
        }
    }
}

/// applies the h^plain_factor curve above sea level
fn landmass_row_plain(row: &mut [f32], conf: &LandMassConf) {
    for h in row {
        if *h >= conf.water_level {
            let coef = (*h - conf.water_level) / (1.0 - conf.water_level);
            let coef = coef.powf(conf.plain_factor);
            *h = conf.water_level + coef * (1.0 - conf.water_level);
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

    #[test]
    fn landmass_moves_the_water_level_to_the_requested_share() {
        let conf = LandMassConf {
            land_proportion: 0.5,
            water_level: 0.12,
            ..Default::default()
        };
        let mut h: Vec<f32> = (0..256).map(|i| i as f32 / 255.0).collect();
        gen_landmass((16, 16), &mut h, &conf, &mut Progress::headless());
        let land = h.iter().filter(|&&v| v >= 0.12).count();
        assert!((120..=136).contains(&land), "{land} land cells");
    }

    /// a ramp stays a ramp: no step at the shoreline, whatever the old `shore_height`
    #[test]
    fn landmass_is_continuous_at_the_shoreline() {
        let conf = LandMassConf {
            land_proportion: 0.5,
            water_level: 0.12,
            plain_factor: 1.0,
        };
        let mut h: Vec<f32> = (0..256).map(|i| i as f32 / 255.0).collect();
        gen_landmass((16, 16), &mut h, &conf, &mut Progress::headless());
        let max_jump = h
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_jump < 0.01, "step of {max_jump} between two cells");
        assert!(h[0].abs() < 1e-6, "the sea floor starts at 0, not {}", h[0]);
    }

    /// files written with the removed `shore_height` field still load
    #[test]
    fn landmass_conf_ignores_shore_height() {
        let old = "(land_proportion:0.6,water_level:0.12,plain_factor:2.5,shore_height:0.1)";
        let conf: LandMassConf = ron::from_str(old).unwrap();
        assert_eq!(conf, LandMassConf::default());
    }
}
