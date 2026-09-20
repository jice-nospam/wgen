use noise::{NoiseFn, RidgedMulti};
use serde::{Deserialize, Serialize};

use super::noise_field::{noise_coef, ridged_stream, virtual_coords, Warp};
use super::{par_rows, Progress};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct RidgedConf {
    pub zoom: f32,
    pub octaves: f32,
    pub scale: f32,
    /// how far the sampling plane is smeared by the fold noise, in % of the map side; 0 = no fold
    pub fold: f32,
    pub fold_zoom: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Default for RidgedConf {
    fn default() -> Self {
        Self {
            zoom: 3.0,
            octaves: 6.0,
            scale: 1.0,
            fold: 0.0,
            fold_zoom: 1.5,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

pub fn render_ridged(ui: &mut egui::Ui, conf: &mut RidgedConf) {
    ui.horizontal(|ui| {
        ui.label("zoom")
            .on_hover_text("Zoom of the noise: higher packs more, narrower ridges across the map");
        ui.add(
            egui::DragValue::new(&mut conf.zoom)
                .speed(0.1)
                .range(0.1..=100.0),
        );
        ui.label("octaves")
            .on_hover_text("Layers of ever finer crests: more = craggier but slower");
        ui.add(
            egui::DragValue::new(&mut conf.octaves)
                .speed(0.5)
                .range(1.0..=MAX_OCTAVES as f32),
        );
        ui.label("scale")
            .on_hover_text("Height of the highest crests");
        ui.add(
            egui::DragValue::new(&mut conf.scale)
                .speed(0.01)
                .range(0.01..=10.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("fold %").on_hover_text(
            "Bends the ranges: how far the noise is smeared, in % of the map; 0 = straight ridges",
        );
        ui.add(
            egui::DragValue::new(&mut conf.fold)
                .speed(0.1)
                .range(0.0..=50.0),
        );
        ui.label("fold zoom")
            .on_hover_text("Size of the bends: higher = tighter twists");
        ui.add(
            egui::DragValue::new(&mut conf.fold_zoom)
                .speed(0.1)
                .range(0.1..=100.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("offset x")
            .on_hover_text("Slides the noise to look at another part of it");
        ui.add(
            egui::DragValue::new(&mut conf.offset_x)
                .speed(0.1)
                .range(0.0..=200.0),
        );
        ui.label("y")
            .on_hover_text("Slides the noise to look at another part of it");
        ui.add(
            egui::DragValue::new(&mut conf.offset_y)
                .speed(0.1)
                .range(0.0..=200.0),
        );
    });
}

/// the crate's octave cap, shared with the GPU twin
pub const MAX_OCTAVES: usize = RidgedMulti::<noise::Perlin>::MAX_OCTAVES;

/// the fold's Fbm octave count and its two noise streams (the ridges are stream 0), shared
/// with the GPU twin
pub(crate) const FOLD_OCTAVES: usize = 3;
pub(crate) const FOLD_STREAM_A: u32 = 1;
pub(crate) const FOLD_STREAM_B: u32 = 2;

/// adds ridged multifractal relief: `0.5 · (r + 1) · scale` per cell with `r` the crate's
/// `RidgedMulti::get` in `[-1, 1]`, so the relief is never negative; with `fold > 0` the
/// sampling plane is warped by a low-frequency Fbm first
pub fn gen_ridged(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &RidgedConf,
    progress: &mut Progress,
) {
    let octaves = (conf.octaves as usize).clamp(1, MAX_OCTAVES);
    let ridged = ridged_stream(seed, 0, octaves);
    let warp = (conf.fold > 0.0).then(|| {
        Warp::new(
            seed,
            FOLD_STREAM_A,
            FOLD_STREAM_B,
            FOLD_OCTAVES,
            conf.fold_zoom,
            conf.fold / 100.0 * 512.0,
        )
    });
    let coef = noise_coef(conf.zoom) as f64;
    par_rows(size.0, hmap, progress, (0.0, 1.0), |y, row| {
        for (x, h) in row.iter_mut().enumerate() {
            let (u, v) = virtual_coords(size, x, y);
            let p = [(u + conf.offset_x) as f64, (v + conf.offset_y) as f64];
            let q = match &warp {
                Some(warp) => warp.apply(p),
                None => p,
            };
            let r = ridged.get([q[0] * coef, q[1] * coef]) as f32;
            *h += 0.5 * (r + 1.0) * conf.scale;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seed: u64, size: (usize, usize), conf: &RidgedConf) -> Vec<f32> {
        let mut h = vec![0.25; size.0 * size.1];
        gen_ridged(seed, size, &mut h, conf, &mut Progress::headless());
        h
    }

    #[test]
    fn ridged_is_deterministic_and_never_digs() {
        let conf = RidgedConf::default();
        let a = run(3, (16, 16), &conf);
        let b = run(3, (16, 16), &conf);
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite()));
        assert!(
            a.iter().all(|v| *v >= 0.25),
            "a ridged step dug below the input"
        );
        assert!(a.iter().any(|v| *v > 0.25), "the step did nothing");
    }

    #[test]
    fn ridged_refines_with_resolution() {
        let conf = RidgedConf {
            fold: 10.0,
            ..Default::default()
        };
        let small = run(5, (16, 16), &conf);
        let large = run(5, (32, 32), &conf);
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    small[x + y * 16],
                    large[2 * x + 2 * y * 32],
                    "cell ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn ridged_handles_non_square_maps() {
        let conf = RidgedConf::default();
        for size in [(16, 32), (32, 16)] {
            let h = run(11, size, &conf);
            assert_eq!(h.len(), 16 * 32);
            assert!(
                h.iter().any(|v| *v != 0.25),
                "{size:?}: the step did nothing"
            );
        }
    }
}
