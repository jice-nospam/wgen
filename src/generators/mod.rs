mod fbm;
mod hills;
mod island;
mod landmass;
mod mid_point;
mod mudslide;
mod normalize;
mod water_erosion;

use std::sync::mpsc::Sender;

pub use fbm::{gen_fbm, render_fbm, FbmConf};
pub use hills::{gen_hills, render_hills, HillsConf};
pub use island::{gen_island, render_island, IslandConf};
pub use landmass::{gen_landmass, render_landmass, LandMassConf};
pub use mid_point::{gen_mid_point, render_mid_point, MidPointConf};
pub use mudslide::{gen_mudslide, render_mudslide, MudSlideConf};
pub use normalize::{gen_normalize, NormalizeConf};
pub use water_erosion::{gen_water_erosion, render_water_erosion, WaterErosionConf};

use crate::ThreadMessage;

const DIRX: [i32; 9] = [0, -1, 0, 1, -1, 1, -1, 0, 1];
const DIRY: [i32; 9] = [0, -1, -1, -1, 0, 0, 1, 1, 1];

pub fn vec_get_safe<T>(v: &Vec<T>, off: usize) -> T
where
    T: Default + Copy,
{
    if off < v.len() {
        return v[off];
    }
    T::default()
}

pub fn get_min_max(v: &[f32]) -> (f32, f32) {
    let mut min = v[0];
    let mut max = v[0];
    for val in v.iter().skip(1) {
        if *val > max {
            max = *val;
        } else if *val < min {
            min = *val;
        }
    }
    (min, max)
}

pub fn normalize(v: &mut [f32], target_min: f32, target_max: f32) {
    let (min, max) = get_min_max(v);
    let invmax = if min == max {
        0.0
    } else {
        (target_max - target_min) / (max - min)
    };
    for val in v {
        *val = target_min + (*val - min) * invmax;
    }
}

pub fn _blur(v: &mut [f32], size: (usize, usize)) {
    const FACTOR: usize = 8;
    let small_size: (usize, usize) = (
        (size.0 + FACTOR - 1) / FACTOR,
        (size.1 + FACTOR - 1) / FACTOR,
    );
    let mut low_res = vec![0.0; small_size.0 * small_size.1];
    for x in 0..size.0 {
        for y in 0..size.1 {
            let value = v[x + y * size.0];
            let ix = x / FACTOR;
            let iy = y / FACTOR;
            low_res[ix + iy * small_size.0] += value;
        }
    }
    let coef = 1.0 / FACTOR as f32;
    for x in 0..size.0 {
        for y in 0..size.1 {
            v[x + y * size.0] =
                _interpolate(&low_res, x as f32 * coef, y as f32 * coef, small_size);
        }
    }
}

pub fn _interpolate(v: &[f32], x: f32, y: f32, size: (usize, usize)) -> f32 {
    let ix = x as usize;
    let iy = y as usize;
    let dx = x.fract();
    let dy = y.fract();

    let val_nw = v[ix + iy * size.0];
    let val_ne = if ix < size.0 - 1 {
        v[ix + 1 + iy * size.0]
    } else {
        val_nw
    };
    let val_sw = if iy < size.1 - 1 {
        v[ix + (iy + 1) * size.0]
    } else {
        val_nw
    };
    let val_se = if ix < size.0 - 1 && iy < size.1 - 1 {
        v[ix + 1 + (iy + 1) * size.0]
    } else {
        val_nw
    };
    let val_n = (1.0 - dx) * val_nw + dx * val_ne;
    let val_s = (1.0 - dx) * val_sw + dx * val_se;
    (1.0 - dy) * val_n + dy * val_s
}

fn report_progress(progress: f32, export: bool, tx: Sender<ThreadMessage>) {
    if export {
        tx.send(ThreadMessage::ExporterStepProgress(progress))
            .unwrap();
    } else {
        tx.send(ThreadMessage::GeneratorStepProgress(progress))
            .unwrap();
    }
}
