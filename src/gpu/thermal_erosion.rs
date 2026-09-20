//! GPU twin of `generators::thermal_erosion::gen_thermal_erosion`: the slide pass as a gather
//! kernel, every pass one dispatch over the working grid through `run_ping_pong`; the resampling
//! around it stays on the CPU.

use std::time::Instant;

use bytemuck::{Pod, Zeroable};

use super::{GpuContext, GpuError};
use crate::generators::{
    add_upsampled, downsample, thermal_plan, Progress, ThermalErosionConf, ThermalParams,
};
use crate::log;

pub(super) const THERMAL_WGSL: &str = include_str!("thermal.wgsl");

/// binding 1 of `thermal.wgsl`
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ThermalGpuParams {
    threshold: f32,
    diag_threshold: f32,
    strength: f32,
    water_level: f32,
}

pub(super) fn thermal_gpu_params(p: &ThermalParams) -> ThermalGpuParams {
    ThermalGpuParams {
        threshold: p.threshold,
        diag_threshold: p.diag_threshold,
        strength: p.strength,
        water_level: p.water_level,
    }
}

/// `hmap` is written last and only on success, so every `Err` leaves it for the CPU fallback
pub fn gen_thermal_erosion_gpu(
    gpu: &GpuContext,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &ThermalErosionConf,
    progress: &mut Progress,
) -> Result<(), GpuError> {
    let start = Instant::now();
    let (work, params) = thermal_plan(size, conf);
    let small = if work == size {
        hmap.to_vec()
    } else {
        downsample(hmap, size, work)
    };
    let mut eroded = small.clone();
    let done = gpu.run_ping_pong(
        "thermal",
        THERMAL_WGSL,
        bytemuck::bytes_of(&thermal_gpu_params(&params)),
        work,
        &mut eroded,
        params.passes,
        (0.0, 1.0),
        progress,
    )?;
    if !done {
        return Ok(());
    }
    if work == size {
        hmap.copy_from_slice(&eroded);
    } else {
        for (delta, initial) in eroded.iter_mut().zip(small.iter()) {
            *delta -= initial;
        }
        add_upsampled(hmap, size, &eroded, work);
    }
    log(&format!(
        "gpu=>thermal {}x{} {} pass(es) {} ms",
        work.0,
        work.1,
        params.passes,
        start.elapsed().as_millis()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::calib::stock_map;
    use crate::generators::gen_thermal_erosion;
    use crate::gpu::test_context;

    const TOLERANCE: f32 = 1e-4;

    fn second_conf() -> ThermalErosionConf {
        ThermalErosionConf {
            talus: 0.1,
            strength: 0.9,
            iterations: 20,
            water_level: 0.3,
            work_res: 512,
        }
    }

    /// 16×16 map, left half at 0.0, right half at 1.0
    fn cliff() -> Vec<f32> {
        (0..256)
            .map(|i| if i % 16 < 8 { 0.0 } else { 1.0 })
            .collect()
    }

    /// the largest |a - b| and the cell where it occurs
    fn max_diff(a: &[f32], b: &[f32]) -> (f32, usize) {
        a.iter()
            .zip(b)
            .enumerate()
            .map(|(i, (x, y))| ((x - y).abs(), i))
            .fold((0.0, 0), |m, d| if d.0 > m.0 { d } else { m })
    }

    /// runs both backends on `input` and asserts they agree within `TOLERANCE`; whether the
    /// step changed the map
    fn assert_agrees(
        gpu: &GpuContext,
        name: &str,
        size: (usize, usize),
        input: &[f32],
        conf: &ThermalErosionConf,
    ) -> bool {
        let mut cpu = input.to_vec();
        let mut on_gpu = input.to_vec();
        gen_thermal_erosion(size, &mut cpu, conf, &mut Progress::headless());
        gen_thermal_erosion_gpu(gpu, size, &mut on_gpu, conf, &mut Progress::headless()).unwrap();
        let (diff, i) = max_diff(&cpu, &on_gpu);
        eprintln!("{name}: max |cpu - gpu| = {diff}");
        assert!(
            diff <= TOLERANCE,
            "{name}: max |cpu - gpu| = {diff} at cell ({}, {}): cpu {} gpu {}",
            i % size.0,
            i / size.0,
            cpu[i],
            on_gpu[i]
        );
        cpu != input
    }

    #[test]
    fn thermal_gpu_is_deterministic() {
        let Some(gpu) = test_context() else { return };
        let input = stock_map(5, (64, 64));
        let conf = ThermalErosionConf::default();
        let mut a = input.clone();
        let mut b = input;
        gen_thermal_erosion_gpu(&gpu, (64, 64), &mut a, &conf, &mut Progress::headless()).unwrap();
        gen_thermal_erosion_gpu(&gpu, (64, 64), &mut b, &conf, &mut Progress::headless()).unwrap();
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn thermal_gpu_matches_cpu() {
        let Some(gpu) = test_context() else { return };
        let input = stock_map(5, (64, 64));
        assert!(assert_agrees(
            &gpu,
            "default",
            (64, 64),
            &input,
            &ThermalErosionConf::default()
        ));
        // the second conf's talus is above every slope of a 64×64 stock map: agreement only
        assert_agrees(&gpu, "second", (64, 64), &input, &second_conf());
    }

    #[test]
    fn thermal_gpu_works_on_the_working_grid() {
        let Some(gpu) = test_context() else { return };
        let input = stock_map(5, (64, 64));
        let conf = ThermalErosionConf {
            work_res: 16,
            ..Default::default()
        };
        assert!(assert_agrees(&gpu, "work_res 16", (64, 64), &input, &conf));
    }

    #[test]
    fn thermal_gpu_cancel_leaves_map_untouched() {
        let Some(gpu) = test_context() else { return };
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut progress = Progress::preview(tx, 1.0, || true);
        let input = stock_map(5, (64, 64));
        let mut h = input.clone();
        let res = gen_thermal_erosion_gpu(
            &gpu,
            (64, 64),
            &mut h,
            &ThermalErosionConf::default(),
            &mut progress,
        );
        assert!(res.is_ok());
        assert_eq!(h, input);
    }

    #[test]
    fn thermal_gpu_conserves_mass() {
        let Some(gpu) = test_context() else { return };
        let input = cliff();
        let conf = ThermalErosionConf {
            work_res: 16,
            water_level: 0.0,
            ..Default::default()
        };
        let mut h = input.clone();
        gen_thermal_erosion_gpu(&gpu, (16, 16), &mut h, &conf, &mut Progress::headless()).unwrap();
        let before: f32 = input.iter().sum();
        let after: f32 = h.iter().sum();
        assert!(h != input, "the cliff did not crumble");
        assert!(
            (before - after).abs() < 1e-3,
            "mass {before} became {after}"
        );
    }
}
