use std::f32::consts::SQRT_2;

use serde::{Deserialize, Serialize};

use super::{add_upsampled, downsample, work_size, Progress, DIRX, DIRY};

// talus erosion after Olsen 2004 : a cell steeper than the talus threshold sheds a share of its
// excess to its lower neighbours, mass conserving
const DEFAULT_TALUS: f32 = 0.5;
const DEFAULT_STRENGTH: f32 = 0.5;
const DEFAULT_ITERATIONS: u32 = 50;
const DEFAULT_WATER_LEVEL: f32 = 0.0;
const DEFAULT_WORK_RES: u32 = 512;
/// the map side the parameters are expressed for
const REFERENCE_RES: f32 = 512.0;
/// per-cell height difference on a `REFERENCE_RES` map that `talus` 0.0 stands for : the steepest
/// slope the other generators produce on a normalized map, so nothing crumbles there
const TALUS_MAX: f32 = 0.03;
/// plain hover text of the talus parameter
const TALUS_HELP: &str =
    "How much the slopes crumble: 0 leaves the terrain untouched, 1 flattens everything";

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ThermalErosionConf {
    /// how much the slopes crumble, 0 (nothing) to 1 (every slope); see `ThermalParams::new`
    pub talus: f32,
    /// share of the excess moved per pass
    pub strength: f32,
    /// passes on a `REFERENCE_RES` map
    pub iterations: u32,
    /// cells below this height do not crumble
    pub water_level: f32,
    /// longest side of the grid the passes run on
    pub work_res: u32,
}

impl Default for ThermalErosionConf {
    fn default() -> Self {
        Self {
            talus: DEFAULT_TALUS,
            strength: DEFAULT_STRENGTH,
            iterations: DEFAULT_ITERATIONS,
            water_level: DEFAULT_WATER_LEVEL,
            work_res: DEFAULT_WORK_RES,
        }
    }
}

pub fn render_thermal_erosion(ui: &mut egui::Ui, conf: &mut ThermalErosionConf) {
    ui.horizontal(|ui| {
        ui.label("talus").on_hover_text(TALUS_HELP);
        ui.add(
            egui::DragValue::new(&mut conf.talus)
                .speed(0.01)
                .range(0.0..=1.0),
        )
        .on_hover_text(TALUS_HELP);
        ui.label("strength")
            .on_hover_text("How much of the loose material slides down each pass");
        ui.add(
            egui::DragValue::new(&mut conf.strength)
                .speed(0.01)
                .range(0.0..=0.5),
        );
        ui.label("iterations")
            .on_hover_text("How many passes to run: more = smoother, slower");
        ui.add(
            egui::DragValue::new(&mut conf.iterations)
                .speed(1)
                .range(1..=500),
        );
    });
    render_thermal_row(ui, conf);
}

fn render_thermal_row(ui: &mut egui::Ui, conf: &mut ThermalErosionConf) {
    ui.horizontal(|ui| {
        ui.label("water level").on_hover_text(
            "Land below this height does not crumble, but still catches what slides down",
        );
        ui.add(
            egui::DragValue::new(&mut conf.water_level)
                .speed(0.01)
                .range(-10.0..=10.0),
        );
        ui.label("resolution")
            .on_hover_text("Level of detail the erosion works at: higher = finer, much slower");
        egui::ComboBox::from_id_salt("thermal_work_res")
            .selected_text(format!("{}", conf.work_res))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut conf.work_res, 256, "256");
                ui.selectable_value(&mut conf.work_res, 512, "512");
                ui.selectable_value(&mut conf.work_res, 1024, "1024");
                ui.selectable_value(&mut conf.work_res, 2048, "2048");
            });
    });
}

/// the conf converted to the working grid : `scale` is working cells per reference cell.
/// Shared with FluvialErosion, which runs `slide_pass` between two incisions
pub(crate) struct ThermalParams {
    /// per-cell height difference above which an axis neighbour receives material
    pub(crate) threshold: f32,
    /// the same for a diagonal neighbour, √2 further away
    pub(crate) diag_threshold: f32,
    pub(crate) strength: f32,
    pub(crate) passes: usize,
    pub(crate) water_level: f32,
}

impl ThermalParams {
    /// `talus` maps to the threshold through `TALUS_MAX * (1 - talus)²` : the square spreads the
    /// visible part of the effect (thresholds below a stock map's median slope) over most of the
    /// 0..1 range instead of its last tenth
    pub(crate) fn new(conf: &ThermalErosionConf, scale: f32) -> Self {
        let threshold = TALUS_MAX * (1.0 - conf.talus).powi(2) / scale;
        Self {
            threshold,
            diag_threshold: threshold * SQRT_2,
            strength: conf.strength,
            passes: ((conf.iterations as f32 * scale).ceil() as usize).max(1),
            water_level: conf.water_level,
        }
    }
}

