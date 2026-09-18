use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use super::{add_upsampled, bilinear, downsample, work_size, Progress};

// water erosion algorithm adapted from https://www.firespark.de/resources/downloads/implementation%20of%20a%20methode%20for%20hydraulic%20erosion.pdf
const MAX_PATH_LENGTH: usize = 40;
const DEFAULT_EVAPORATION: f32 = 0.05;
const DEFAULT_CAPACITY: f32 = 8.0;
const DEFAULT_MIN_SLOPE: f32 = 0.05;
const DEFAULT_DEPOSITION: f32 = 0.1;
const DEFAULT_INERTIA: f32 = 0.4;
const DEFAULT_DROP_AMOUNT: f32 = 0.5;
const DEFAULT_EROSION_STRENGTH: f32 = 0.1;
const DEFAULT_RADIUS: f32 = 4.0;
const DEFAULT_WORK_RES: u32 = 512;
/// grid the distance-like parameters are expressed against
const REFERENCE_RES: f32 = 512.0;
/// acceleration applied to the drop by the slope, per reference cell
const GRAVITY: f32 = 1.0;

fn default_work_res() -> u32 {
    DEFAULT_WORK_RES
}

/// a drop of water
struct Drop {
    /// position on the grid
    pub pos: (f32, f32),
    /// water amount
    pub water: f32,
    /// movement direction
    pub dir: (f32, f32),
    /// maximum sediment capacity of the drop
    pub capacity: f32,
    /// amount of accumulated sediment
    pub sediment: f32,
    /// velocity
    pub speed: f32,
}

