use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{Progress, DIRX, DIRY};

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
        ui.label("iterations")
            .on_hover_text("How many smoothing passes to run: more = softer terrain");
        ui.add(
            egui::DragValue::new(&mut conf.iterations)
                .speed(0.5)
                .range(1.0..=10.0),
        );
        ui.label("max altitude")
            .on_hover_text("Land above this height is left untouched");
        ui.add(
            egui::DragValue::new(&mut conf.max_erosion_alt)
                .speed(0.01)
                .range(0.0..=1.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("strength")
            .on_hover_text("How much each pass softens the slopes");
        ui.add(
            egui::DragValue::new(&mut conf.strength)
                .speed(0.01)
                .range(0.0..=1.0),
        );
        ui.label("water level")
            .on_hover_text("Land below this height is left untouched");
        ui.add(
            egui::DragValue::new(&mut conf.water_level)
                .speed(0.01)
                .range(0.0..=1.0),
        );
    });
}

pub fn gen_mudslide(
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &MudSlideConf,
    progress: &mut Progress,
) {
    let mut scratch = vec![0.0; size.0 * size.1];
    for i in 0..conf.iterations as usize {
        // a cancelled pass leaves `hmap` untouched
        if !mudslide(size, hmap, &mut scratch, conf, i, progress) {
            return;
        }
        hmap.copy_from_slice(&scratch);
    }
}

/// one smoothing pass, reading `hmap` and writing `out`; false when the step was cancelled
fn mudslide(
    size: (usize, usize),
    hmap: &[f32],
    out: &mut [f32],
    conf: &MudSlideConf,
    iteration: usize,
    progress: &mut Progress,
) -> bool {
    let sand_coef = 1.0 / (1.0 - conf.water_level);
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let h = hmap[x + yoff];
            if h < conf.water_level - 0.01 || h >= conf.max_erosion_alt {
                out[x + yoff] = h;
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
                    let ih = hmap[ix + iy * size.0];
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
            out[x + yoff] = h + dh;
        }
        let p = iteration as f32 / conf.iterations as f32
            + (y as f32 / size.1 as f32) / conf.iterations as f32;
        if !progress.report(p) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mudslide_is_deterministic_and_keeps_a_flat_map_flat() {
        let conf = MudSlideConf::default();
        let mut flat = vec![0.5; 8 * 8];
        gen_mudslide((8, 8), &mut flat, &conf, &mut Progress::headless());
        assert!(flat.iter().all(|&v| v == 0.5));
        let mut a: Vec<f32> = (0..64).map(|i| ((i * 7) % 11) as f32 / 11.0).collect();
        let mut b = a.clone();
        gen_mudslide((8, 8), &mut a, &conf, &mut Progress::headless());
        gen_mudslide((8, 8), &mut b, &conf, &mut Progress::headless());
        assert_eq!(a, b);
    }

    #[test]
    fn cancelled_mudslide_leaves_map_untouched() {
        let conf = MudSlideConf {
            iterations: 3.0,
            ..Default::default()
        };
        let ramp: Vec<f32> = (0..64).map(|i| i as f32 / 63.0).collect();
        let mut run = ramp.clone();
        gen_mudslide((8, 8), &mut run, &conf, &mut Progress::headless());
        assert_ne!(run, ramp, "an uncancelled run must change the ramp");
        let (tx, _) = std::sync::mpsc::channel();
        let mut cancelled = ramp.clone();
        gen_mudslide(
            (8, 8),
            &mut cancelled,
            &conf,
            &mut Progress::preview(tx, 1.0, || true),
        );
        assert_eq!(cancelled, ramp);
    }
}
