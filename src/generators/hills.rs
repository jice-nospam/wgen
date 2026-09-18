use rand::{prelude::*, rngs::StdRng};
use serde::{Deserialize, Serialize};

use super::{par_rows, Progress};

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
        ui.label("count")
            .on_hover_text("How many hills to scatter over the map");
        ui.add(
            egui::DragValue::new(&mut conf.nb_hill)
                .speed(1.0)
                .range(1.0..=5000.0),
        );
        ui.label("radius")
            .on_hover_text("Typical size of a hill, relative to the map size");
        ui.add(
            egui::DragValue::new(&mut conf.base_radius)
                .speed(1.0)
                .range(1.0..=255.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("radius variation").on_hover_text(
            "How much hill sizes vary: 0 = all the same, 1 = from tiny to twice the size",
        );
        ui.add(
            egui::DragValue::new(&mut conf.radius_var)
                .speed(0.01)
                .range(0.0..=1.0),
        );
    });
}

/// one disc: centre, squared radius, height per unit of `radius2 - dist2`, and its cell bounds
struct Hill {
    x: f32,
    y: f32,
    radius2: f32,
    coef: f32,
    minx: usize,
    maxx: usize,
    miny: usize,
    maxy: usize,
}

pub fn gen_hills(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &HillsConf,
    progress: &mut Progress,
) {
    let hills = hill_list(seed, size, conf);
    par_rows(size.0, hmap, progress, (0.0, 1.0), |y, row| {
        add_hills_row(y, row, &hills)
    });
}

/// draws every hill from the seed, in the order radius (skipped when `radius_var == 0`), x, y
fn hill_list(seed: u64, size: (usize, usize), conf: &HillsConf) -> Vec<Hill> {
    let mut rng = StdRng::seed_from_u64(seed);
    let real_radius = conf.base_radius * size.0 as f32 / 200.0;
    let hill_min_radius = real_radius * (1.0 - conf.radius_var);
    let hill_max_radius = real_radius * (1.0 + conf.radius_var);
    (0..conf.nb_hill)
        .map(|_| {
            let radius: f32 = if conf.radius_var == 0.0 {
                hill_min_radius
            } else {
                rng.random_range(hill_min_radius..hill_max_radius)
            };
            let x: f32 = rng.random_range(0.0..size.0 as f32);
            let y: f32 = rng.random_range(0.0..size.1 as f32);
            let radius2 = radius * radius;
            Hill {
                x,
                y,
                radius2,
                coef: conf.height / radius2,
                minx: (x - radius).max(0.0) as usize,
                maxx: (x + radius).min(size.0 as f32) as usize,
                miny: (y - radius).max(0.0) as usize,
                maxy: (y + radius).min(size.1 as f32) as usize,
            }
        })
        .collect()
}

/// adds every hill crossing row `y`, in list order, so each cell sums its contributions as the serial loop did
fn add_hills_row(y: usize, row: &mut [f32], hills: &[Hill]) {
    for hill in hills.iter().filter(|h| h.miny <= y && y < h.maxy) {
        let ydist = (y as f32 - hill.y).powi(2);
        for (px, h) in row.iter_mut().enumerate().take(hill.maxx).skip(hill.minx) {
            let z = hill.radius2 - (px as f32 - hill.x).powi(2) - ydist;
            if z > 0.0 {
                *h += z * hill.coef;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hills_are_deterministic_and_never_dig() {
        let conf = HillsConf {
            nb_hill: 20,
            ..Default::default()
        };
        let mut a = vec![0.0; 16 * 16];
        let mut b = vec![0.0; 16 * 16];
        gen_hills(3, (16, 16), &mut a, &conf, &mut Progress::headless());
        gen_hills(3, (16, 16), &mut b, &conf, &mut Progress::headless());
        assert_eq!(a, b);
        assert!(a.iter().all(|&v| v >= 0.0));
        assert!(a.iter().any(|&v| v > 0.0));
    }

    #[test]
    fn hills_row_pass_matches_per_hill_accumulation() {
        let size = (32, 32);
        let conf = HillsConf {
            nb_hill: 50,
            ..Default::default()
        };
        let mut rows = vec![0.0; 32 * 32];
        gen_hills(7, size, &mut rows, &conf, &mut Progress::headless());
        // the serial loop: hill outer, rows inner
        let mut serial = vec![0.0; 32 * 32];
        for hill in hill_list(7, size, &conf) {
            for py in hill.miny..hill.maxy {
                let ydist = (py as f32 - hill.y).powi(2);
                for px in hill.minx..hill.maxx {
                    let z = hill.radius2 - (px as f32 - hill.x).powi(2) - ydist;
                    if z > 0.0 {
                        serial[px + py * 32] += z * hill.coef;
                    }
                }
            }
        }
        assert_eq!(rows, serial);
    }
}
