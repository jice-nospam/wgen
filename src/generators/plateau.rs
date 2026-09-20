use noise::NoiseFn;
use serde::{Deserialize, Serialize};

use super::noise_field::{fbm_stream, noise_coef, virtual_coords};
use super::{get_min_max, par_rows, Progress};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct PlateauConf {
    pub levels: u32,
    pub flat: f32,
    pub rounding: f32,
    pub jitter: f32,
    pub jitter_zoom: f32,
}

impl Default for PlateauConf {
    fn default() -> Self {
        Self {
            levels: 6,
            flat: 0.6,
            rounding: 0.3,
            jitter: 0.3,
            jitter_zoom: 4.0,
        }
    }
}

pub fn render_plateau(ui: &mut egui::Ui, conf: &mut PlateauConf) {
    ui.horizontal(|ui| {
        ui.label("levels")
            .on_hover_text("How many flat steps between the lowest and the highest point");
        ui.add(
            egui::DragValue::new(&mut conf.levels)
                .speed(1)
                .range(2..=64),
        );
        ui.label("flat").on_hover_text(
            "Share of each step that is flat: 0 = no change, 0.95 = cliffs between flat treads",
        );
        ui.add(
            egui::DragValue::new(&mut conf.flat)
                .speed(0.01)
                .range(0.0..=0.95),
        );
        ui.label("rounding")
            .on_hover_text("Softens the ledges: 0 = sharp edges, 1 = rolled");
        ui.add(
            egui::DragValue::new(&mut conf.rounding)
                .speed(0.01)
                .range(0.0..=1.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("jitter").on_hover_text(
            "Makes the step edges wander instead of following the contours; in steps",
        );
        ui.add(
            egui::DragValue::new(&mut conf.jitter)
                .speed(0.01)
                .range(0.0..=1.0),
        );
        ui.label("jitter zoom")
            .on_hover_text("Size of the wander: higher = finer wiggles");
        ui.add(
            egui::DragValue::new(&mut conf.jitter_zoom)
                .speed(0.1)
                .range(0.1..=100.0),
        );
    });
}

/// the fixed octave count of the jitter noise, shared with the GPU twin
pub(crate) const JITTER_OCTAVES: usize = 4;

/// the tread/riser shape inside one level: `f` the position in `0..1`, half a tread at each end
/// and the riser in the middle. `terrace_profile(0, ..) == 0` and `terrace_profile(1, ..) == 1`,
/// so the profile is continuous across level boundaries; at `flat = 0, rounding = 0` it is the
/// identity. `flat` must be below 1 (the generator clamps it before calling).
pub(super) fn terrace_profile(f: f32, flat: f32, rounding: f32) -> f32 {
    let g = ((f - 0.5 * flat) / (1.0 - flat)).clamp(0.0, 1.0);
    let s = g * g * (3.0 - 2.0 * g);
    g + (s - g) * rounding
}

/// cuts the terrain into `levels` flat treads between the map's current minimum and maximum,
/// each riser shaped by `terrace_profile`; `jitter` displaces the level boundaries by a
/// low-frequency noise so they wander instead of tracing the input's contours. Leaves `hmap`
/// unchanged when it has no range (a flat map, or a single value).
pub fn gen_plateau(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &PlateauConf,
    progress: &mut Progress,
) {
    let (min, max) = get_min_max(hmap);
    let range = max - min;
    if range <= 0.0 {
        return;
    }
    let flat = conf.flat.min(0.95);
    let levels = conf.levels as f32;
    let jitter_noise = fbm_stream(seed, 0, JITTER_OCTAVES);
    let jitter_coef = noise_coef(conf.jitter_zoom) as f64;
    par_rows(size.0, hmap, progress, (0.0, 1.0), |y, row| {
        for (x, h) in row.iter_mut().enumerate() {
            let (u, v) = virtual_coords(size, x, y);
            let j = conf.jitter
                * jitter_noise.get([u as f64 * jitter_coef, v as f64 * jitter_coef]) as f32;
            let frac = (*h - min) / range;
            let s = frac * levels + j;
            let k = s.floor();
            let f = s - k;
            *h = min + (k + terrace_profile(f, flat, conf.rounding)) / levels * range;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::calib;
    use super::*;

    #[test]
    fn terrace_profile_is_continuous_and_monotone() {
        for (flat, rounding) in [(0.0, 0.0), (0.6, 0.3), (0.95, 1.0)] {
            assert!(terrace_profile(0.0, flat, rounding).abs() < 1e-6);
            assert!((terrace_profile(1.0, flat, rounding) - 1.0).abs() < 1e-6);
            let mut prev = terrace_profile(0.0, flat, rounding);
            for i in 1..=100 {
                let f = i as f32 / 100.0;
                let v = terrace_profile(f, flat, rounding);
                assert!(v >= prev - 1e-6, "not monotone at f={f}: {v} < {prev}");
                prev = v;
            }
        }
        for i in 0..=100 {
            let f = i as f32 / 100.0;
            assert!(
                (terrace_profile(f, 0.0, 0.0) - f).abs() < 1e-6,
                "identity at f={f}"
            );
        }
    }

    fn ramp16() -> Vec<f32> {
        let mut h = vec![0.0; 16 * 16];
        for y in 0..16 {
            for x in 0..16 {
                h[x + y * 16] = x as f32 / 15.0;
            }
        }
        h
    }

    #[test]
    fn plateau_quantises_a_ramp() {
        let mut h = ramp16();
        let conf = PlateauConf {
            levels: 4,
            flat: 0.95,
            rounding: 0.0,
            jitter: 0.0,
            jitter_zoom: 4.0,
        };
        gen_plateau(1, (16, 16), &mut h, &conf, &mut Progress::headless());
        let close = h
            .iter()
            .filter(|v| {
                let k = (*v * 4.0).round() / 4.0;
                (*v - k).abs() < 1e-6
            })
            .count();
        assert!(close >= 220, "only {close} cells landed on a k/4 tread");
        let (min, max) = get_min_max(&h);
        assert!(min.abs() < 1e-6, "min = {min}");
        assert!((max - 1.0).abs() < 1e-6, "max = {max}");
    }

    #[test]
    fn plateau_is_deterministic() {
        let base = calib::stock_map(9, (16, 16));
        let conf = PlateauConf::default();
        let mut a = base.clone();
        let mut b = base;
        gen_plateau(9, (16, 16), &mut a, &conf, &mut Progress::headless());
        gen_plateau(9, (16, 16), &mut b, &conf, &mut Progress::headless());
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn plateau_refines_with_resolution() {
        let small_size = (16, 16);
        let large_size = (32, 32);
        // capped at the small map's own maximum virtual x-coordinate, so a matching cell of
        // both maps holds the same value *and* both maps compute the same (min, max) — the
        // large map's odd columns (no counterpart in the small map) clamp to that same cap
        // rather than reaching further towards the plane's edge
        let cap = virtual_coords(small_size, small_size.0 - 1, 0).0;
        let ramp = |size: (usize, usize), x: usize, y: usize| virtual_coords(size, x, y).0.min(cap);
        let mut small = vec![0.0; small_size.0 * small_size.1];
        for y in 0..small_size.1 {
            for x in 0..small_size.0 {
                small[x + y * small_size.0] = ramp(small_size, x, y);
            }
        }
        let mut large = vec![0.0; large_size.0 * large_size.1];
        for y in 0..large_size.1 {
            for x in 0..large_size.0 {
                large[x + y * large_size.0] = ramp(large_size, x, y);
            }
        }
        let conf = PlateauConf {
            jitter: 0.0,
            ..PlateauConf::default()
        };
        gen_plateau(3, small_size, &mut small, &conf, &mut Progress::headless());
        gen_plateau(3, large_size, &mut large, &conf, &mut Progress::headless());
        for y in 0..16 {
            for x in 0..16 {
                let a = small[x + y * 16];
                let b = large[2 * x + 2 * y * 32];
                assert!((a - b).abs() < 1e-6, "cell ({x}, {y}): {a} vs {b}");
            }
        }
    }

    #[test]
    fn plateau_leaves_a_flat_map_alone() {
        let mut h = vec![0.5; 16 * 16];
        gen_plateau(
            1,
            (16, 16),
            &mut h,
            &PlateauConf::default(),
            &mut Progress::headless(),
        );
        assert!(h.iter().all(|v| *v == 0.5));
    }
}