impl Drop {
    pub fn grid_offset(&self, grid_width: usize) -> usize {
        self.pos.0 as usize + self.pos.1 as usize * grid_width
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct WaterErosionConf {
    drop_amount: f32,
    erosion_strength: f32,
    evaporation: f32,
    capacity: f32,
    min_slope: f32,
    deposition: f32,
    inertia: f32,
    radius: f32,
    #[serde(default)]
    water_level: f32,
    #[serde(default = "default_work_res")]
    work_res: u32,
}

impl Default for WaterErosionConf {
    fn default() -> Self {
        Self {
            drop_amount: DEFAULT_DROP_AMOUNT,
            erosion_strength: DEFAULT_EROSION_STRENGTH,
            evaporation: DEFAULT_EVAPORATION,
            capacity: DEFAULT_CAPACITY,
            min_slope: DEFAULT_MIN_SLOPE,
            deposition: DEFAULT_DEPOSITION,
            inertia: DEFAULT_INERTIA,
            radius: DEFAULT_RADIUS,
            water_level: 0.0,
            work_res: DEFAULT_WORK_RES,
        }
    }
}

pub fn render_water_erosion(ui: &mut egui::Ui, conf: &mut WaterErosionConf) {
    ui.horizontal(|ui| {
        ui.label("drop amount")
            .on_hover_text("How much rain falls on the map: more = more erosion, slower");
        ui.add(
            egui::DragValue::new(&mut conf.drop_amount)
                .speed(0.01)
                .range(0.1..=2.0),
        );
        ui.label("erosion strength")
            .on_hover_text("How much ground each drop digs out");
        ui.add(
            egui::DragValue::new(&mut conf.erosion_strength)
                .speed(0.01)
                .range(0.01..=1.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("drop capacity")
            .on_hover_text("How much soil a drop can carry before it starts leaving it behind");
        ui.add(
            egui::DragValue::new(&mut conf.capacity)
                .speed(0.5)
                .range(2.0..=32.0),
        );
        ui.label("inertia")
            .on_hover_text("How straight the drops run: higher = smoother, longer channels");
        ui.add(
            egui::DragValue::new(&mut conf.inertia)
                .speed(0.01)
                .range(0.01..=0.5),
        );
    });
    ui.horizontal(|ui| {
        ui.label("deposition")
            .on_hover_text("How quickly a drop lets go of the soil it carries");
        ui.add(
            egui::DragValue::new(&mut conf.deposition)
                .speed(0.01)
                .range(0.01..=1.0),
        );
        ui.label("evaporation")
            .on_hover_text("How fast drops dry up: higher = shorter, gentler channels");
        ui.add(
            egui::DragValue::new(&mut conf.evaporation)
                .speed(0.01)
                .range(0.01..=0.5),
        );
    });
    ui.horizontal(|ui| {
        ui.label("radius")
            .on_hover_text("Width of the groove a drop carves");
        ui.add(
            egui::DragValue::new(&mut conf.radius)
                .speed(0.1)
                .range(1.0..=10.0),
        );
        ui.label("minimum slope")
            .on_hover_text("Keeps drops digging on nearly flat ground: higher = more");
        ui.add(
            egui::DragValue::new(&mut conf.min_slope)
                .speed(0.001)
                .range(0.001..=0.1),
        );
    });
    render_water_row(ui, conf);
}

/// water level and the resolution the drops run at
fn render_water_row(ui: &mut egui::Ui, conf: &mut WaterErosionConf) {
    ui.horizontal(|ui| {
        ui.label("water level")
            .on_hover_text("Sea level: drops stop when they reach it and none start below it");
        ui.add(
            egui::DragValue::new(&mut conf.water_level)
                .speed(0.01)
                .range(-10.0..=10.0),
        );
        ui.label("resolution")
            .on_hover_text("Level of detail the erosion works at: higher = finer, much slower");
        egui::ComboBox::from_id_salt("work_res")
            .selected_text(format!("{}", conf.work_res))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut conf.work_res, 256, "256");
                ui.selectable_value(&mut conf.work_res, 512, "512");
                ui.selectable_value(&mut conf.work_res, 1024, "1024");
                ui.selectable_value(&mut conf.work_res, 2048, "2048");
            });
    });
}

/// the working grid a call runs on : its size, and the distances derived from `scale`
struct DropParams {
    /// working grid the drops run on
    size: (usize, usize),
    /// steps a drop may take on the working grid
    path_len: usize,
    /// working cells per reference cell
    scale: f32,
    /// erosion brush : (dx, dy, weight), weights summing to 1
    kernel: Vec<(i32, i32, f32)>,
}

impl DropParams {
    fn new(conf: &WaterErosionConf, size: (usize, usize), scale: f32) -> Self {
        let radius = (conf.radius * scale).max(1.0);
        Self {
            size,
            path_len: ((MAX_PATH_LENGTH as f32 * scale).round() as usize).max(4),
            scale,
            kernel: erosion_kernel(radius),
        }
    }
}

/// every offset within `radius`, weighted by `radius - dist`, normalized to sum 1
fn erosion_kernel(radius: f32) -> Vec<(i32, i32, f32)> {
    let extent = radius.ceil() as i32;
    let mut kernel = Vec::new();
    let mut total = 0.0;
    for y in -extent..=extent {
        for x in -extent..=extent {
            let dist = ((x * x + y * y) as f32).sqrt();
            if dist < radius {
                kernel.push((x, y, radius - dist));
                total += radius - dist;
            }
        }
    }
    for (_, _, weight) in kernel.iter_mut() {
        *weight /= total;
    }
    kernel
}

pub fn gen_water_erosion(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &WaterErosionConf,
    progress: &mut Progress,
) {
    let work = work_size(size, conf.work_res as usize);
    let scale = work.0.max(work.1) as f32 / REFERENCE_RES;
    if work == size {
        erode_particles(seed, size, hmap, conf, scale, progress);
        return;
    }
    // erode a reduced copy, then add the height delta back onto the full map
    let small = downsample(hmap, size, work);
    let mut eroded = small.clone();
    if !erode_particles(seed, work, &mut eroded, conf, scale, progress) {
        // cancelled : leave the map untouched
        return;
    }
    for (delta, initial) in eroded.iter_mut().zip(small.iter()) {
        *delta -= initial;
    }
    add_upsampled(hmap, size, &eroded, work);
}

/// runs the drops on `hmap`; returns false when the step was cancelled
fn erode_particles(
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &WaterErosionConf,
    scale: f32,
    progress: &mut Progress,
) -> bool {
    if size.0 < 2 || size.1 < 2 {
        return true;
    }
    let mut rng = StdRng::seed_from_u64(seed);
    let params = DropParams::new(conf, size, scale);
    // maximum drop count is 2 per cell
    let drop_count = ((size.1 * 2) as f32 * conf.drop_amount) as usize;
    // use a double loop to check progress every size.0 drops
    for y in 0..drop_count {
        for _ in 0..size.0 {
            let start = (
                rng.random_range(0..size.0 - 1),
                rng.random_range(0..size.1 - 1),
            );
            simulate_drop(hmap, conf, &params, &mut rng, start);
        }
        if !progress.report(y as f32 / drop_count as f32) {
            return false;
        }
    }
    true
}

/// downhill direction at a fractional position, blended with the drop's previous direction
fn descent_dir(
    h: (f32, f32, f32, f32),
    frac: (f32, f32),
    dir: (f32, f32),
    inertia: f32,
    rng: &mut StdRng,
) -> (f32, f32) {
    let (h00, h10, h01, h11) = h;
    let (u, v) = frac;
    let mut gx = (h00 - h10) * (1.0 - v) + (h01 - h11) * v;
    let mut gy = (h00 - h01) * (1.0 - u) + (h10 - h11) * u;
    (gx, gy) = normalize_dir(gx, gy, rng);
    // interpolate between old direction and new one to account for inertia
    gx = (dir.0 - gx) * inertia + gx;
    gy = (dir.1 - gy) * inertia + gy;
    normalize_dir(gx, gy, rng)
}

/// spreads `amount` over the four cells around a position, by their bilinear weights
fn deposit(hmap: &mut [f32], width: usize, off: usize, amount: f32, weights: (f32, f32, f32, f32)) {
    hmap[off] += amount * weights.0;
    hmap[off + 1] += amount * weights.1;
    hmap[off + width] += amount * weights.2;
    hmap[off + 1 + width] += amount * weights.3;
}

/// digs `amount` around `cell` following the brush; weight falling outside the map is lost
fn erode_around(
    hmap: &mut [f32],
    size: (usize, usize),
    cell: (usize, usize),
    amount: f32,
    kernel: &[(i32, i32, f32)],
) {
    for (dx, dy, weight) in kernel.iter() {
        let x = cell.0 as i32 + dx;
        let y = cell.1 as i32 + dy;
        if x < 0 || y < 0 || x >= size.0 as i32 || y >= size.1 as i32 {
            continue;
        }
        hmap[x as usize + y as usize * size.0] -= amount * weight;
    }
}

/// downhill step : the drop deposits its excess sediment or digs the brush into the map
fn erode_or_deposit(
    hmap: &mut [f32],
    conf: &WaterErosionConf,
    params: &DropParams,
    drop: &mut Drop,
    old_cell: (usize, usize),
    weights: (f32, f32, f32, f32),
    hdif: f32,
) {
    let slope = -hdif / params.scale;
    let old_off = old_cell.0 + old_cell.1 * params.size.0;
    drop.capacity = conf.min_slope.max(slope) * drop.water * conf.capacity * drop.speed;
    if drop.sediment > drop.capacity {
        // too much sediment in the drop. deposit
        let amount = (drop.sediment - drop.capacity) * conf.deposition;
        deposit(hmap, params.size.0, old_off, amount, weights);
        drop.sediment -= amount;
    } else {
        // erode around the old cell
        let amount = ((drop.capacity - drop.sediment) * conf.erosion_strength).min(-hdif);
        erode_around(hmap, params.size, old_cell, amount, &params.kernel);
        drop.sediment += amount;
    }
}

/// one drop, from `start` until it leaves the map, runs out of path or reaches the water level
fn simulate_drop(
    hmap: &mut [f32],
    conf: &WaterErosionConf,
    params: &DropParams,
    rng: &mut StdRng,
    start: (usize, usize),
) {
    let size = params.size;
    let mut off = start.0 + start.1 * size.0;
    if hmap[off] < conf.water_level {
        return;
    }
    let mut drop = Drop {
        pos: (start.0 as f32, start.1 as f32),
        dir: (0.0, 0.0),
        sediment: 0.0,
        water: 1.0,
        capacity: conf.capacity,
        speed: 0.0,
    };
    let mut count = 0;
    while count < params.path_len {
        let oldh = hmap[off];
        let old_off = off;
        let old_cell = (drop.pos.0 as usize, drop.pos.1 as usize);
        // interpolate slope at old position
        let h00 = oldh;
        let h10 = hmap[off + 1];
        let h01 = hmap[off + size.0];
        let h11 = hmap[off + 1 + size.0];
        let old_u = drop.pos.0.fract();
        let old_v = drop.pos.1.fract();
        // weight for each cell surrounding the drop position
        let (w00, w10, w01, w11) = (
            (1.0 - old_u) * (1.0 - old_v),
            old_u * (1.0 - old_v),
            (1.0 - old_u) * old_v,
            old_u * old_v,
        );
        let weights = (w00, w10, w01, w11);
        (drop.dir.0, drop.dir.1) = descent_dir(
            (h00, h10, h01, h11),
            (old_u, old_v),
            drop.dir,
            conf.inertia,
            rng,
        );
        // compute the droplet new position
        drop.pos.0 += drop.dir.0;
        drop.pos.1 += drop.dir.1;
        if drop.pos.0 < 0.0
            || drop.pos.1 < 0.0
            || drop.pos.0 >= (size.0 - 1) as f32
            || drop.pos.1 >= (size.1 - 1) as f32
        {
            // out of the map
            break;
        }
        off = drop.grid_offset(size.0);
        // interpolate height at new drop position
        let newh = bilinear(hmap, drop.pos.0, drop.pos.1, size);
        if newh < conf.water_level {
            // the drop reached the water : its sediment is lost
            break;
        }
        let hdif = newh - oldh;
        // height drop per reference cell, positive downhill
        let slope = -hdif / params.scale;
        if hdif >= 0.0 {
            // going uphill : deposit sediment at old position
            let amount = drop.sediment.min(hdif);
            deposit(hmap, size.0, old_off, amount, weights);
            drop.sediment -= amount;
            if drop.sediment <= 0.0 && drop.speed == 0.0 {
                // nothing left to carry and no momentum. stop the path
                break;
            }
        } else {
            erode_or_deposit(hmap, conf, params, &mut drop, old_cell, weights, hdif);
        }
        drop.speed = (drop.speed * drop.speed + slope * GRAVITY).max(0.0).sqrt();
        drop.water *= 1.0 - conf.evaporation;
        count += 1;
    }
}

fn normalize_dir(dx: f32, dy: f32, rng: &mut StdRng) -> (f32, f32) {
    let len = (dx * dx + dy * dy).sqrt();
    if len < std::f32::EPSILON {
        // random direction
        let angle = rng.random_range(0.0..std::f32::consts::PI * 2.0);
        (angle.cos(), angle.sin())
    } else {
        (dx / len, dy / len)
    }
}

#[cfg(test)]
mod tests {
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

