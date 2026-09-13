use eframe::egui;
use serde::{Deserialize, Serialize};

use super::thermal_erosion::{slide_pass, ThermalErosionConf, ThermalParams};
use super::{add_upsampled, downsample, receiver_distance, work_size, FlowNet, Progress};

// stream-power incision after Braun & Willett 2013 : every cell is lowered toward the cell it
// drains into, in proportion to the square root of its drainage area, with the implicit solve
// that is stable for any strength. The base level is the height `water_level` : the sea is
// fixed, nothing cuts below it, and the map border drains down to it. Between two incisions
// the slopes steeper than a talus crumble (ThermalErosion's pass), so valleys widen as they
// deepen
const DEFAULT_STRENGTH: f32 = 0.6;
const DEFAULT_TALUS: f32 = 0.2;
const DEFAULT_UPLIFT: f32 = 0.0;
const DEFAULT_ITERATIONS: u32 = 50;
const DEFAULT_WATER_LEVEL: f32 = 0.0;
const DEFAULT_WORK_RES: u32 = 512;
/// the map side the parameters are expressed for
const REFERENCE_RES: f32 = 512.0;
/// exponent of the normalized drainage area in the incision rate
const AREA_EXPONENT: f32 = 0.5;
/// incision coefficient at `strength` 1.0 on a `REFERENCE_RES` map
const STRENGTH_MAX: f32 = 8.0;
/// drainage area, as a fraction of the map, below which every cell incises at the same rate :
/// the hillslopes get a common minimum rate instead of a single cell's share of the trunk
const HILLSLOPE_AREA: f32 = 1e-4;
/// share of the excess a talus pass moves (ThermalErosion's `strength`)
const TALUS_STRENGTH: f32 = 0.5;
/// plain hover text of the strength parameter
const STRENGTH_HELP: &str =
    "How fast rivers cut into the land: 0 = no change, 1 = deep valleys in a few rounds";
/// plain hover text of the talus parameter
const TALUS_HELP: &str = "How much the valley sides crumble: 0 = not at all, 1 = every slope";

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct FluvialErosionConf {
    /// incision rate, 0 (nothing) to 1
    pub strength: f32,
    /// how much the slopes crumble between two incisions, 0 (nothing) to 1; ThermalErosion's
    /// mapping. Files saved before the talus existed load as 0 : pure incision
    #[serde(default)]
    pub talus: f32,
    /// height added to every land cell each iteration
    pub uplift: f32,
    /// implicit time steps
    pub iterations: u32,
    /// cells at or below this height are the sea : fixed, and where every river ends
    pub water_level: f32,
    /// longest side of the grid the iterations run on
    pub work_res: u32,
}

impl Default for FluvialErosionConf {
    fn default() -> Self {
        Self {
            strength: DEFAULT_STRENGTH,
            talus: DEFAULT_TALUS,
            uplift: DEFAULT_UPLIFT,
            iterations: DEFAULT_ITERATIONS,
            water_level: DEFAULT_WATER_LEVEL,
            work_res: DEFAULT_WORK_RES,
        }
    }
}

pub fn render_fluvial_erosion(ui: &mut egui::Ui, conf: &mut FluvialErosionConf) {
    ui.horizontal(|ui| {
        ui.label("strength").on_hover_text(STRENGTH_HELP);
        ui.add(
            egui::DragValue::new(&mut conf.strength)
                .speed(0.01)
                .range(0.0..=1.0),
        )
        .on_hover_text(STRENGTH_HELP);
        ui.label("talus").on_hover_text(TALUS_HELP);
        ui.add(
            egui::DragValue::new(&mut conf.talus)
                .speed(0.01)
                .range(0.0..=1.0),
        )
        .on_hover_text(TALUS_HELP);
        ui.label("iterations")
            .on_hover_text("How many rounds to run: more = deeper, more settled rivers, slower");
        ui.add(
            egui::DragValue::new(&mut conf.iterations)
                .speed(1)
                .range(1..=500),
        );
    });
    render_fluvial_row(ui, conf);
}

