mod fbm;
mod hills;
mod island;
mod landmass;
mod mid_point;
mod mudslide;
mod normalize;
mod water_erosion;

use std::sync::mpsc::Sender;

use noise::{Fbm, NoiseFn};

pub use fbm::{gen_fbm, render_fbm, FbmConf};
pub use hills::{gen_hills, render_hills, HillsConf};
pub use island::{gen_island, render_island, IslandConf};
pub use landmass::{gen_landmass, render_landmass, LandMassConf};
pub use mid_point::{gen_mid_point, render_mid_point, MidPointConf};
pub use mudslide::{gen_mudslide, render_mudslide, MudSlideConf};
pub use normalize::{gen_normalize, NormalizeConf};
pub use water_erosion::{gen_water_erosion, render_water_erosion, WaterErosionConf};

use crate::ThreadMessage;

const WATER_LEVEL: f32 = 0.12;
const OPPOSITE_DIR: [usize; 9] = [0, 8, 7, 6, 5, 4, 3, 2, 1];
const DIRX: [i32; 9] = [0, -1, 0, 1, -1, 1, -1, 0, 1];
const DIRY: [i32; 9] = [0, -1, -1, -1, 0, 0, 1, 1, 1];
const DIRCOEF: [f32; 9] = [
    1.0,
    1.0 / 1.414,
    1.0,
    1.0 / 1.414,
    1.0,
    1.0,
    1.0 / 1.414,
    1.0,
    1.0 / 1.414,
];

pub fn vec_get_safe<T>(v: &Vec<T>, off: usize) -> T
where
    T: Default + Copy,
{
    if off < v.len() {
        return v[off];
    }
    T::default()
}

pub fn get_min_max(v: &Vec<f32>) -> (f32, f32) {
    let mut min = v[0];
    let mut max = v[0];
    for i in 1..v.len() {
        if v[i] > max {
            max = v[i];
        } else if v[i] < min {
            min = v[i];
        }
    }
    (min, max)
}

pub fn normalize(v: &mut Vec<f32>, target_min: f32, target_max: f32) {
    let (min, max) = get_min_max(v);
    let invmax = if min == max {
        0.0
    } else {
        (target_max - target_min) / (max - min)
    };
    for i in 0..v.len() {
        v[i] = target_min + (v[i] - min) * invmax;
    }
}

pub fn blur(v: &mut Vec<f32>, size: (usize, usize)) {
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
            v[x + y * size.0] = interpolate(&low_res, x as f32 * coef, y as f32 * coef, small_size);
        }
    }
}

pub fn interpolate(v: &Vec<f32>, x: f32, y: f32, size: (usize, usize)) -> f32 {
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

/** water flow direction 0-8 : none NW N NE W E SW S SE
+ slope amount*/
pub fn compute_flow_and_slopes(size: (usize, usize), hmap: &Vec<f32>) -> (Vec<usize>, Vec<f32>) {
    let mut flow = Vec::with_capacity(size.0 * size.1);
    let mut slopes = Vec::with_capacity(size.0 * size.1);
    for y in 0..size.1 {
        let yoff = y * size.0;
        for x in 0..size.0 {
            let h = vec_get_safe(hmap, x + yoff);
            let mut hmin = h;
            let mut flow_dir = 0;
            for i in 1..9 {
                let ix = (x as i32 + DIRX[i]) as usize;
                let iy = (y as i32 + DIRY[i]) as usize;
                if ix < size.0 && iy < size.1 {
                    let h2 = vec_get_safe(hmap, ix + iy * size.0);
                    if h2 < hmin {
                        hmin = h2;
                        flow_dir = i;
                    }
                }
            }
            flow.push(flow_dir);
            let mut slope = hmin - h; // this is negative
            slope *= DIRCOEF[flow_dir];
            slopes.push(slope);
        }
    }
    (flow, slopes)
}

/** precipitations 0.0-1.0 */
pub fn compute_precipitations(size: (usize, usize), hmap: &Vec<f32>) -> Vec<f32> {
    const WATER_ADD: f32 = 0.03;
    const SLOPE_COEF: f32 = 2.0;
    const BASE_PRECIP: f32 = 0.01;
    let mut fbm = Fbm::new();
    fbm.octaves = 3;
    let mut prec = vec![0.0; size.0 * size.1];
    // north/south winds
    for dir in 0i32..2 {
        let diry = (dir * 2) - 1; // -1 and 1
        for x in 0..size.0 {
            let noval_sex = (x * 5) as f64 / size.0 as f64;
            let mut water_amount = (1.0 + fbm.get([noval_sex, 3.0])) as f32;
            let starty = if diry == -1 { size.1 as i32 - 1 } else { 0 };
            let endy = if diry == -1 { -1 } else { size.1 as i32 };
            let mut y = starty;
            while y != endy {
                let h = vec_get_safe(hmap, x + size.0 * y as usize);
                if h < WATER_LEVEL {
                    water_amount += WATER_ADD;
                } else if water_amount > 0.0 {
                    let slope = if ((y + diry) as usize) < size.1 {
                        vec_get_safe(hmap, x + size.0 * (y + diry) as usize) - h
                    } else {
                        h - vec_get_safe(hmap, x + size.0 * (y - diry) as usize)
                    };
                    if slope >= 0.0 {
                        let precip = water_amount * (BASE_PRECIP + slope * SLOPE_COEF);
                        prec[x + y as usize * size.0] += precip;
                        water_amount = (water_amount - precip).max(0.0);
                    }
                }
                y += diry;
            }
        }
    }
    // east/west winds
    for dir in 0i32..2 {
        let dirx = (dir * 2) - 1; // -1 and 1
        for y in 0..size.1 {
            let noval_sey = (y * 5) as f64 / size.1 as f64;
            let mut water_amount = (1.0 + fbm.get([noval_sey, 3.0])) as f32;
            let startx = if dirx == -1 { size.0 as i32 - 1 } else { 0 };
            let endx = if dirx == -1 { -1 } else { size.0 as i32 };
            let mut x = startx;
            while x != endx {
                let h = vec_get_safe(hmap, x as usize + y * size.0);
                if h < WATER_LEVEL {
                    water_amount += WATER_ADD;
                } else if water_amount > 0.0 {
                    let slope = if ((x + dirx) as usize) < size.0 {
                        vec_get_safe(hmap, (x + dirx) as usize + y * size.0) - h
                    } else {
                        h - vec_get_safe(hmap, (x - dirx) as usize + y * size.0)
                    };
                    if slope >= 0.0 {
                        let precip = water_amount * (BASE_PRECIP + slope * SLOPE_COEF);
                        prec[x as usize + y * size.0] += precip;
                        water_amount = (water_amount - precip).max(0.0);
                    }
                }
                x += dirx;
            }
        }
    }
    let (min, max) = get_min_max(&prec);
    // latitude impact
    for y in size.1 / 4..size.1 * 3 / 4 {
        // latitude (0 : equator, -1/1 : pole)
        let lat = (y - size.1 / 4) as f32 * 2.0 / size.1 as f32;
        let coef = (2.0 * 3.1415926 * lat).sin();
        for x in 0..size.0 {
            let xcoef =
                coef + 0.5 * fbm.get([x as f64 / size.0 as f64, y as f64 / size.1 as f64]) as f32;
            prec[x + y * size.0] += (max - min) * xcoef * 0.1;
        }
    }
    // blur
    blur(&mut prec, size);
    normalize(&mut prec, 0.0, 1.0);
    prec
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
