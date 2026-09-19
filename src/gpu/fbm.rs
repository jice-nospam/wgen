//! GPU twin of `generators::fbm::gen_fbm`: the same cell expression as a per-pixel kernel over
//! `noise.wgsl`, fed the `noise` crate's own permutation tables.

use bytemuck::{Pod, Zeroable};
use noise::permutationtable::{NoiseHasher, PermutationTable};

use super::{GpuContext, GpuError, BAND_CELLS};
use crate::generators::{FbmConf, Progress};

const FBM_WGSL: &str = concat!(include_str!("noise.wgsl"), include_str!("fbm.wgsl"));
const MAX_OCTAVES: usize = 32;

/// binding 1 of `fbm.wgsl`
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FbmParams {
    xcoef: f32,
    ycoef: f32,
    addx: f32,
    addy: f32,
    delta: f32,
    scale: f32,
    scale_factor: f32,
    width_f: f32,
    height_f: f32,
    octaves: u32,
    _pad: [u32; 2],
}

/// `octaves` permutation tables of 256 entries, table `i` being the crate's
/// `PermutationTable::new(seed + i)` — the per-octave sources `Fbm::new(seed)` builds
pub fn perm_tables(seed: u32, octaves: usize) -> Vec<u32> {
    (0..octaves)
        .flat_map(|i| {
            let t = PermutationTable::new(seed.wrapping_add(i as u32));
            (0..256).map(move |j| t.hash(&[j as isize]) as u32)
        })
        .collect()
}

pub fn gen_fbm_gpu(
    gpu: &GpuContext,
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &FbmConf,
    progress: &mut Progress,
) -> Result<(), GpuError> {
    gen_fbm_gpu_banded(gpu, seed, size, hmap, conf, BAND_CELLS, progress)
}

/// `gen_fbm_gpu` with an explicit band size (tests force several bands on a small map)
pub(crate) fn gen_fbm_gpu_banded(
    gpu: &GpuContext,
    seed: u64,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &FbmConf,
    cells_per_band: usize,
    progress: &mut Progress,
) -> Result<(), GpuError> {
    let octaves = (conf.octaves as usize).clamp(1, MAX_OCTAVES);
    let denom: f64 = (1..=octaves).map(|k| 0.5f64.powi(k as i32)).sum();
    let params = FbmParams {
        xcoef: conf.mulx / 400.0,
        ycoef: conf.muly / 400.0,
        addx: conf.addx,
        addy: conf.addy,
        delta: conf.delta,
        scale: conf.scale,
        scale_factor: (1.0 / denom) as f32,
        width_f: size.0 as f32,
        height_f: size.1 as f32,
        octaves: octaves as u32,
        _pad: [0; 2],
    };
    let table = perm_tables(seed as u32, octaves);
    gpu.run_per_pixel(
        "fbm",
        FBM_WGSL,
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
    use crate::generators::gen_fbm;
    use crate::gpu::test_context;

    fn second_conf() -> FbmConf {
        FbmConf {
            mulx: 10.0,
            muly: 4.0,
            addx: 50.0,
            addy: 20.0,
            octaves: 3.0,
            delta: 0.0,
            scale: 1.0,
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
    fn perm_tables_match_noise_crate() {
        let seed = 12345u32;
        let tables = perm_tables(seed, 3);
        assert_eq!(tables.len(), 3 * 256);
        let reference = PermutationTable::new(seed);
        for i in 0..256 {
            assert_eq!(
                tables[i] as usize,
                reference.hash(&[i as isize]),
                "entry {i}"
            );
        }
        for t in 0..3 {
            let mut seen = [false; 256];
            for &v in &tables[t * 256..(t + 1) * 256] {
                assert!(
                    v < 256 && !seen[v as usize],
                    "table {t} is not a permutation"
                );
                seen[v as usize] = true;
            }
        }
    }

    #[test]
    fn fbm_gpu_is_deterministic() {
        let Some(gpu) = test_context() else { return };
        let conf = FbmConf::default();
        let mut a = vec![0.0; 64 * 64];
        let mut b = vec![0.0; 64 * 64];
        gen_fbm_gpu(&gpu, 7, (64, 64), &mut a, &conf, &mut Progress::headless()).unwrap();
        gen_fbm_gpu(&gpu, 7, (64, 64), &mut b, &conf, &mut Progress::headless()).unwrap();
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn fbm_gpu_matches_cpu() {
        let Some(gpu) = test_context() else { return };
        for (name, conf) in [("default", FbmConf::default()), ("second", second_conf())] {
            let size = (64, 64);
            let mut cpu = vec![0.25; 64 * 64];
            let mut gpu_map = cpu.clone();
            gen_fbm(0xdeadbeef, size, &mut cpu, &conf, &mut Progress::headless());
            gen_fbm_gpu(
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
    fn fbm_gpu_is_band_independent() {
        let Some(gpu) = test_context() else { return };
        let conf = FbmConf::default();
        let mut one = vec![0.0; 64 * 64];
        let mut four = vec![0.0; 64 * 64];
        gen_fbm_gpu_banded(
            &gpu,
            3,
            (64, 64),
            &mut one,
            &conf,
            64 * 64,
            &mut Progress::headless(),
        )
        .unwrap();
        gen_fbm_gpu_banded(
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