fn render_fluvial_row(ui: &mut egui::Ui, conf: &mut FluvialErosionConf) {
    ui.horizontal(|ui| {
        ui.label("uplift").on_hover_text(
            "How much the land rises each round while the rivers cut it: higher = steeper relief",
        );
        ui.add(
            egui::DragValue::new(&mut conf.uplift)
                .speed(0.0001)
                .range(0.0..=0.01),
        );
        ui.label("water level")
            .on_hover_text("Sea level: rivers end here and the sea itself never changes");
        ui.add(
            egui::DragValue::new(&mut conf.water_level)
                .speed(0.01)
                .range(-10.0..=10.0),
        );
        ui.label("resolution")
            .on_hover_text("Level of detail the rivers are carved at: higher = finer, much slower");
        egui::ComboBox::from_id_salt("fluvial_work_res")
            .selected_text(format!("{}", conf.work_res))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut conf.work_res, 256, "256");
                ui.selectable_value(&mut conf.work_res, 512, "512");
                ui.selectable_value(&mut conf.work_res, 1024, "1024");
                ui.selectable_value(&mut conf.work_res, 2048, "2048");
            });
    });
}

/// the conf converted to the working grid : `scale` is working cells per reference cell
struct FluvialParams {
    /// incision coefficient; slopes are measured per reference cell, hence the `scale` factor
    k: f32,
    uplift: f32,
    iterations: usize,
    water_level: f32,
    /// the talus passes run between two incisions; `passes` is `ceil(scale)`, as one
    /// ThermalErosion iteration
    talus: ThermalParams,
}

impl FluvialParams {
    fn new(conf: &FluvialErosionConf, scale: f32) -> Self {
        let talus_conf = ThermalErosionConf {
            talus: conf.talus,
            strength: TALUS_STRENGTH,
            iterations: 1,
            water_level: conf.water_level,
            work_res: conf.work_res,
        };
        Self {
            k: STRENGTH_MAX * conf.strength * scale,
            uplift: conf.uplift,
            iterations: (conf.iterations as usize).max(1),
            water_level: conf.water_level,
            talus: ThermalParams::new(&talus_conf, scale),
        }
    }
}

pub fn gen_fluvial_erosion(
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &FluvialErosionConf,
    progress: &mut Progress,
) {
    let work = work_size(size, conf.work_res as usize);
    let scale = work.0.max(work.1) as f32 / REFERENCE_RES;
    let params = FluvialParams::new(conf, scale);
    if work == size {
        incise_iterations(size, hmap, &params, progress);
        return;
    }
    // erode a reduced copy, then add the height delta back onto the full map
    let small = downsample(hmap, size, work);
    let mut eroded = small.clone();
    if !incise_iterations(work, &mut eroded, &params, progress) {
        // cancelled : leave the map untouched
        return;
    }
    for (delta, initial) in eroded.iter_mut().zip(small.iter()) {
        *delta -= initial;
    }
    add_upsampled(hmap, size, &eroded, work);
}

/// routes, incises and crumbles `hmap` once per iteration; returns false when the step was
/// cancelled, `hmap` then untouched by the cancelled incision or talus pass
fn incise_iterations(
    size: (usize, usize),
    hmap: &mut [f32],
    params: &FluvialParams,
    progress: &mut Progress,
) -> bool {
    let mut net = FlowNet::new();
    let mut out = hmap.to_vec();
    let iterations = params.iterations as f32;
    let passes = params.talus.passes as f32;
    for it in 0..params.iterations {
        if !progress.report(it as f32 / iterations) {
            return false;
        }
        net.route(size, hmap, params.water_level);
        incise(size, hmap, &net, params);
        // the incision takes the first half of the iteration's progress, the talus the second
        for pass in 0..params.talus.passes {
            out.copy_from_slice(hmap);
            let from = (it as f32 + 0.5 + 0.5 * pass as f32 / passes) / iterations;
            let to = (it as f32 + 0.5 + 0.5 * (pass + 1) as f32 / passes) / iterations;
            if !slide_pass(size, hmap, &mut out, &params.talus, from, to, progress) {
                return false;
            }
            hmap.copy_from_slice(&out);
        }
    }
    true
}