    fn erode(seed: u64, size: (usize, usize), conf: &WaterErosionConf) -> Vec<f32> {
        let mut hmap = pyramid(size);
        gen_water_erosion(seed, size, &mut hmap, conf, &mut Progress::headless());
        hmap
    }

    #[test]
    fn same_seed_is_identical() {
        let conf = WaterErosionConf::default();
        let a = erode(42, (16, 16), &conf);
        assert_eq!(
            a,
            erode(42, (16, 16), &conf),
            "same seed gave a different map"
        );
        assert_ne!(a, erode(43, (16, 16), &conf), "two seeds gave the same map");
    }

    #[test]
    fn erosion_changes_the_map() {
        let input = pyramid((16, 16));
        let out = erode(7, (16, 16), &WaterErosionConf::default());
        assert_ne!(out, input, "erosion left the map untouched");
        let delta: f32 = out.iter().zip(input.iter()).map(|(o, i)| o - i).sum();
        assert!(
            delta < 0.0,
            "net change over a pyramid is {delta}, expected erosion"
        );
    }

    #[test]
    fn water_level_above_the_map_leaves_it_untouched() {
        let conf = WaterErosionConf {
            water_level: 100.0,
            ..Default::default()
        };
        assert_eq!(erode(7, (16, 16), &conf), pyramid((16, 16)));
    }

