use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use super::{bilinear, Progress};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct MidPointConf {
    pub roughness: f32,
    #[serde(default = "default_persistence")]
    pub persistence: f32,
}

fn default_persistence() -> f32 {
    0.5
}

impl Default for MidPointConf {
    fn default() -> Self {
        Self {
            roughness: 0.7,
            persistence: default_persistence(),
        }
    }
}

pub fn render_mid_point(ui: &mut egui::Ui, conf: &mut MidPointConf) {
    ui.horizontal(|ui| {
        ui.label("roughness")
            .on_hover_text("How jagged the terrain is: lower = rolling, higher = rocky");
        ui.add(
            egui::DragValue::new(&mut conf.roughness)
                .speed(0.01)
                .range(0.01..=1.0),
        );
        ui.label("persistence").on_hover_text(
            "How much the small details keep of that roughness: lower = smooth, higher = busy",
        );
        ui.add(
            egui::DragValue::new(&mut conf.persistence)
                .speed(0.01)
                .range(0.1..=1.0),
        );
    });
}

/// Diamond-square on an internal `(2^k + 1)²` lattice covering the map, resampled into `hmap`.
/// Each octave draws its displacements from its own stream (`sub_seed`) in an order that does
/// not depend on `k`, so a larger map is the smaller one plus finer octaves.
pub fn gen_mid_point(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &MidPointConf,
    progress: &mut Progress,
) {
    let k = lattice_levels(size);
    let span = 1usize << k;
    let n = span + 1;
    let mut lattice = vec![0.0f32; n * n];
    let mut rng = StdRng::seed_from_u64(seed);
    lattice[0] = rng.random_range(0.0..1.0);
    lattice[span] = rng.random_range(0.0..1.0);
    lattice[span * n] = rng.random_range(0.0..1.0);
    lattice[span + span * n] = rng.random_range(0.0..1.0);
    let mut done = 4;
    for level in 1..=k {
        let amp = conf.roughness * conf.persistence.powi(level as i32 - 1);
        let mut rng = StdRng::seed_from_u64(sub_seed(seed, level));
        if !square_pass(
            &mut lattice,
            n,
            level,
            k,
            amp,
            &mut rng,
            &mut done,
            progress,
        ) {
            return;
        }
        if !diamond_pass(
            &mut lattice,
            n,
            level,
            k,
            amp,
            &mut rng,
            &mut done,
            progress,
        ) {
            return;
        }
    }
    let scale_x = span as f32 / size.0 as f32;
    let scale_y = span as f32 / size.1 as f32;
    for y in 0..size.1 {
        for x in 0..size.0 {
            hmap[x + y * size.0] =
                bilinear(&lattice, x as f32 * scale_x, y as f32 * scale_y, (n, n));
        }
    }
}

/// Smallest `k` with `2^k >= max(size)`; `0` for a 1×1 map.
fn lattice_levels(size: (usize, usize)) -> u32 {
    let largest = size.0.max(size.1);
    let mut k = 0;
    while (1usize << k) < largest {
        k += 1;
    }
    k
}

/// The RNG seed of one octave, a fixed formula over `seed` only.
fn sub_seed(seed: u64, level: u32) -> u64 {
    seed ^ ((level as u64) << 32)
}

/// Square pass of `level`: the centre of every `2s`-cell. Returns `false` once cancelled.
#[allow(clippy::too_many_arguments)]
fn square_pass(
    lattice: &mut [f32],
    n: usize,
    level: u32,
    k: u32,
    amp: f32,
    rng: &mut StdRng,
    done: &mut usize,
    progress: &mut Progress,
) -> bool {
    let s = 1usize << (k - level);
    let cells = 1usize << (level - 1);
    let total = (n * n) as f32;
    for j in 0..cells {
        for i in 0..cells {
            square_step(lattice, n, (2 * i + 1) * s, (2 * j + 1) * s, s, amp, rng);
            *done += 1;
        }
        if !progress.report(*done as f32 / total) {
            return false;
        }
    }
    true
}

