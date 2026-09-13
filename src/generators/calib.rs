//! Calibration harness, test-only : renders a generator's effect on a stock map to PNGs under
//! `target/calib` so a change is judged on a picture, never on a launch. The ignored tests are
//! the renders; the others pin the helpers.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{gen_fbm, gen_fluvial_erosion, normalize, FbmConf, FluvialErosionConf, Progress};

/// the "Fbm → Normalize" stack : `gen_fbm` with its defaults, normalized to 0..1
pub fn stock_map(seed: u64, size: (usize, usize)) -> Vec<f32> {
    let mut h = vec![0.0; size.0 * size.1];
    gen_fbm(seed, size, &mut h, &FbmConf::default(), &mut Progress::headless());
    normalize(&mut h, 0.0, 1.0);
    h
}

/// `target/calib` under the crate, created; gitignored through `/target`
pub fn calib_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/calib");
    std::fs::create_dir_all(&dir).expect("cannot create target/calib");
    dir
}

/// `max - min` of a map, 1.0 for a flat one so a gain divided by it stays finite
pub fn range(h: &[f32]) -> f32 {
    let (min, max) = super::get_min_max(h);
    if max > min {
        max - min
    } else {
        1.0
    }
}

/// 8-bit grayscale, min/max stretched
pub fn write_height_png(path: &Path, size: (usize, usize), h: &[f32]) {
    let (min, _) = super::get_min_max(h);
    let scale = 255.0 / range(h);
    let bytes: Vec<u8> = h
        .iter()
        .map(|v| ((v - min) * scale).round() as u8)
        .collect();
    save_gray(path, size, &bytes);
}

/// 8-bit hillshade, light from the top-left : `0.5 + gain · (dh/dx + dh/dy)` over the two
/// neighbours on each axis, clamped; edge cells copy their inner neighbour
pub fn hillshade_bytes(size: (usize, usize), h: &[f32], gain: f32) -> Vec<u8> {
    let (w, rows) = size;
    let mut out = vec![128u8; w * rows];
    for y in 1..rows - 1 {
        for x in 1..w - 1 {
            let dx = h[x - 1 + y * w] - h[x + 1 + y * w];
            let dy = h[x + (y - 1) * w] - h[x + (y + 1) * w];
            let shade = (0.5 + gain * (dx + dy)).clamp(0.0, 1.0);
            out[x + y * w] = (shade * 255.0).round() as u8;
        }
        out[y * w] = out[1 + y * w];
        out[w - 1 + y * w] = out[w - 2 + y * w];
    }
    for x in 0..w {
        out[x] = out[x + w];
        out[x + (rows - 1) * w] = out[x + (rows - 2) * w];
    }
    out
}

/// hillshade gain that puts the 95th percentile of `|dh/dx + dh/dy|` at the edge of the
/// shade range, so a map's own relief sets the contrast
pub fn shade_gain(size: (usize, usize), h: &[f32]) -> f32 {
    let (w, rows) = size;
    let mut grads = Vec::with_capacity(w * rows);
    for y in 1..rows - 1 {
        for x in 1..w - 1 {
            let dx = h[x - 1 + y * w] - h[x + 1 + y * w];
            let dy = h[x + (y - 1) * w] - h[x + (y + 1) * w];
            grads.push((dx + dy).abs());
        }
    }
    grads.sort_by(|a, b| a.total_cmp(b));
    let p95 = grads[grads.len() * 95 / 100];
    if p95 > 0.0 {
        0.45 / p95
    } else {
        1.0
    }
}

pub fn write_hillshade_png(path: &Path, size: (usize, usize), h: &[f32], gain: f32) {
    save_gray(path, size, &hillshade_bytes(size, h, gain));
}

fn save_gray(path: &Path, size: (usize, usize), bytes: &[u8]) {
    image::save_buffer(
        path,
        bytes,
        size.0 as u32,
        size.1 as u32,
        image::ColorType::L8,
    )
    .unwrap_or_else(|e| panic!("cannot write {} : {e}", path.display()));
}

