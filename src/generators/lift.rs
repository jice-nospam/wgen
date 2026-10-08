use serde::{Deserialize, Serialize};

use super::{par_rows, Progress};
use crate::height_range::drag_meters;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct LiftConf {
    pub height: f32,
}

impl Default for LiftConf {
    fn default() -> Self {
        Self { height: 0.1 }
    }
}

pub fn render_lift(ui: &mut egui::Ui, conf: &mut LiftConf) {
    ui.horizontal(|ui| {
        ui.label("height").on_hover_text(
            "Raises or lowers the whole map by this height; with a mask, the mask is the shape",
        );
        drag_meters(ui, &mut conf.height, 1.0);
    });
}

/// adds `conf.height` to every cell
pub fn gen_lift(size: (usize, usize), hmap: &mut [f32], conf: &LiftConf, progress: &mut Progress) {
    let height = conf.height;
    par_rows(size.0, hmap, progress, (0.0, 1.0), |_, row| {
        for h in row.iter_mut() {
            *h += height;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lift_adds_exactly_the_height() {
        let input: Vec<f32> = (0..16 * 16).map(|i| (i % 7) as f32 * 0.1).collect();
        let mut h = input.clone();
        gen_lift(
            (16, 16),
            &mut h,
            &LiftConf { height: 0.25 },
            &mut Progress::headless(),
        );
        for (a, b) in h.iter().zip(&input) {
            assert_eq!(*a, b + 0.25);
        }
    }
}