/// Diamond pass of `level`: the edge midpoints between the points set so far. Returns `false`
/// once cancelled.
#[allow(clippy::too_many_arguments)]
fn diamond_pass(
    lattice: &mut [f32],
    n: usize,
    level: u32,
    k: u32,
    amp: f32,
    rng: &mut StdRng,
    done: &mut usize,
    progress: &mut Progress,
) -> bool {
    let s = 1usize << (k - level);
    let points = 1usize << level;
    let total = (n * n) as f32;
    for jj in 0..=points {
        for ii in 0..=points {
            if (ii + jj) % 2 == 1 {
                diamond_step(lattice, n, ii * s, jj * s, s, amp, rng);
                *done += 1;
            }
        }
        if !progress.report(*done as f32 / total) {
            return false;
        }
    }
    true
}

/// Mean of the four diagonal neighbours at distance `reach` (always inside the lattice) plus a
/// random displacement in `-amp..amp`.
fn square_step(
    lattice: &mut [f32],
    n: usize,
    x: usize,
    y: usize,
    reach: usize,
    amp: f32,
    rng: &mut StdRng,
) {
    let avg = (lattice[x - reach + (y - reach) * n]
        + lattice[x - reach + (y + reach) * n]
        + lattice[x + reach + (y - reach) * n]
        + lattice[x + reach + (y + reach) * n])
        / 4.0;
    lattice[x + y * n] = avg + rng.random_range(-amp..amp);
}

/// Mean of the axial neighbours at distance `reach` that lie inside the lattice (3 on an edge,
/// 4 inside) plus a random displacement in `-amp..amp`.
fn diamond_step(
    lattice: &mut [f32],
    n: usize,
    x: usize,
    y: usize,
    reach: usize,
    amp: f32,
    rng: &mut StdRng,
) {
    let mut count = 0;
    let mut avg = 0.0;
    if x >= reach {
        avg += lattice[x - reach + y * n];
        count += 1;
    }
    if x + reach < n {
        avg += lattice[x + reach + y * n];
        count += 1;
    }
    if y >= reach {
        avg += lattice[x + (y - reach) * n];
        count += 1;
    }
    if y + reach < n {
        avg += lattice[x + (y + reach) * n];
        count += 1;
    }
    avg /= count as f32;
    lattice[x + y * n] = avg + rng.random_range(-amp..amp);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seed: u64, size: (usize, usize), conf: &MidPointConf) -> Vec<f32> {
        let mut h = vec![-1e9; size.0 * size.1];
        gen_mid_point(seed, size, &mut h, conf, &mut Progress::headless());
        h
    }

    #[test]
    fn every_cell_is_written() {
        let conf = MidPointConf::default();
        for size in [(64, 64), (100, 100), (16, 32), (32, 16), (1, 1), (2, 3)] {
            let h = run(1, size, &conf);
            let unwritten = h.iter().position(|&v| v == -1e9);
            assert_eq!(
                unwritten, None,
                "size {size:?}: cell {unwritten:?} never written"
            );
            let nan = h.iter().position(|v| v.is_nan());
            assert_eq!(nan, None, "size {size:?}: cell {nan:?} is NaN");
        }
    }

    #[test]
    fn same_seed_is_identical() {
        let conf = MidPointConf::default();
        let a = run(5, (32, 32), &conf);
        let b = run(5, (32, 32), &conf);
        let c = run(6, (32, 32), &conf);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn larger_map_refines_smaller() {
        let conf = MidPointConf {
            roughness: 0.7,
            ..Default::default()
        };
        let h16 = run(7, (16, 16), &conf);
        let h32 = run(7, (32, 32), &conf);
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    h32[2 * x + 2 * y * 32],
                    h16[x + y * 16],
                    "cell ({x}, {y}) differs between the 16 and 32 maps"
                );
            }
        }
    }

    #[test]
    fn persistence_one_keeps_amplitude() {
        let range = |persistence: f32| {
            let conf = MidPointConf {
                roughness: 0.7,
                persistence,
            };
            let h = run(3, (32, 32), &conf);
            let (min, max) = super::super::get_min_max(&h);
            max - min
        };
        assert!(range(1.0) > range(0.1));
    }

    #[test]
    fn conf_without_persistence_loads_with_default() {
        let conf: MidPointConf = ron::from_str("(roughness: 0.7)").unwrap();
        assert_eq!(conf.roughness, 0.7);
        assert_eq!(conf.persistence, 0.5);
    }
}
