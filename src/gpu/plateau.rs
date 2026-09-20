//! GPU twin of `generators::plateau::gen_plateau`: the same cell expression as a per-pixel
//! kernel over `noise.wgsl` and `terrace.wgsl`, with `min`/`range` taken from the CPU pass so
//! both backends quantise against the same bounds.

use bytemuck::{Pod, Zeroable};

use super::fbm::{fbm_scale_factor, stream_tables};
use super::{GpuContext, GpuError, BAND_CELLS};
use crate::generators::noise_field::noise_coef;
use crate::generators::{get_min_max, PlateauConf, Progress, PLATEAU_JITTER_OCTAVES};

const PLATEAU_WGSL: &str = concat!(
    include_str!("noise.wgsl"),
    include_str!("terrace.wgsl"),
    include_str!("plateau.wgsl")
);

/// binding 1 of `plateau.wgsl`
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PlateauParams {
    min: f32,
    range: f32,
    levels: f32,
    flat: f32,
    rounding: f32,
    jitter: f32,
    jitter_coef: f32,
    jitter_scale: f32,
    width_f: f32,
    height_f: f32,
    octaves: u32,
    _pad: u32,
}

pub fn gen_plateau_gpu(
    gpu: &GpuContext,
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &PlateauConf,
    progress: &mut Progress,
) -> Result<(), GpuError> {
    gen_plateau_gpu_banded(gpu, seed, size, hmap, conf, BAND_CELLS, progress)
}

/// `gen_plateau_gpu` with an explicit band size (tests force several bands on a small map)
pub(crate) fn gen_plateau_gpu_banded(
    gpu: &GpuContext,
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &PlateauConf,
    cells_per_band: usize,
    progress: &mut Progress,
) -> Result<(), GpuError> {
    let (min, max) = get_min_max(hmap);
    let range = max - min;
    if range <= 0.0 {
        return Ok(());
    }
    let params = PlateauParams {
        min,
        range,
        levels: conf.levels as f32,
        flat: conf.flat.min(0.95),
        rounding: conf.rounding,
        jitter: conf.jitter,
        jitter_coef: noise_coef(conf.jitter_zoom),
        jitter_scale: fbm_scale_factor(PLATEAU_JITTER_OCTAVES),
        width_f: size.0 as f32,
        height_f: size.1 as f32,
        octaves: PLATEAU_JITTER_OCTAVES as u32,
        _pad: 0,
    };
    let table = stream_tables(seed, &[(0, PLATEAU_JITTER_OCTAVES)]);
    gpu.run_per_pixel(
        "plateau",
        PLATEAU_WGSL,
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
    use crate::generators::calib::stock_map;
    use crate::generators::gen_plateau;
    use crate::gpu::test_context;

    fn second_conf() -> PlateauConf {
        PlateauConf {
            levels: 12,
            flat: 0.9,
            rounding: 0.0,
            jitter: 0.8,
            jitter_zoom: 10.0,
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
    fn plateau_gpu_is_deterministic() {
        let Some(gpu) = test_context() else { return };
        let conf = PlateauConf::default();
        let base = stock_map(9, (64, 64));
        let mut a = base.clone();
        let mut b = base;
        gen_plateau_gpu(&gpu, 7, (64, 64), &mut a, &conf, &mut Progress::headless()).unwrap();
        gen_plateau_gpu(&gpu, 7, (64, 64), &mut b, &conf, &mut Progress::headless()).unwrap();
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn plateau_gpu_matches_cpu() {
        let Some(gpu) = test_context() else { return };
        for (name, conf) in [
            ("default", PlateauConf::default()),
            ("second", second_conf()),
        ] {
            let size = (64, 64);
            let base = stock_map(9, size);
            let mut cpu = base.clone();
            let mut gpu_map = base;
            gen_plateau(0xdeadbeef, size, &mut cpu, &conf, &mut Progress::headless());
            gen_plateau_gpu(
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
                diff <= 5e-4,
                "{name}: max |cpu - gpu| = {diff} at cell ({}, {}): cpu {} gpu {}",
                i % 64,
                i / 64,
                cpu[i],
                gpu_map[i]
            );
        }
    }

    #[test]
    fn plateau_gpu_is_band_independent() {
        let Some(gpu) = test_context() else { return };
        let conf = second_conf();
        let base = stock_map(3, (64, 64));
        let mut one = base.clone();
        let mut four = base;
        gen_plateau_gpu_banded(
            &gpu,
            3,
            (64, 64),
            &mut one,
            &conf,
            64 * 64,
            &mut Progress::headless(),
        )
        .unwrap();
        gen_plateau_gpu_banded(
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
