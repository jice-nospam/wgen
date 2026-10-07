use serde::{Deserialize, Serialize};

use super::noise_field::{virtual_coords, Warp};
use super::{bilinear, par_rows, Progress};

/// the side of the lattice the logged shift and fold are measured on
const STATS_LATTICE: usize = 256;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BendConf {
    /// the largest push along each axis, in % of the map side
    pub amount: f32,
    /// Fbm's zoom convention: the widest bends repeat `1.28 × zoom` times across the map
    pub zoom: f32,
    /// the number of finer layers
    pub octaves: u32,
    /// the share of strength each finer layer keeps
    pub roughness: f32,
}

impl Default for BendConf {
    fn default() -> Self {
        Self {
            amount: 2.0,
            zoom: 4.0,
            octaves: 6,
            roughness: 0.4,
        }
    }
}

pub fn render_bend(ui: &mut egui::Ui, conf: &mut BendConf) {
    ui.horizontal(|ui| {
        ui.label("amount")
            .on_hover_text("The largest sideways push, in % of the map side");
        ui.add(
            egui::DragValue::new(&mut conf.amount)
                .speed(0.05)
                .range(0.0..=10.0),
        );
        ui.label("zoom")
            .on_hover_text("Higher packs more, smaller bends across the map");
        ui.add(
            egui::DragValue::new(&mut conf.zoom)
                .speed(0.1)
                .range(0.5..=20.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("octaves")
            .on_hover_text("Layers of ever finer wiggles: more = richer but slower");
        ui.add(egui::DragValue::new(&mut conf.octaves).range(1..=10));
        ui.label("roughness")
            .on_hover_text("How strong the finer wiggles are next to the wide bends");
        ui.add(
            egui::DragValue::new(&mut conf.roughness)
                .speed(0.01)
                .range(0.0..=1.0),
        );
    });
}

/// the bend's warp and its per-axis bound, both in virtual pixels
fn bend_warp(seed: u64, conf: &BendConf) -> (Warp, f64) {
    let amount_px = conf.amount / 100.0 * 512.0;
    let octaves = (conf.octaves as usize).clamp(1, 10);
    let warp = Warp::new(
        seed,
        0,
        1,
        octaves,
        conf.zoom,
        amount_px,
        conf.roughness as f64,
    );
    (warp, amount_px as f64)
}

/// the displacement of virtual point `p`, each axis clamped to `±bound`
fn displacement(warp: &Warp, bound: f64, p: [f64; 2]) -> [f64; 2] {
    let q = warp.apply(p);
    [
        (q[0] - p[0]).clamp(-bound, bound),
        (q[1] - p[1]).clamp(-bound, bound),
    ]
}

/// over a lattice of the virtual plane: the largest shift in % of the side, and the smallest
/// determinant of the bent mapping's Jacobian (1 is no squeeze, ≤ 0 folds the map over itself)
pub fn bend_stats(seed: u64, conf: &BendConf) -> (f32, f32) {
    let (warp, bound) = bend_warp(seed, conf);
    let step = 512.0 / STATS_LATTICE as f64;
    let at = |i: usize, j: usize| displacement(&warp, bound, [i as f64 * step, j as f64 * step]);
    let mut shift: f64 = 0.0;
    let mut fold = f64::MAX;
    for j in 0..STATS_LATTICE {
        for i in 0..STATS_LATTICE {
            let d = at(i, j);
            shift = shift.max((d[0] * d[0] + d[1] * d[1]).sqrt());
            let (dx, dy) = (at(i + 1, j), at(i, j + 1));
            let a = 1.0 + (dx[0] - d[0]) / step;
            let b = (dy[0] - d[0]) / step;
            let c = (dx[1] - d[1]) / step;
            let e = 1.0 + (dy[1] - d[1]) / step;
            fold = fold.min(a * e - b * c);
        }
    }
    ((shift / 512.0 * 100.0) as f32, fold as f32)
}

/// moves every cell sideways: its height is read from the copy of the map at the cell's
/// position displaced by two fractal noise fields
pub fn gen_bend(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &BendConf,
    progress: &mut Progress,
) {
    if conf.amount <= 0.0 {
        return;
    }
    let src = hmap.to_vec();
    let (warp, bound) = bend_warp(seed, conf);
    let (sx, sy) = (size.0 as f64 / 512.0, size.1 as f64 / 512.0);
    let (maxx, maxy) = ((size.0 - 1) as f64, (size.1 - 1) as f64);
    let done = par_rows(size.0, hmap, progress, (0.0, 1.0), |y, row| {
        for (x, h) in row.iter_mut().enumerate() {
            let (u, v) = virtual_coords(size, x, y);
            let p = [u as f64, v as f64];
            let d = displacement(&warp, bound, p);
            let cx = ((p[0] + d[0]) * sx).clamp(0.0, maxx);
            let cy = ((p[1] + d[1]) * sy).clamp(0.0, maxy);
            *h = bilinear(&src, cx as f32, cy as f32, size);
        }
    });
    if done {
        let (shift, fold) = bend_stats(seed, conf);
        crate::log(&format!("bend=>shift max {shift:.2}% fold {fold:.3}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bend(map: &[f32], size: (usize, usize), conf: &BendConf) -> Vec<f32> {
        let mut h = map.to_vec();
        gen_bend(7, size, &mut h, conf, &mut Progress::headless());
        h
    }

    #[test]
    fn amount_zero_is_identity() {
        let map: Vec<f32> = (0..64 * 64).map(|i| (i % 13) as f32).collect();
        let conf = BendConf {
            amount: 0.0,
            ..Default::default()
        };
        assert_eq!(bend(&map, (64, 64), &conf), map);
    }

    #[test]
    fn constant_stays_constant_and_runs_agree() {
        let map = vec![0.3; 64 * 64];
        let bent = bend(&map, (64, 64), &BendConf::default());
        assert!(bent.iter().all(|&h| (h - 0.3).abs() < 1e-6));
        let ramp: Vec<f32> = (0..64 * 64).map(|i| (i % 64) as f32).collect();
        let conf = BendConf::default();
        assert_eq!(bend(&ramp, (64, 64), &conf), bend(&ramp, (64, 64), &conf));
    }

    #[test]
    fn step_edge_wanders_within_the_bound() {
        let n = 256;
        let map: Vec<f32> = (0..n * n)
            .map(|i| if i % n < n / 2 { 0.0 } else { 1.0 })
            .collect();
        let conf = BendConf {
            amount: 2.0,
            ..Default::default()
        };
        let bent = bend(&map, (n, n), &conf);
        let mut moved = false;
        for y in 0..n {
            let row = &bent[y * n..(y + 1) * n];
            let edge = row.iter().position(|&h| h >= 0.5).unwrap();
            let off = (edge as f32 - (n / 2) as f32).abs();
            assert!(off <= 0.02 * n as f32 + 1.0, "row {y}: edge at {edge}");
            moved |= off >= 1.0;
        }
        assert!(moved, "the edge never moved");
    }

    #[test]
    fn stats_bounded_and_unfolded() {
        for amount in [1.0, 2.0, 3.0] {
            let conf = BendConf {
                amount,
                ..Default::default()
            };
            let (shift, fold) = bend_stats(7, &conf);
            assert!(shift <= 2f32.sqrt() * amount + 1e-4, "shift {shift}");
            assert!(shift > 0.0);
            if amount == 2.0 {
                assert!(fold > 0.0, "fold {fold}");
            }
        }
    }
}
