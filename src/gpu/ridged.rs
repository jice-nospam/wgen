//! GPU twin of `generators::ridged::gen_ridged`: the same cell expression as a per-pixel
//! kernel over `noise.wgsl`, with the ridge stream's tables followed by the fold's two streams.

use bytemuck::{Pod, Zeroable};

use super::fbm::{fbm_scale_factor, stream_tables};
use super::{GpuContext, GpuError, BAND_CELLS};
use crate::generators::{
    Progress, RidgedConf, FOLD_OCTAVES, FOLD_STREAM_A, FOLD_STREAM_B, RIDGED_MAX_OCTAVES,
};

const RIDGED_WGSL: &str = concat!(include_str!("noise.wgsl"), include_str!("ridged.wgsl"));

/// binding 1 of `ridged.wgsl`
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RidgedParams {
    coef: f32,
    offset_x: f32,
    offset_y: f32,
    scale: f32,
    scale_factor: f32,
    fold_px: f32,
    fold_coef: f32,
    fold_scale: f32,
    width_f: f32,
    height_f: f32,
    octaves: u32,
    fold_octaves: u32,
    fold_first_a: u32,
    fold_first_b: u32,
    _pad: [u32; 2],
}

/// the crate's `RidgedMulti::calc_scale_factor(1.0, 2.0, octaves)`, which sums one weighted
/// term more than the octave count — mirrored as is, so the GPU scales like the CPU
pub(crate) fn ridged_scale_factor(octaves: usize) -> f32 {
    let mut denom = 1.0f64;
    let mut signal = 1.0f64;
    for x in 1..=octaves {
        let weight = (signal / 2.0f64.powi(x as i32)).clamp(0.0, 1.0);
        signal = weight;
        denom += signal;
    }
    (2.0 / denom) as f32
}

pub fn gen_ridged_gpu(
    gpu: &GpuContext,
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &RidgedConf,
    progress: &mut Progress,
) -> Result<(), GpuError> {
    gen_ridged_gpu_banded(gpu, seed, size, hmap, conf, BAND_CELLS, progress)
}