    #[test]
    fn non_square_maps_run() {
        for size in [(16, 32), (32, 16)] {
            let out = erode(7, size, &WaterErosionConf::default());
            assert_ne!(
                out,
                pyramid(size),
                "{size:?} : erosion left the map untouched"
            );
        }
    }

    #[test]
    fn kernel_weights_sum_to_one() {
        let sum: f32 = erosion_kernel(4.0).iter().map(|(_, _, w)| w).sum();
        assert!((sum - 1.0).abs() < 1e-5, "radius 4 weights sum to {sum}");
        assert_eq!(erosion_kernel(1.0), vec![(0, 0, 1.0)]);
    }

    #[test]
    fn conf_without_new_fields_loads() {
        let old = "(drop_amount: 0.5, erosion_strength: 0.08, evaporation: 0.05, capacity: 6.0,
                    min_slope: 0.05, deposition: 0.06, inertia: 0.5, radius: 4.0)";
        let conf: WaterErosionConf = ron::from_str(old).unwrap();
        assert_eq!(conf.water_level, 0.0);
        assert_eq!(conf.work_res, 512);
    }

    /// nearest-neighbour blow-up : each `factor` square block is constant, so downsampling
    /// back to the small grid is bit-exact
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

    #[test]
    fn working_grid_result_is_the_upsampled_in_place_delta() {
        let conf = WaterErosionConf {
            work_res: 16,
            ..Default::default()
        };
        let input16 = pyramid((16, 16));
        let out16 = erode(11, (16, 16), &conf);
        let delta16: Vec<f32> = out16
            .iter()
            .zip(input16.iter())
            .map(|(o, i)| o - i)
            .collect();

        let input64 = blow_up(&input16, (16, 16), 4);
        let mut out64 = input64.clone();
        gen_water_erosion(11, (64, 64), &mut out64, &conf, &mut Progress::headless());

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
    fn in_place_when_map_fits() {
        let big_res = erode(3, (16, 16), &WaterErosionConf::default());
        let exact = erode(
            3,
            (16, 16),
            &WaterErosionConf {
                work_res: 16,
                ..Default::default()
            },
        );
        assert_eq!(big_res, exact, "a map smaller than work_res was resampled");
    }

    #[test]
    fn non_square_working_grid() {
        let conf = WaterErosionConf {
            work_res: 16,
            ..Default::default()
        };
        let input = pyramid((64, 32));
        let mut out = input.clone();
        gen_water_erosion(5, (64, 32), &mut out, &conf, &mut Progress::headless());
        assert_ne!(
            out, input,
            "erosion on a non-square working grid did nothing"
        );
    }
}
