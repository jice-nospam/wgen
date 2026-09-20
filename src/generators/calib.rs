//! Calibration harness, test-only : renders a generator's effect on a stock map to PNGs under
//! `target/calib` so a change is judged on a picture, never on a launch. The ignored tests are
//! the renders; the others pin the helpers.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{gen_fbm, gen_fluvial_erosion, normalize, FbmConf, FluvialErosionConf, Progress};

/// the "Fbm → Normalize" stack : `gen_fbm` with its defaults, normalized to 0..1
pub fn stock_map(seed: u64, size: (usize, usize)) -> Vec<f32> {
    let mut h = vec![0.0; size.0 * size.1];
    gen_fbm(
        seed,
        size,
        &mut h,
        &FbmConf::default(),
        &mut Progress::headless(),
    );
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

    /// `cargo test generator_timing_report -- --ignored --nocapture`: every generator at export
    /// sizes, one line each; the numbers that decide which generator deserves a GPU twin
    #[test]
    #[ignore]
    fn generator_timing_report() {
        use super::super::*;
        fn time(label: &str, f: impl FnOnce()) {
            let start = Instant::now();
            f();
            println!(
                "{label:<44} {:>9.0} ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        let sides: Vec<usize> = std::env::var("WGEN_BENCH_SIDES")
            .map(|v| v.split(',').map(|s| s.parse().unwrap()).collect())
            .unwrap_or_else(|_| vec![4096, 8192]);
        let work_res: Vec<u32> = std::env::var("WGEN_BENCH_WORK_RES")
            .map(|v| v.split(',').map(|s| s.parse().unwrap()).collect())
            .unwrap_or_else(|_| vec![512, 2048]);
        for side in sides {
            let size = (side, side);
            let mut p = Progress::headless();
            let mut base = vec![0.0; side * side];
            time(&format!("fbm cpu {side}"), || {
                gen_fbm(1, size, &mut base, &FbmConf::default(), &mut p)
            });
            if let Some(gpu) = crate::gpu::test_context() {
                let mut h = vec![0.0; side * side];
                time(&format!("fbm gpu {side}"), || {
                    crate::gpu::fbm::gen_fbm_gpu(&gpu, 1, size, &mut h, &FbmConf::default(), &mut p)
                        .unwrap()
                });
            }
            normalize(&mut base, 0.0, 1.0);
            let mut h = vec![0.0; side * side];
            time(&format!("hills {side}"), || {
                gen_hills(1, size, &mut h, &HillsConf::default(), &mut p)
            });
            time(&format!("mid_point {side}"), || {
                gen_mid_point(1, size, &mut h, &MidPointConf::default(), &mut p)
            });
            h.copy_from_slice(&base);
            time(&format!("landmass {side}"), || {
                gen_landmass(size, &mut h, &LandMassConf::default(), &mut p)
            });
            time(&format!("island {side}"), || {
                gen_island(size, &mut h, &IslandConf::default(), &mut p)
            });
            time(&format!("normalize {side}"), || {
                gen_normalize(&mut h, &NormalizeConf::default())
            });
            time(&format!("mudslide {side}"), || {
                gen_mudslide(size, &mut h, &MudSlideConf::default(), &mut p)
            });
            h.copy_from_slice(&base);
            time(&format!("water_erosion {side} work 512"), || {
                gen_water_erosion(1, size, &mut h, &WaterErosionConf::default(), &mut p)
            });
            for &wr in &work_res {
                h.copy_from_slice(&base);
                let conf = ThermalErosionConf {
                    work_res: wr,
                    ..Default::default()
                };
                time(&format!("thermal {side} work {wr}"), || {
                    gen_thermal_erosion(size, &mut h, &conf, &mut p)
                });
                if let Some(gpu) = crate::gpu::test_context() {
                    h.copy_from_slice(&base);
                    time(&format!("thermal gpu {side} work {wr}"), || {
                        crate::gpu::thermal_erosion::gen_thermal_erosion_gpu(
                            &gpu, size, &mut h, &conf, &mut p,
                        )
                        .unwrap()
                    });
                }
                h.copy_from_slice(&base);
                let conf = FluvialErosionConf {
                    work_res: wr,
                    ..Default::default()
                };
                time(&format!("fluvial {side} work {wr}"), || {
                    gen_fluvial_erosion(size, &mut h, &conf, &mut p)
                });
                if let Some(gpu) = crate::gpu::test_context() {
                    h.copy_from_slice(&base);
                    time(&format!("fluvial gpu {side} work {wr}"), || {
                        crate::gpu::fluvial_erosion::gen_fluvial_erosion_gpu(
                            &gpu, size, &mut h, &conf, &mut p,
                        )
                        .unwrap()
                    });
                }
            }
        }
    }

    /// `cargo test project_timing_report -- --ignored --nocapture`: the stack of a `.wgen`
    /// project (`WGEN_PROJECT`, default `ex_continent.wgen`) at every side of
    /// `WGEN_BENCH_SIDES` (default 512,2048), on the CPU and on the GPU: one line per step, the
    /// difference between the two results, and a hillshade of each in `target/calib` — the
    /// command-line export (`cli.rs`) run from the test harness
    #[test]
    #[ignore]
    fn project_timing_report() {
        use crate::gpu::Backend;
        use crate::project::Project;
        use crate::worldgen::WorldGenerator;
        use crate::ThreadMessage;
        let path = std::env::var("WGEN_PROJECT").unwrap_or_else(|_| "ex_continent.wgen".into());
        let project = Project::load(&path).unwrap();
        let stem = Path::new(&path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let sides: Vec<usize> = std::env::var("WGEN_BENCH_SIDES")
            .map(|v| v.split(',').map(|s| s.parse().unwrap()).collect())
            .unwrap_or_else(|_| vec![512, 2048]);
        let mut backends = vec![("cpu", Backend::Cpu)];
        if let Some(gpu) = crate::gpu::test_context() {
            backends.push(("gpu", Backend::Gpu(gpu)));
        }
        let dir = calib_dir();
        for side in sides {
            let size = (side, side);
            let mut results: Vec<Vec<f32>> = Vec::new();
            for (name, backend) in &backends {
                let (tx, rx) = std::sync::mpsc::channel();
                let (steps, backend, seed) = (project.steps.clone(), backend.clone(), project.seed);
                let start = Instant::now();
                let worker = std::thread::spawn(move || {
                    let mut wgen = WorldGenerator::new(seed, size);
                    wgen.set_backend(backend);
                    wgen.generate(&steps, tx, 0.25);
                    wgen.get_export_map()
                });
                let mut step_start = Instant::now();
                while let Ok(msg) = rx.recv() {
                    if let ThreadMessage::ExporterStepDone(i) = msg {
                        println!(
                            "{stem} {side} {name} step {i} {:<20} {:>9.0} ms",
                            project.steps[i].typ.name(),
                            step_start.elapsed().as_secs_f64() * 1000.0
                        );
                        step_start = Instant::now();
                    }
                }
                let map = worker.join().unwrap();
                println!(
                    "{stem} {side} {name} total {:>28.0} ms",
                    start.elapsed().as_secs_f64() * 1000.0
                );
                let h = map.borrow();
                let gain = shade_gain(size, h);
                write_hillshade_png(
                    &dir.join(format!("{stem}_{side}_{name}_shade.png")),
                    size,
                    h,
                    gain,
                );
                results.push(h.clone());
            }
            if let [cpu, gpu] = results.as_slice() {
                let n = cpu.len() as f32;
                let range = range(cpu);
                let (mut sum, mut max) = (0.0f32, 0.0f32);
                for (a, b) in cpu.iter().zip(gpu) {
                    let d = (a - b).abs();
                    sum += d;
                    max = max.max(d);
                }
                println!(
                    "{stem} {side} cpu vs gpu: mean |d| {:.4} max {:.4} (map range {:.3})",
                    sum / n,
                    max,
                    range
                );
            }
        }
    }

    /// `cargo test phase_timing_report -- --ignored --nocapture`: the phases behind the slow
    /// lines of `generator_timing_report` (fluvial routing vs slide, MidPoint draws vs resample,
    /// Hills at heavy settings)
    #[test]
    #[ignore]
    fn phase_timing_report() {
        use super::super::thermal_erosion::{slide_pass, ThermalParams};
        use super::super::*;
        use rand::{rngs::StdRng, Rng, SeedableRng};
        fn time(label: &str, f: impl FnOnce()) {
            let start = Instant::now();
            f();
            println!(
                "{label:<44} {:>9.0} ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        let mut p = Progress::headless();
        for work in [512usize, 1024, 2048] {
            let size = (work, work);
            let h = stock_map(1, size);
            let mut net = FlowNet::new();
            time(&format!("fluvial route {work}"), || {
                net.route(size, &h, 0.0)
            });
            let params = ThermalParams::new(&ThermalErosionConf::default(), work as f32 / 512.0);
            let mut out = h.clone();
            time(&format!("thermal slide_pass {work}"), || {
                slide_pass(size, &h, &mut out, &params, 0.0, 1.0, &mut p);
            });
        }
        let side = 8192usize;
        let n = side + 1;
        time("mid_point 8192: 67M StdRng draws", || {
            let mut rng = StdRng::seed_from_u64(1);
            let mut acc = 0.0f32;
            for _ in 0..n * n {
                acc += rng.random_range(-0.5f32..0.5);
            }
            assert!(acc.is_finite());
        });
        let lattice: Vec<f32> = (0..n * n).map(|i| (i % 7) as f32).collect();
        let mut h = vec![0.0f32; side * side];
        time("mid_point 8192: lattice->map bilinear", || {
            let scale = side as f32 / side as f32;
            for y in 0..side {
                for x in 0..side {
                    h[x + y * side] =
                        bilinear(&lattice, x as f32 * scale, y as f32 * scale, (n, n));
                }
            }
        });
        for (nb_hill, base_radius) in [(2000usize, 40.0f32), (5000, 16.0), (600, 100.0)] {
            let conf = HillsConf {
                nb_hill,
                base_radius,
                ..Default::default()
            };
            h.fill(0.0);
            time(
                &format!("hills 8192 count {nb_hill} radius {base_radius}"),
                || gen_hills(1, (side, side), &mut h, &conf, &mut p),
            );
        }
    }
}
