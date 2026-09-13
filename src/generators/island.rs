use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{get_min_max, par_rows, Progress};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct IslandConf {
    pub coast_range: f32,
}

impl Default for IslandConf {
    fn default() -> Self {
        Self { coast_range: 50.0 }
    }
}

pub fn render_island(ui: &mut egui::Ui, conf: &mut IslandConf) {
    ui.horizontal(|ui| {
        ui.label("coast range %").on_hover_text(
            "Width of the band along the edges where the land sinks into the sea, in % of the map",
        );
        ui.add(
            egui::DragValue::new(&mut conf.coast_range)
                .speed(0.1)
                .range(0.1..=50.0),
        );
    });
}

pub fn gen_island(
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &IslandConf,
    progress: &mut Progress,
) {
    let coast_h = size.0 as f32 * conf.coast_range / 100.0;
    let coast_v = size.1 as f32 * conf.coast_range / 100.0;
    let (min, _) = get_min_max(hmap);
    par_rows(size.0, hmap, progress, (0.0, 1.0), |y, row| {
        island_row(size, y, row, coast_h, coast_v, min)
    });
}

/// sinks row `y` towards `min` inside the vertical band, then each cell inside the horizontal band
fn island_row(
    size: (usize, usize),
    y: usize,
    row: &mut [f32],
    coast_h: f32,
    coast_v: f32,
    min: f32,
) {
    let d = y.min(size.1 - 1 - y);
    if d < coast_v as usize {
        let c = d as f32 / coast_v;
        for h in row.iter_mut() {
            *h = (*h - min) * c + min;
        }
    }
    for (x, h) in row.iter_mut().enumerate() {
        let d = x.min(size.0 - 1 - x);
        if d < coast_h as usize {
            let c = d as f32 / coast_h;
            *h = (*h - min) * c + min;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn island_lowers_borders_only() {
        let size = (16, 16);
        let mut h = vec![1.0; 256];
        h[3 + 5 * 16] = 0.0;
        let conf = IslandConf { coast_range: 25.0 };
        gen_island(size, &mut h, &conf, &mut Progress::headless());
        assert_eq!(h[8], 0.0, "top edge cell (8, 0)");
        assert_eq!(h[8 * 16], 0.0, "left edge cell (0, 8)");
        assert_eq!(h[8 + 8 * 16], 1.0, "centre cell (8, 8)");
    }

    #[test]
    fn island_is_mirror_symmetric() {
        let mut h: Vec<f32> = (0..256).map(|i| (i / 16) as f32).collect();
        let conf = IslandConf::default();
        gen_island((16, 16), &mut h, &conf, &mut Progress::headless());
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(h[x + y * 16], h[(15 - x) + y * 16], "cell ({x}, {y})");
            }
        }
    }
}