/// `gen_ridged_gpu` with an explicit band size (tests force several bands on a small map)
pub(crate) fn gen_ridged_gpu_banded(
    gpu: &GpuContext,
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &RidgedConf,
    cells_per_band: usize,
    progress: &mut Progress,
) -> Result<(), GpuError> {
    let octaves = (conf.octaves as usize).clamp(1, RIDGED_MAX_OCTAVES);
    let params = RidgedParams {
        coef: conf.zoom / 400.0,
        offset_x: conf.offset_x,
        offset_y: conf.offset_y,
        scale: conf.scale,
        scale_factor: ridged_scale_factor(octaves),
        fold_px: conf.fold / 100.0 * 512.0,
        fold_coef: conf.fold_zoom / 400.0,
        fold_scale: fbm_scale_factor(FOLD_OCTAVES),
        width_f: size.0 as f32,
        height_f: size.1 as f32,
        octaves: octaves as u32,
        fold_octaves: FOLD_OCTAVES as u32,
        fold_first_a: octaves as u32,
        fold_first_b: (octaves + FOLD_OCTAVES) as u32,
        _pad: [0; 2],
    };
    let table = stream_tables(
        seed,
        &[
            (0, octaves),
            (FOLD_STREAM_A, FOLD_OCTAVES),
            (FOLD_STREAM_B, FOLD_OCTAVES),
        ],
    );
    gpu.run_per_pixel(
        "ridged",
        RIDGED_WGSL,
        bytemuck::bytes_of(&params),
        &table,
        size,
        hmap,
        cells_per_band,
        progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::gen_ridged;
    use crate::gpu::test_context;

    fn second_conf() -> RidgedConf {
        RidgedConf {
            zoom: 5.0,
            octaves: 3.0,
            scale: 0.7,
            fold: 10.0,
            fold_zoom: 2.0,
            offset_x: 30.0,
            offset_y: 70.0,
        }
    }

    /// the largest |a - b| and the cell where it occurs
    fn max_diff(a: &[f32], b: &[f32]) -> (f32, usize) {
        a.iter()
            .zip(b)
            .enumerate()
            .map(|(i, (x, y))| ((x - y).abs(), i))
            .fold((0.0, 0), |m, d| if d.0 > m.0 { d } else { m })
    }

    #[test]
    fn ridged_scale_factor_matches_crate() {
        // the crate's `RidgedMulti::get` of a point whose every octave reads 0 is
        // `scale_factor · Σ weights − 1`; the same sum on the CPU reproduces it
        use noise::{MultiFractal, NoiseFn, Perlin, RidgedMulti};
        for octaves in [1usize, 3, 6] {
            let ridged = RidgedMulti::<Perlin>::new(0).set_octaves(octaves);
            // an integer lattice point: every Perlin octave is exactly 0 there
            let r = ridged.get([0.0, 0.0]);
            let mut sum = 0.0f64;
            let mut weight = 1.0f64;
            for _ in 0..octaves {
                let s = weight;
                weight = (s / 2.0).clamp(0.0, 1.0);
                sum += s;
            }
            let expected = sum * ridged_scale_factor(octaves) as f64 - 1.0;
            assert!(
                (r - expected).abs() < 1e-6,
                "{octaves} octaves: crate {r} vs {expected}"
            );
        }
    }

    #[test]
    fn ridged_gpu_is_deterministic() {
        let Some(gpu) = test_context() else { return };
        let conf = RidgedConf::default();
        let mut a = vec![0.0; 64 * 64];
        let mut b = vec![0.0; 64 * 64];
        gen_ridged_gpu(&gpu, 7, (64, 64), &mut a, &conf, &mut Progress::headless()).unwrap();
        gen_ridged_gpu(&gpu, 7, (64, 64), &mut b, &conf, &mut Progress::headless()).unwrap();
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn ridged_gpu_matches_cpu() {
        let Some(gpu) = test_context() else { return };
        for (name, conf) in [
            ("default", RidgedConf::default()),
            ("second", second_conf()),
        ] {
            let size = (64, 64);
            let mut cpu = vec![0.25; 64 * 64];
            let mut gpu_map = cpu.clone();
            gen_ridged(0xdeadbeef, size, &mut cpu, &conf, &mut Progress::headless());
            gen_ridged_gpu(
                &gpu,
                0xdeadbeef,
                size,
                &mut gpu_map,
                &conf,
                &mut Progress::headless(),
            )
            .unwrap();
            let (diff, i) = max_diff(&cpu, &gpu_map);
            eprintln!("{name}: max |cpu - gpu| = {diff}");
            assert!(
                diff <= 2e-3,
                "{name}: max |cpu - gpu| = {diff} at cell ({}, {}): cpu {} gpu {}",
                i % 64,
                i / 64,
                cpu[i],
                gpu_map[i]
            );
        }
    }

    #[test]
    fn ridged_gpu_is_band_independent() {
        let Some(gpu) = test_context() else { return };
        let conf = second_conf();
        let mut one = vec![0.0; 64 * 64];
        let mut four = vec![0.0; 64 * 64];
        gen_ridged_gpu_banded(
            &gpu,
            3,
            (64, 64),
            &mut one,
            &conf,
            64 * 64,
            &mut Progress::headless(),
        )
        .unwrap();
        gen_ridged_gpu_banded(
            &gpu,
            3,
            (64, 64),
            &mut four,
            &conf,
            64 * 16,
            &mut Progress::headless(),
        )
        .unwrap();
        if let Some(y) = (0..64).find(|&y| one[y * 64..(y + 1) * 64] != four[y * 64..(y + 1) * 64])
        {
            panic!("row {y} differs between one band and four bands");
        }
    }
}