/// the working grid the passes run on and the conf converted to it; shared with the GPU twin
pub(crate) fn thermal_plan(
    size: (usize, usize),
    conf: &ThermalErosionConf,
) -> ((usize, usize), ThermalParams) {
    let work = work_size(size, conf.work_res as usize);
    let scale = work.0.max(work.1) as f32 / REFERENCE_RES;
    (work, ThermalParams::new(conf, scale))
}

pub fn gen_thermal_erosion(
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &ThermalErosionConf,
    progress: &mut Progress,
) {
    let (work, params) = thermal_plan(size, conf);
    if work == size {
        slide_passes(size, hmap, &params, progress);
        return;
    }
    // erode a reduced copy, then add the height delta back onto the full map
    let small = downsample(hmap, size, work);
    let mut eroded = small.clone();
    if !slide_passes(work, &mut eroded, &params, progress) {
        // cancelled : leave the map untouched
        return;
    }
    for (delta, initial) in eroded.iter_mut().zip(small.iter()) {
        *delta -= initial;
    }
    add_upsampled(hmap, size, &eroded, work);
}

/// runs every pass on `hmap`; returns false when the step was cancelled, `hmap` then untouched
/// by the cancelled pass
fn slide_passes(
    size: (usize, usize),
    hmap: &mut [f32],
    params: &ThermalParams,
    progress: &mut Progress,
) -> bool {
    let mut out = hmap.to_vec();
    let passes = params.passes as f32;
    for pass in 0..params.passes {
        out.copy_from_slice(hmap);
        let from = pass as f32 / passes;
        let to = (pass + 1) as f32 / passes;
        if !slide_pass(size, hmap, &mut out, params, from, to, progress) {
            return false;
        }
        hmap.copy_from_slice(&out);
    }
    true
}