/// incision coefficient of a cell draining `area` cells out of `n_cells`, `d` cells away from
/// its receiver; areas below `HILLSLOPE_AREA` of the map share the floor's rate
fn incision_coef(area: f32, n_cells: f32, k: f32, d: f32) -> f32 {
    k * (area.max(HILLSLOPE_AREA * n_cells) / n_cells).powf(AREA_EXPONENT) / d
}

/// one implicit stream-power step in drainage order : a cell's receiver already holds its new
/// height when the cell is solved, so any incision coefficient is stable. Sea cells are
/// skipped, border land drains to `water_level` one cell away, and no receiver counts as
/// lower than `water_level`
fn incise(size: (usize, usize), hmap: &mut [f32], net: &FlowNet, params: &FluvialParams) {
    let n_cells = (size.0 * size.1) as f32;
    for &i in &net.order {
        let i = i as usize;
        if hmap[i] <= params.water_level {
            continue;
        }
        let (h_recv, d) = if net.is_base_level(i) {
            (params.water_level, 1.0)
        } else {
            let r = net.recv[i] as usize;
            (
                hmap[r].max(params.water_level),
                receiver_distance(i, r, size.0),
            )
        };
        let c = incision_coef(net.area[i], n_cells, params.k, d);
        hmap[i] = (hmap[i] + params.uplift + c * h_recv) / (1.0 + c);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{bilinear, DIRX, DIRY};
    use super::*;

    const SIZE: (usize, usize) = (16, 16);

    // every fixture keeps its per-cell slope below 0.96, the `talus` 0 threshold at 16×16
    // (`TALUS_MAX` × 512 / 16), so `conf16()` runs pure incision

    /// Euclidean cone, slope 0.5 per cell : no pits, every interior cell has a lower neighbour
    fn cone(size: (usize, usize)) -> Vec<f32> {
        let mut hmap = vec![0.0; size.0 * size.1];
        for y in 0..size.1 {
            for x in 0..size.0 {
                let dx = x as f32 - 7.5;
                let dy = y as f32 - 7.5;
                hmap[x + y * size.0] = (16.0 - (dx * dx + dy * dy).sqrt()) * 0.5;
            }
        }
        hmap
    }

    /// height grows with x, 0.5 per cell
    fn ramp() -> Vec<f32> {
        (0..256).map(|i| (i % 16) as f32 * 0.5).collect()
    }

    /// 16×16 map at 0.5 with a plateau at 1.0 for `x >= 8`
    fn step() -> Vec<f32> {
        (0..256)
            .map(|i| if i % 16 < 8 { 0.5 } else { 1.0 })
            .collect()
    }

    fn conf16() -> FluvialErosionConf {
        FluvialErosionConf {
            work_res: 16,
            talus: 0.0,
            ..Default::default()
        }
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

    /// mean of the 3×3 block around the cone's apex
    fn apex_mean(hmap: &[f32]) -> f32 {
        let mut sum = 0.0;
        for y in 6..9 {
            for x in 6..9 {
                sum += hmap[x + y * 16];
            }
        }
        sum / 9.0
    }

    fn erode(size: (usize, usize), hmap: &[f32], conf: &FluvialErosionConf) -> Vec<f32> {
        let mut out = hmap.to_vec();
        gen_fluvial_erosion(size, &mut out, conf, &mut Progress::headless());
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

    fn is_border(i: usize) -> bool {
        let (x, y) = (i % 16, i / 16);
        x == 0 || y == 0 || x == 15 || y == 15
    }

    /// a flat map above the sea would drain from its border : the invariant one sits at the base level
    #[test]
    fn flat_map_at_the_base_level_stays_flat() {
        let flat = vec![0.5; 256];
        let conf = FluvialErosionConf {
            water_level: 0.5,
            ..conf16()
        };
        let out = erode(SIZE, &flat, &conf);
        assert!(out.iter().all(|h| (h - 0.5).abs() < 1e-6));
    }

    #[test]
    fn bigger_drainage_incises_faster() {
        let input = ramp();
        let conf = FluvialErosionConf {
            strength: 1.0,
            iterations: 1,
            water_level: -1.0,
            ..conf16()
        };
        let out = erode(SIZE, &input, &conf);
        let drop = |i: usize| input[i] - out[i];
        assert!(
            drop(1 + 8 * 16) > drop(14 + 8 * 16),
            "14 cells: {}, 1 cell: {}",
            drop(1 + 8 * 16),
            drop(14 + 8 * 16)
        );
        assert!(drop(14 + 8 * 16) > 0.0);
    }

    #[test]
    fn strength_zero_and_uplift_zero_is_identity() {
        let input = cone(SIZE);
        let conf = FluvialErosionConf {
            strength: 0.0,
            ..conf16()
        };
        assert_eq!(erode(SIZE, &input, &conf), input);
    }

    #[test]
    fn erosion_only_lowers_land() {
        let input = cone(SIZE);
        let conf = FluvialErosionConf {
            strength: 1.0,
            iterations: 20,
            ..conf16()
        };
        let out = erode(SIZE, &input, &conf);
        for i in 0..256 {
            assert!(out[i] <= input[i] + 1e-6, "cell {i} rose");
            if is_border(i) {
                assert!(
                    out[i] < input[i],
                    "border cell {i} did not drain to the base level"
                );
            }
        }
        let sum_in: f32 = input.iter().sum();
        let sum_out: f32 = out.iter().sum();
        assert!(sum_out < sum_in, "nothing eroded");
    }

    #[test]
    fn uplift_raises_land_not_sea() {
        let mut input = vec![0.5; 256];
        input[5 + 5 * 16] = 0.0;
        let conf = FluvialErosionConf {
            strength: 0.0,
            uplift: 0.01,
            iterations: 10,
            ..conf16()
        };
        let out = erode(SIZE, &input, &conf);
        for i in 0..256 {
            if i == 5 + 5 * 16 {
                assert_eq!(out[i], input[i], "sea cell {i} moved");
            } else {
                assert!((out[i] - 0.6).abs() < 1e-5, "cell {i} : {}", out[i]);
            }
        }
    }

    #[test]
    fn incision_coef_floors_small_areas() {
        let n = 1e6;
        assert_eq!(
            incision_coef(1.0, n, 1.0, 1.0),
            incision_coef(HILLSLOPE_AREA * n, n, 1.0, 1.0)
        );
        assert_eq!(incision_coef(5e5, n, 1.0, 1.0), 0.5f32.sqrt());
        assert_eq!(
            incision_coef(5e5, n, 1.0, std::f32::consts::SQRT_2),
            0.5f32.sqrt() / std::f32::consts::SQRT_2
        );
    }

    #[test]
    fn border_land_drains_to_water_level() {
        let input: Vec<f32> = (0..256).map(|i| 1.0 + (i % 16) as f32 / 15.0).collect();
        let conf = FluvialErosionConf {
            strength: 1.0,
            iterations: 10,
            water_level: 0.0,
            ..conf16()
        };
        let out = erode(SIZE, &input, &conf);
        for i in 0..256 {
            if is_border(i) {
                assert!(out[i] < input[i], "border cell {i} did not drop");
                assert!(out[i] >= 0.0, "border cell {i} went below the base level");
            }
        }
    }

    #[test]
    fn nothing_cuts_below_the_sea() {
        let mut input = vec![0.4; 256];
        let pit = 5 + 5 * 16;
        input[pit] = -0.4;
        let conf = FluvialErosionConf {
            strength: 1.0,
            iterations: 20,
            water_level: 0.0,
            ..conf16()
        };
        let out = erode(SIZE, &input, &conf);
        assert_eq!(out[pit], -0.4, "the sea cell changed");
        for i in 0..256 {
            if i != pit {
                assert!(out[i] >= 0.0, "cell {i} cut below the sea : {}", out[i]);
            }
        }
    }

    #[test]
    fn sea_cells_stay_fixed() {
        let input = cone(SIZE);
        let conf = FluvialErosionConf {
            strength: 1.0,
            iterations: 10,
            water_level: 4.0,
            ..conf16()
        };
        let out = erode(SIZE, &input, &conf);
        for i in 0..256 {
            if input[i] <= 4.0 {
                assert_eq!(out[i], input[i], "sea cell {i} changed");
            }
        }
    }

    #[test]
    fn same_input_is_identical() {
        let input = cone(SIZE);
        let conf = FluvialErosionConf {
            strength: 1.0,
            ..conf16()
        };
        assert_eq!(erode(SIZE, &input, &conf), erode(SIZE, &input, &conf));
    }

    #[test]
    fn params_scale_with_the_working_grid() {
        let conf = FluvialErosionConf::default();
        let full = FluvialParams::new(&conf, 1.0);
        assert_eq!(full.k, STRENGTH_MAX * conf.strength);
        assert_eq!(full.iterations, conf.iterations as usize);
        let quarter = FluvialParams::new(&conf, 0.25);
        assert_eq!(quarter.k, full.k * 0.25);
        assert_eq!(quarter.iterations, full.iterations);
        assert_eq!(quarter.uplift, conf.uplift);
    }

    /// `talus` 0.5 keeps the threshold above zero : at exactly zero, equal neighbours are a
    /// tie the block-average rounding of the downsample would break differently
    #[test]
    fn working_grid_result_is_the_upsampled_in_place_delta() {
        for talus in [0.0, 0.5] {
            let conf = FluvialErosionConf {
                strength: 1.0,
                talus,
                ..conf16()
            };
            let input16 = cone(SIZE);
            let out16 = erode(SIZE, &input16, &conf);
            let delta16: Vec<f32> = out16
                .iter()
                .zip(input16.iter())
                .map(|(o, i)| o - i)
                .collect();

            let input64 = blow_up(&input16, SIZE, 4);
            let out64 = erode((64, 64), &input64, &conf);

            for y in 0..64 {
                for x in 0..64 {
                    let u = ((x as f32 + 0.5) * 16.0 / 64.0 - 0.5).max(0.0);
                    let v = ((y as f32 + 0.5) * 16.0 / 64.0 - 0.5).max(0.0);
                    let expected = input64[x + y * 64] + bilinear(&delta16, u, v, SIZE);
                    let got = out64[x + y * 64];
                    assert!(
                        (got - expected).abs() < 1e-4,
                        "talus {talus} ({x}, {y}) : got {got}, expected {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn non_square_maps_run() {
        let conf = FluvialErosionConf {
            strength: 1.0,
            ..Default::default()
        };
        for size in [(16, 32), (32, 16)] {
            let input = cone(size);
            assert_ne!(erode(size, &input, &conf), input, "{size:?} unchanged");
        }
    }

    #[test]
    fn cancelled_run_leaves_map_untouched() {
        let input = cone(SIZE);
        let conf = FluvialErosionConf {
            strength: 1.0,
            ..conf16()
        };
        assert_ne!(erode(SIZE, &input, &conf), input);
        let (tx, _) = std::sync::mpsc::channel();
        let mut cancelled = input.clone();
        gen_fluvial_erosion(
            SIZE,
            &mut cancelled,
            &conf,
            &mut Progress::preview(tx, 1.0, || true),
        );
        assert_eq!(cancelled, input);
    }

    /// the defaults on a stock map : the numbers this prints are what a launch calibrates against
    #[test]
    fn default_conf_carves_a_stock_map() {
        use super::super::calib;
        let size = (128, 128);
        let input = calib::stock_map(1234, size);
        let conf = FluvialErosionConf {
            work_res: 128,
            ..Default::default()
        };
        let out = erode(size, &input, &conf);
        let stats = calib::print_stats("fluvial defaults on fbm 128", &input, &out);
        assert!(
            out.iter().all(|h| *h <= 1.0 + 1e-6),
            "a cell rose above the input range"
        );
        // the accepted render measured 0.38 and 86.5 % here : ±50 % on the drop, half the share
        assert!(
            (0.19..=0.57).contains(&stats.max_drop),
            "max drop {} outside the calibration band",
            stats.max_drop
        );
        assert!(
            stats.carved >= 0.43,
            "only {:.1} % of the cells carved",
            stats.carved * 100.0
        );
    }

    #[test]
    fn conf_round_trips_through_ron() {
        let conf = FluvialErosionConf {
            strength: 0.7,
            talus: 0.4,
            uplift: 0.002,
            iterations: 12,
            water_level: 0.1,
            work_res: 1024,
        };
        let text = ron::to_string(&conf).unwrap();
        let back: FluvialErosionConf = ron::from_str(&text).unwrap();
        assert_eq!(back, conf);
    }

    /// a file saved before the talus existed keeps pure incision
    #[test]
    fn conf_without_talus_loads() {
        let text = "(strength:0.7,uplift:0.002,iterations:12,water_level:0.1,work_res:1024)";
        let conf: FluvialErosionConf = ron::from_str(text).unwrap();
        assert_eq!(conf.talus, 0.0);
        assert_eq!(conf.strength, 0.7);
        assert_eq!(conf.work_res, 1024);
    }

    #[test]
    fn talus_zero_is_pure_incision() {
        let input = step();
        let conf = FluvialErosionConf {
            strength: 0.0,
            water_level: -1.0,
            ..conf16()
        };
        assert_eq!(erode(SIZE, &input, &conf), input);
    }

    #[test]
    fn talus_crumbles_the_step() {
        let input = step();
        let conf = FluvialErosionConf {
            strength: 0.0,
            talus: 1.0,
            iterations: 1,
            water_level: -1.0,
            ..conf16()
        };
        let out = erode(SIZE, &input, &conf);
        assert!(
            max_neighbour_diff(SIZE, &out) < 0.5,
            "the step did not crumble : {}",
            max_neighbour_diff(SIZE, &out)
        );
        let sum_in: f32 = input.iter().sum();
        let sum_out: f32 = out.iter().sum();
        assert!(
            (sum_in - sum_out).abs() < 1e-3,
            "mass changed : {sum_in} -> {sum_out}"
        );
    }

    #[test]
    fn talus_and_incision_co_evolve() {
        let input = cone(SIZE);
        let pure = FluvialErosionConf {
            strength: 1.0,
            iterations: 20,
            ..conf16()
        };
        let with_talus = FluvialErosionConf {
            talus: 1.0,
            ..pure.clone()
        };
        let out_pure = erode(SIZE, &input, &pure);
        let out_talus = erode(SIZE, &input, &with_talus);
        assert!(
            apex_mean(&out_talus) < apex_mean(&out_pure),
            "apex : talus {}, pure {}",
            apex_mean(&out_talus),
            apex_mean(&out_pure)
        );
        // the talus alone (mass conserving) also lowers the apex; the two together lower it further
        let talus_alone = FluvialErosionConf {
            strength: 0.0,
            ..with_talus.clone()
        };
        let out_alone = erode(SIZE, &input, &talus_alone);
        assert!(
            apex_mean(&out_talus) < apex_mean(&out_alone),
            "apex : both {}, talus alone {}",
            apex_mean(&out_talus),
            apex_mean(&out_alone)
        );
    }

    #[test]
    fn talus_passes_follow_the_grid() {
        let conf = FluvialErosionConf::default();
        assert_eq!(FluvialParams::new(&conf, 4.0).talus.passes, 4);
        assert_eq!(FluvialParams::new(&conf, 1.0).talus.passes, 1);
        assert_eq!(FluvialParams::new(&conf, 0.25).talus.passes, 1);
        assert_eq!(
            FluvialParams::new(&conf, 0.25).talus.threshold,
            4.0 * FluvialParams::new(&conf, 1.0).talus.threshold
        );
    }
}