/// what one erosion did to a map, in the three numbers the calibration bands are set on
pub struct Stats {
    /// largest `input - out`
    pub max_drop: f32,
    /// mean `input - out`
    pub mean_drop: f32,
    /// share of cells lowered by more than 0.01
    pub carved: f32,
}

/// prints and returns the stats of `out` against `input`
pub fn print_stats(label: &str, input: &[f32], out: &[f32]) -> Stats {
    let drops: Vec<f32> = input.iter().zip(out.iter()).map(|(i, o)| i - o).collect();
    let stats = Stats {
        max_drop: drops.iter().cloned().fold(0.0f32, f32::max),
        mean_drop: drops.iter().sum::<f32>() / drops.len() as f32,
        carved: drops.iter().filter(|d| **d > 0.01).count() as f32 / drops.len() as f32,
    };
    println!(
        "{label} : max drop {:.4}, mean drop {:.4}, cells carved > 0.01 : {:.1} %",
        stats.max_drop,
        stats.mean_drop,
        stats.carved * 100.0
    );
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_map_is_normalized() {
        let h = stock_map(1234, (32, 32));
        let (min, max) = super::super::get_min_max(&h);
        assert!(min.abs() < 1e-6, "min {min}");
        assert!((max - 1.0).abs() < 1e-6, "max {max}");
    }

    #[test]
    fn hillshade_of_a_flat_map_is_mid_gray() {
        let flat = vec![0.3; 256];
        let bytes = hillshade_bytes((16, 16), &flat, 60.0);
        assert_eq!(bytes.len(), 256);
        assert!(bytes.iter().all(|b| *b == 127 || *b == 128));
    }

    #[test]
    fn shade_gain_puts_a_ramp_at_the_edge_of_the_range() {
        let ramp: Vec<f32> = (0..256).map(|i| (i % 16) as f32 * 0.01).collect();
        let gain = shade_gain((16, 16), &ramp);
        assert!((gain - 0.45 / 0.02).abs() < 1e-3, "gain {gain}");
        let flat = vec![0.3; 256];
        assert_eq!(shade_gain((16, 16), &flat), 1.0);
    }

    #[test]
    fn height_png_writes_a_file() {
        let h: Vec<f32> = (0..256).map(|i| i as f32).collect();
        let path = calib_dir().join("height_png_writes_a_file.png");
        write_height_png(&path, (16, 16), &h);
        assert!(path.exists());
        std::fs::remove_file(&path).unwrap();
    }

    /// the stock stack "Fbm → Normalize → FluvialErosion" with the defaults, at 512 and 256 :
    /// `cargo test --ignored render_fluvial_stock_stack -- --nocapture`
    #[test]
    #[ignore]
    fn render_fluvial_stock_stack() {
        let dir = calib_dir();
        for (side, work_res) in [(512usize, 512u32), (256, 256)] {
            let size = (side, side);
            let input = stock_map(1234, size);
            let gain = shade_gain(size, &input);
            if side == 512 {
                write_hillshade_png(&dir.join("fluvial_input_shade.png"), size, &input, gain);
            }
            let conf = FluvialErosionConf {
                work_res,
                ..Default::default()
            };
            let start = Instant::now();
            let mut out = input.clone();
            gen_fluvial_erosion(size, &mut out, &conf, &mut Progress::headless());
            let elapsed = start.elapsed();
            let shade = dir.join(format!("fluvial_{side}_shade.png"));
            write_hillshade_png(&shade, size, &out, gain);
            println!("{}", shade.display());
            if side == 512 {
                let height = dir.join("fluvial_512_height.png");
                write_height_png(&height, size, &out);
                println!("{}", height.display());
            }
            print_stats(&format!("fluvial defaults on fbm {side}"), &input, &out);
            println!("fluvial {side} : {:.2} s", elapsed.as_secs_f32());
        }
    }
}