/// one pass reading `hmap` and scattering into `out`, which starts as a copy of `hmap`;
/// reports progress from `progress_from` to `progress_to` across the rows; false when the step
/// was cancelled
pub(super) fn slide_pass(
    size: (usize, usize),
    hmap: &[f32],
    out: &mut [f32],
    params: &ThermalParams,
    progress_from: f32,
    progress_to: f32,
    progress: &mut Progress,
) -> bool {
    let mut excess = [0.0f32; 9];
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let h = hmap[x + yoff];
            if h < params.water_level {
                continue;
            }
            let mut d_total = 0.0;
            let mut d_max = 0.0f32;
            for i in 1..9 {
                excess[i] = 0.0;
                let ix = (x as i32 + DIRX[i]) as usize;
                let iy = (y as i32 + DIRY[i]) as usize;
                if ix >= size.0 || iy >= size.1 {
                    continue;
                }
                let threshold = if DIRX[i] != 0 && DIRY[i] != 0 {
                    params.diag_threshold
                } else {
                    params.threshold
                };
                let e = (h - hmap[ix + iy * size.0]) - threshold;
                if e > 0.0 {
                    excess[i] = e;
                    d_total += e;
                    d_max = d_max.max(e);
                }
            }
            if d_total <= 0.0 {
                continue;
            }
            let moved = params.strength * d_max;
            out[x + yoff] -= moved;
            for i in 1..9 {
                if excess[i] > 0.0 {
                    let ix = (x as i32 + DIRX[i]) as usize;
                    let iy = (y as i32 + DIRY[i]) as usize;
                    out[ix + iy * size.0] += moved * excess[i] / d_total;
                }
            }
        }
        let p = progress_from + (progress_to - progress_from) * y as f32 / size.1 as f32;
        if !progress.report(p) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::bilinear;
    use super::*;

    /// integer pyramid : gradients are rarely zero and every value is exact in f32
    fn pyramid(size: (usize, usize)) -> Vec<f32> {
        let mut hmap = vec![0.0; size.0 * size.1];
        for y in 0..size.1 {
            for x in 0..size.0 {
                hmap[x + y * size.0] = (32 - (x as i32 - 8).abs() - (y as i32 - 8).abs()) as f32
                    + ((x * 7 + y * 13) % 5) as f32;
            }
        }
        hmap
    }

    /// 16×16 map, left half at 0.0, right half at 1.0
    fn cliff() -> Vec<f32> {
        (0..256)
            .map(|i| if i % 16 < 8 { 0.0 } else { 1.0 })
            .collect()
    }

    fn conf16() -> ThermalErosionConf {
        ThermalErosionConf {
            work_res: 16,
            ..Default::default()
        }
    }

    fn erode(size: (usize, usize), hmap: &[f32], conf: &ThermalErosionConf) -> Vec<f32> {
        let mut out = hmap.to_vec();
        gen_thermal_erosion(size, &mut out, conf, &mut Progress::headless());
        out
    }

    fn blow_up(src: &[f32], size: (usize, usize), factor: usize) -> Vec<f32> {
        let big = (size.0 * factor, size.1 * factor);
        let mut out = vec![0.0; big.0 * big.1];
        for y in 0..big.1 {
            for x in 0..big.0 {
                out[x + y * big.0] = src[x / factor + (y / factor) * size.0];
            }
        }
        out
    }

    /// largest height difference between two 8-connected neighbours
    fn max_neighbour_diff(size: (usize, usize), hmap: &[f32]) -> f32 {
        let mut max = 0.0f32;
        for y in 0..size.1 {
            for x in 0..size.0 {
                for i in 1..9 {
                    let ix = (x as i32 + DIRX[i]) as usize;
                    let iy = (y as i32 + DIRY[i]) as usize;
                    if ix < size.0 && iy < size.1 {
                        max = max.max((hmap[x + y * size.0] - hmap[ix + iy * size.0]).abs());
                    }
                }
            }
        }
        max
    }

    #[test]
    fn flat_map_stays_flat() {
        let flat = vec![0.5; 256];
        assert_eq!(erode((16, 16), &flat, &conf16()), flat);
    }

    #[test]
    fn gentle_ramp_below_talus_is_untouched() {
        let ramp: Vec<f32> = (0..256).map(|i| (i % 16) as f32 * 0.001).collect();
        assert_eq!(erode((16, 16), &ramp, &conf16()), ramp);
    }

    #[test]
    fn cliff_crumbles_and_mass_is_conserved() {
        let input = cliff();
        let out = erode((16, 16), &input, &conf16());
        assert!(
            max_neighbour_diff((16, 16), &out) < max_neighbour_diff((16, 16), &input),
            "the cliff did not crumble"
        );
        let sum_in: f32 = input.iter().sum();
        let sum_out: f32 = out.iter().sum();
        assert!(
            (sum_in - sum_out).abs() < 1e-3,
            "mass changed : {sum_in} -> {sum_out}"
        );
        assert!(out.iter().all(|h| (0.0..=1.0).contains(h)));
    }

    #[test]
    fn same_input_is_identical() {
        let input = pyramid((16, 16));
        assert_eq!(
            erode((16, 16), &input, &conf16()),
            erode((16, 16), &input, &conf16())
        );
    }

    #[test]
    fn params_scale_with_the_working_grid() {
        let conf = ThermalErosionConf::default();
        let full = ThermalParams::new(&conf, 1.0);
        assert_eq!(full.passes, conf.iterations as usize);
        let reference = TALUS_MAX * (1.0 - conf.talus).powi(2);
        assert_eq!(full.threshold, reference);
        let quarter = ThermalParams::new(&conf, 0.25);
        assert_eq!(
            quarter.passes,
            (conf.iterations as f32 / 4.0).ceil() as usize
        );
        assert_eq!(quarter.threshold, 4.0 * reference);
        assert_eq!(quarter.diag_threshold, quarter.threshold * SQRT_2);
        let off = ThermalErosionConf {
            talus: 0.0,
            ..Default::default()
        };
        assert_eq!(ThermalParams::new(&off, 1.0).threshold, TALUS_MAX);
        let full_impact = ThermalErosionConf {
            talus: 1.0,
            ..Default::default()
        };
        assert_eq!(ThermalParams::new(&full_impact, 1.0).threshold, 0.0);
    }

    #[test]
    fn working_grid_result_is_the_upsampled_in_place_delta() {
        let conf = conf16();
        let input16 = pyramid((16, 16));
        let out16 = erode((16, 16), &input16, &conf);
        let delta16: Vec<f32> = out16
            .iter()
            .zip(input16.iter())
            .map(|(o, i)| o - i)
            .collect();

        let input64 = blow_up(&input16, (16, 16), 4);
        let out64 = erode((64, 64), &input64, &conf);

        for y in 0..64 {
            for x in 0..64 {
                let u = ((x as f32 + 0.5) * 16.0 / 64.0 - 0.5).max(0.0);
                let v = ((y as f32 + 0.5) * 16.0 / 64.0 - 0.5).max(0.0);
                let expected = input64[x + y * 64] + bilinear(&delta16, u, v, (16, 16));
                let got = out64[x + y * 64];
                assert!(
                    (got - expected).abs() < 1e-4,
                    "({x}, {y}) : got {got}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn non_square_maps_run() {
        let conf = ThermalErosionConf::default();
        for size in [(16, 32), (32, 16)] {
            let input = pyramid(size);
            assert_ne!(erode(size, &input, &conf), input, "{size:?} unchanged");
        }
    }

    #[test]
    fn cancelled_run_leaves_map_untouched() {
        let input = cliff();
        assert_ne!(erode((16, 16), &input, &conf16()), input);
        let (tx, _) = std::sync::mpsc::channel();
        let mut cancelled = input.clone();
        gen_thermal_erosion(
            (16, 16),
            &mut cancelled,
            &conf16(),
            &mut Progress::preview(tx, 1.0, || true),
        );
        assert_eq!(cancelled, input);
    }
}
