//! GPU twin of `generators::fluvial_erosion::gen_fluvial_erosion`, rethought for the GPU: the
//! working grid stays resident in device buffers for the whole run and every phase of an
//! iteration is a kernel — depression fill (`fluvial_fill_init.wgsl` + `fluvial_fill.wgsl`, a
//! Planchon-Darboux relaxation in 32x32 tiles), D8 routing (`fluvial_route.wgsl`), drainage
//! area by pointer doubling with integer atomics (`fluvial_area_jump.wgsl` +
//! `fluvial_area_scatter.wgsl`), the implicit incision as repeated Jacobi sweeps
//! (`fluvial_incise.wgsl`) and the talus slides with the thermal kernel. Iterations are
//! submitted in chunks; the one readback per chunk carries the convergence flags that size the
//! next chunk's fill and accumulation. The CPU generator is the reference the landscape is
//! compared to, not reproduced (`work/GPU.md` §4 G-5).

use std::sync::atomic::Ordering;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::thermal_erosion::{thermal_gpu_params, THERMAL_WGSL};
use super::{Band, GpuContext, GpuError, Layout, WORKGROUP};
use crate::generators::{
    add_upsampled, downsample, fluvial_plan, get_min_max, FluvialErosionConf, FluvialParams,
    Progress, HILLSLOPE_AREA,
};
use crate::log;

const FILL_INIT_WGSL: &str = include_str!("fluvial_fill_init.wgsl");
const FILL_WGSL: &str = include_str!("fluvial_fill.wgsl");
const ROUTE_WGSL: &str = include_str!("fluvial_route.wgsl");
const AREA_JUMP_WGSL: &str = include_str!("fluvial_area_jump.wgsl");
const AREA_SCATTER_WGSL: &str = include_str!("fluvial_area_scatter.wgsl");
const INCISE_WGSL: &str = include_str!("fluvial_incise.wgsl");

/// side of the tile one fill workgroup relaxes (`fluvial_fill.wgsl`)
const FILL_TILE: u32 = 32;
/// the fill's `eps` per cell, as a fraction of the map's relief on a `REFERENCE_RES` map:
/// slopes below it route as flats, and a lake surface tilts by it per cell towards its spill
const FILL_EPS: f32 = 1e-4;
/// the map side the fill's `eps` is expressed for
const REFERENCE_RES: f32 = 512.0;
/// fewest fill dispatches an iteration runs, whatever the previous chunk needed
const FILL_MIN: usize = 2;
/// most fill dispatches a warm iteration runs: a region that converges from below (one `eps`
/// per sweep) is left to the next full fill instead
const WARM_FILL_CAP: usize = 8;
/// fill dispatches of the cold start's first submission; each next one doubles
const COLD_FILL_STEP: usize = 16;
/// fewest accumulation levels an iteration runs
const LEVELS_MIN: usize = 4;
/// most accumulation levels: `2^32` steps
const MAX_LEVELS: usize = 32;
/// incision sweeps: `INCISE_SWEEPS_BASE + INCISE_SWEEPS_PER_SCALE * ceil(scale)`
const INCISE_SWEEPS_BASE: usize = 8;
const INCISE_SWEEPS_PER_SCALE: usize = 4;
/// cells one submission is sized for: `CHUNK_CELLS / n` iterations per chunk (2 at 2048²)
const CHUNK_CELLS: usize = 8 << 20;
const MAX_CHUNK: usize = 16;
/// iterations of the first chunk after the cold start; each next chunk doubles up to the size
const FIRST_CHUNK: usize = 2;
/// a full (cold) fill every so many iterations, so the lakes a warm iteration leaves frozen
/// follow the terrain
const REFILL_EVERY: usize = 16;
/// params entries: the warm fill init, the cold one, one per cold fill dispatch (its flag
/// slot), one per warm fill dispatch (frozen lakes), then one per accumulation level
const SLOT_WARM: usize = 0;
const SLOT_COLD: usize = 1;
const SLOT_FILL: usize = 2;

/// binding 1 of every fluvial kernel; `slot` is the flag the dispatch sets, `cold` selects the
/// fill init's cold start
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FluvialGpuParams {
    k: f32,
    uplift: f32,
    water_level: f32,
    n_cells: f32,
    area_floor: f32,
    eps: f32,
    slot: u32,
    cold: u32,
    /// the fill leaves last iteration's lake cells untouched (warm dispatches)
    freeze: u32,
    _pad: [u32; 3],
}

/// `hmap` is written last and only on success, so every `Err` leaves it for the CPU fallback
pub fn gen_fluvial_erosion_gpu(
    gpu: &GpuContext,
    size: (usize, usize),
    hmap: &mut [f32],
    conf: &FluvialErosionConf,
    progress: &mut Progress,
) -> Result<(), GpuError> {
    let start = Instant::now();
    let (work, params) = fluvial_plan(size, conf);
    let small = if work == size {
        hmap.to_vec()
    } else {
        downsample(hmap, size, work)
    };
    let mut eroded = small.clone();
    let Some(stats) = gpu.erode_fluvial(work, &mut eroded, &params, progress)? else {
        return Ok(());
    };
    if work == size {
        hmap.copy_from_slice(&eroded);
    } else {
        for (delta, initial) in eroded.iter_mut().zip(small.iter()) {
            *delta -= initial;
        }
        add_upsampled(hmap, size, &eroded, work);
    }
    log(&format!(
        "gpu=>fluvial {}x{} {} iteration(s): fill {} cold, {}..{} warm, {}..{} per refill, area {}..{} level(s), {} sweep(s), {} slide pass(es), {} submission(s), {} ms on the GPU of {} ms",
        work.0,
        work.1,
        params.iterations,
        stats.cold_fill,
        stats.fill.0,
        stats.fill.1,
        stats.refill.0,
        stats.refill.1,
        stats.levels.0,
        stats.levels.1,
        stats.sweeps,
        params.talus.passes,
        stats.submissions,
        stats.gpu_ms,
        start.elapsed().as_millis()
    ));
    Ok(())
}

/// what a run did, for the log line
pub(super) struct FluvialStats {
    /// fill dispatches the cold start needed
    cold_fill: usize,
    /// fewest and most dispatches a periodic full fill needed
    refill: (usize, usize),
    /// fewest and most fill dispatches a warm iteration needed
    fill: (usize, usize),
    /// fewest and most accumulation levels an iteration needed
    levels: (usize, usize),
    sweeps: usize,
    submissions: usize,
    gpu_ms: u128,
}

/// everything one run binds
struct FluvialRun {
    n: usize,
    bytes: u64,
    /// most fill dispatches an iteration may run
    fill_cap: usize,
    /// params entry / flag slot of the first warm fill dispatch
    slot_fill_warm: usize,
    /// params entry / flag slot of accumulation level 0
    slot_area: usize,
    flag_count: usize,
    /// bytes between two params entries
    stride: u64,
    workgroups: (u32, u32),
    fill_workgroups: (u32, u32),
    fill_init: wgpu::ComputePipeline,
    fill: wgpu::ComputePipeline,
    route: wgpu::ComputePipeline,
    jump: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    incise: wgpu::ComputePipeline,
    thermal: wgpu::ComputePipeline,
    /// the iteration's input heights; the two buffers the sweeps and slides alternate on live
    /// in their bind groups only
    x: wgpu::Buffer,
    #[cfg_attr(not(test), allow(dead_code))]
    y: wgpu::Buffer,
    /// filled surface, ping-pong; the bind groups hold the buffers, the fields serve the
    /// test-only `fluvial_route_gpu` readback
    #[cfg_attr(not(test), allow(dead_code))]
    w: [wgpu::Buffer; 2],
    #[cfg_attr(not(test), allow(dead_code))]
    recv: wgpu::Buffer,
    /// doubling counts, ping-pong (the pointers live in their bind groups only)
    #[cfg_attr(not(test), allow(dead_code))]
    s: [wgpu::Buffer; 2],
    flags: wgpu::Buffer,
    flags_staging: wgpu::Buffer,
    h_staging: wgpu::Buffer,
    /// `[p]`: `x`, lake, `w[p]` (the previous surface) → `w[1 - p]`
    bg_fill_init: [wgpu::BindGroup; 2],
    /// `[d % 2]`: `w[0] → w[1]`, `w[1] → w[0]`
    bg_fill: [wgpu::BindGroup; 2],
    /// `[i]`: reads `w[i]`
    bg_route: [wgpu::BindGroup; 2],
    /// `[k % 2]`: `p[0], s[0] → p[1], s[1]` and back
    bg_jump: [wgpu::BindGroup; 2],
    bg_scatter: [wgpu::BindGroup; 2],
    /// `[i][role]`: area from `s[i]`; roles `x → y`, `y → z`, `z → y`
    bg_incise: [[wgpu::BindGroup; 3]; 2],
    /// `y → z`, `z → y`, then the last pass of an iteration `y → x`, `z → x`
    bg_thermal: [wgpu::BindGroup; 4],
}

/// the dispatch counts of one iteration
struct IterationPlan {
    /// warm fill dispatches, and those of a periodic full fill
    fill_dispatches: usize,
    refill_dispatches: usize,
    area_levels: usize,
    sweeps: usize,
    talus_passes: usize,
}

impl GpuContext {
    /// runs `params.iterations` of the resident fluvial pipeline over `hmap` (the working
    /// grid) and reads the result back once. `Ok(Some)` with `hmap` holding the result,
    /// `Ok(None)` when `progress` cancelled and `Err` on any device error, `hmap` untouched in
    /// both cases.
    pub(super) fn erode_fluvial(
        &self,
        size: (usize, usize),
        hmap: &mut [f32],
        params: &FluvialParams,
        progress: &mut Progress,
    ) -> Result<Option<FluvialStats>, GpuError> {
        debug_assert_eq!(hmap.len(), size.0 * size.1);
        let sweeps = INCISE_SWEEPS_BASE + INCISE_SWEEPS_PER_SCALE * params.talus.passes;
        let mut stats = FluvialStats {
            cold_fill: 0,
            refill: (usize::MAX, 0),
            fill: (usize::MAX, 0),
            levels: (usize::MAX, 0),
            sweeps,
            submissions: 0,
            gpu_ms: 0,
        };
        if hmap.is_empty() {
            return Ok(Some(stats));
        }
        let start = Instant::now();
        let run = self.prepare_fluvial(size, hmap, params)?;
        if !progress.report(0.0) {
            return Ok(None);
        }
        let (mut parity, needed, submissions) = self.cold_fill(&run)?;
        stats.cold_fill = needed;
        stats.submissions = submissions;
        let mut refill_budget = (needed + 2 + needed / 4).clamp(FILL_MIN, run.fill_cap);
        let mut fill_budget = FILL_MIN;
        let mut levels = initial_levels(size);
        let chunk = (CHUNK_CELLS / run.n).clamp(1, MAX_CHUNK);
        // iteration 0's fill is done; its erosion opens the first chunk. Chunks grow from
        // `FIRST_CHUNK` so the budgets settle on a few iterations before a long submission
        let mut it = 0;
        let mut ramp = FIRST_CHUNK;
        while it < params.iterations {
            if it > 0 && !progress.report(it as f32 / params.iterations as f32) {
                return Ok(None);
            }
            let count = chunk.min(ramp).min(params.iterations - it);
            ramp *= 2;
            let last = it + count == params.iterations;
            let plan = IterationPlan {
                fill_dispatches: fill_budget,
                refill_dispatches: refill_budget,
                area_levels: levels,
                sweeps,
                talus_passes: params.talus.passes,
            };
            let (idx, after) = self.submit_fluvial_chunk(&run, parity, it, count, &plan, last);
            parity = after;
            let flags: Vec<u32> =
                self.read_staging(&run.flags_staging, run.flag_count as u64 * 4, Some(idx))?;
            stats.submissions += 1;
            let adapt = |flags: &[u32],
                         first: usize,
                         budget: usize,
                         cap: usize,
                         stat: &mut (usize, usize)| {
                let last = (0..budget).rev().find(|&d| flags[first + d] != 0);
                let needed = last.map_or(1, |d| d + 1);
                *stat = (stat.0.min(needed), stat.1.max(needed));
                match last {
                    Some(d) if d + 1 == budget => (budget * 2).min(cap),
                    Some(d) => (d + 2 + d / 4).clamp(FILL_MIN, cap),
                    None => FILL_MIN,
                }
            };
            // the first chunk may hold no warm fill (the cold start did iteration 0's)
            if it > 0 || count > 1 {
                fill_budget = adapt(
                    &flags,
                    run.slot_fill_warm,
                    fill_budget,
                    WARM_FILL_CAP,
                    &mut stats.fill,
                );
            }
            if (it.max(1)..it + count).any(|i| i % REFILL_EVERY == 0) {
                refill_budget = adapt(
                    &flags,
                    SLOT_FILL,
                    refill_budget,
                    run.fill_cap,
                    &mut stats.refill,
                );
            }
            let last_level = (0..levels).rev().find(|&k| flags[run.slot_area + k] != 0);
            let needed = last_level.map_or(1, |k| k + 2);
            stats.levels = (stats.levels.0.min(needed), stats.levels.1.max(needed));
            levels = match last_level {
                Some(k) if k + 1 == levels => (levels + 4).min(MAX_LEVELS),
                Some(k) => (k + 2).clamp(LEVELS_MIN, MAX_LEVELS),
                None => LEVELS_MIN,
            };
            if last {
                // the chunk is complete once the flags are in: no index to wait for
                let result: Vec<f32> = self.read_staging(&run.h_staging, run.bytes, None)?;
                hmap.copy_from_slice(&result);
            }
            it += count;
        }
        if stats.fill.0 == usize::MAX {
            stats.fill = (0, 0);
        }
        if stats.refill.0 == usize::MAX {
            stats.refill = (0, 0);
        }
        stats.gpu_ms = start.elapsed().as_millis();
        Ok(Some(stats))
    }

    /// the cold start's fill: the cold init, then relaxation dispatches in submissions of
    /// `COLD_FILL_STEP`, `2 * COLD_FILL_STEP`, .. until a submission's last dispatch changed
    /// nothing or `fill_cap` dispatches ran. Returns the parity of the buffer holding the
    /// filled surface, the dispatches that were needed and the submissions made
    fn cold_fill(&self, run: &FluvialRun) -> Result<(usize, usize, usize), GpuError> {
        let mut done = 0;
        let mut step = COLD_FILL_STEP.min(run.fill_cap);
        let mut submissions = 0;
        let mut parity = 0;
        loop {
            self.queue
                .write_buffer(&run.flags, 0, &vec![0u8; run.flag_count * 4]);
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                let init = if done == 0 { Some(true) } else { None };
                parity = encode_fill(run, &mut pass, init, parity, done, step, false);
            }
            encoder.copy_buffer_to_buffer(
                &run.flags,
                0,
                &run.flags_staging,
                0,
                run.flag_count as u64 * 4,
            );
            let idx = self.queue.submit([encoder.finish()]);
            let flags: Vec<u32> =
                self.read_staging(&run.flags_staging, run.flag_count as u64 * 4, Some(idx))?;
            submissions += 1;
            let last = (done..done + step)
                .rev()
                .find(|&d| flags[SLOT_FILL + d] != 0);
            done += step;
            match last {
                Some(d) if d + 1 == done && done < run.fill_cap => {
                    step = (step * 2).min(run.fill_cap - done);
                }
                Some(d) => return Ok((parity, d + 1, submissions)),
                None => return Ok((parity, done - step, submissions)),
            }
        }
    }

    /// one submission holding iterations `first..first + count` (iteration 0's fill is
    /// skipped: the cold start left it in `w[parity]`; every `REFILL_EVERY`th iteration
    /// starts with a full fill, the others with a warm one), the flags copy and, for the last
    /// chunk, the result copy; the flags are cleared first. Returns the parity of the last
    /// filled surface
    fn submit_fluvial_chunk(
        &self,
        run: &FluvialRun,
        parity: usize,
        first: usize,
        count: usize,
        plan: &IterationPlan,
        last: bool,
    ) -> (wgpu::SubmissionIndex, usize) {
        self.queue
            .write_buffer(&run.flags, 0, &vec![0u8; run.flag_count * 4]);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut parity = parity;
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            for i in first..first + count {
                if i > 0 && i % REFILL_EVERY == 0 {
                    parity = encode_fill(
                        run,
                        &mut pass,
                        Some(true),
                        parity,
                        0,
                        plan.refill_dispatches,
                        false,
                    );
                } else if i > 0 {
                    parity = encode_fill(
                        run,
                        &mut pass,
                        Some(false),
                        parity,
                        0,
                        plan.fill_dispatches,
                        true,
                    );
                }
                encode_erosion(run, &mut pass, parity, plan);
            }
        }
        encoder.copy_buffer_to_buffer(
            &run.flags,
            0,
            &run.flags_staging,
            0,
            run.flag_count as u64 * 4,
        );
        if last {
            encoder.copy_buffer_to_buffer(&run.x, 0, &run.h_staging, 0, run.bytes);
        }
        (self.queue.submit([encoder.finish()]), parity)
    }

    /// waits for submission `idx` (or for the queue when `None`) and returns the staging
    /// buffer's first `bytes`
    fn read_staging<T: Pod>(
        &self,
        staging: &wgpu::Buffer,
        bytes: u64,
        idx: Option<wgpu::SubmissionIndex>,
    ) -> Result<Vec<T>, GpuError> {
        let (tx, rx) = std::sync::mpsc::channel();
        staging.map_async(wgpu::MapMode::Read, 0..bytes, move |r| {
            let _ = tx.send(r);
        });
        let wait = self.device.poll(wgpu::PollType::Wait {
            submission_index: idx,
            timeout: None,
        });
        let mapped = match (wait, rx.recv()) {
            (Err(e), _) => Err(e.to_string()),
            (_, Ok(Ok(()))) => Ok(()),
            (_, Ok(Err(e))) => Err(e.to_string()),
            (_, Err(_)) => Err("readback callback dropped".to_string()),
        };
        if let Err(msg) = mapped {
            return Err(self.fail(false, format!("fluvial: {msg}")));
        }
        if self.failed.load(Ordering::Relaxed) {
            return Err(self.fail(
                false,
                "fluvial: device reported an error during the chunk".to_string(),
            ));
        }
        let data = {
            let view = staging.get_mapped_range(0..bytes);
            bytemuck::cast_slice(&view).to_vec()
        };
        staging.unmap();
        Ok(data)
    }

    /// the setup phase: pipelines, buffers, uniforms, bind groups and the upload of `hmap`,
    /// under error scopes so that a validation or allocation failure is an `Err`
    fn prepare_fluvial(
        &self,
        size: (usize, usize),
        hmap: &[f32],
        params: &FluvialParams,
    ) -> Result<FluvialRun, GpuError> {
        let n = hmap.len();
        let bytes = n as u64 * 4;
        if bytes > self.limits.max_storage_buffer_binding_size {
            return Err(self.fail(
                false,
                format!(
                    "fluvial: {}x{} exceeds the storage binding limit",
                    size.0, size.1
                ),
            ));
        }
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let (fill_init, fill_init_layout) = self.kernel(
            "fluvial_fill_init",
            FILL_INIT_WGSL,
            Layout::Resident(&[true, true, true, false]),
        );
        let (fill, fill_layout) = self.kernel(
            "fluvial_fill",
            FILL_WGSL,
            Layout::Resident(&[true, true, false, false, true]),
        );
        let (route, route_layout) = self.kernel(
            "fluvial_route",
            ROUTE_WGSL,
            Layout::Resident(&[true, true, false, false, false, false]),
        );
        let (jump, jump_layout) = self.kernel(
            "fluvial_area_jump",
            AREA_JUMP_WGSL,
            Layout::Resident(&[true, false, true, false, false]),
        );
        let (scatter, scatter_layout) = self.kernel(
            "fluvial_area_scatter",
            AREA_SCATTER_WGSL,
            Layout::Resident(&[true, true, false]),
        );
        let (incise, incise_layout) = self.kernel(
            "fluvial_incise",
            INCISE_WGSL,
            Layout::Resident(&[true, true, false, true, true]),
        );
        let (thermal, thermal_layout) = self.kernel("thermal", THERMAL_WGSL, Layout::PingPong);

        let storage = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let x = storage("fluvial x");
        let y = storage("fluvial y");
        let z = storage("fluvial z");
        let w = [storage("fluvial w0"), storage("fluvial w1")];
        let recv = storage("fluvial recv");
        let p = [storage("fluvial p0"), storage("fluvial p1")];
        let s = [storage("fluvial s0"), storage("fluvial s1")];
        let lake = storage("fluvial lake");
        self.queue.write_buffer(&x, 0, bytemuck::cast_slice(hmap));

        let fill_cap = (size.0 + size.1) / 8 + 32;
        let slot_fill_warm = SLOT_FILL + fill_cap;
        let slot_area = slot_fill_warm + fill_cap;
        let flag_count = slot_area + MAX_LEVELS;
        let flags = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fluvial flags"),
            size: flag_count as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = |label: &str, size: u64| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let flags_staging = staging("fluvial flags staging", flag_count as u64 * 4);
        let h_staging = staging("fluvial staging", bytes);

        let band = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("band"),
                contents: bytemuck::bytes_of(&Band {
                    width: size.0 as u32,
                    height: size.1 as u32,
                    first_row: 0,
                    rows: size.1 as u32,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let stride = (self.limits.min_uniform_buffer_offset_alignment as u64)
            .max(std::mem::size_of::<FluvialGpuParams>() as u64);
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fluvial params"),
                contents: &params_entries(
                    size,
                    hmap,
                    params,
                    flag_count,
                    slot_fill_warm..slot_area,
                    stride as usize,
                ),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let thermal_params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("thermal params"),
                contents: bytemuck::bytes_of(&thermal_gpu_params(&params.talus)),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bg = |label: &str, layout: &wgpu::BindGroupLayout, buffers: &[&wgpu::Buffer]| {
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: band.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &params_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<FluvialGpuParams>() as u64),
                    }),
                },
            ];
            entries.extend(
                buffers
                    .iter()
                    .enumerate()
                    .map(|(i, b)| wgpu::BindGroupEntry {
                        binding: i as u32 + 2,
                        resource: b.as_entire_binding(),
                    }),
            );
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &entries,
            })
        };
        let bg_fill_init = [
            bg(
                "fluvial_fill_init",
                &fill_init_layout,
                &[&x, &lake, &w[0], &w[1]],
            ),
            bg(
                "fluvial_fill_init",
                &fill_init_layout,
                &[&x, &lake, &w[1], &w[0]],
            ),
        ];
        let bg_fill = [
            bg(
                "fluvial_fill",
                &fill_layout,
                &[&x, &w[0], &w[1], &flags, &lake],
            ),
            bg(
                "fluvial_fill",
                &fill_layout,
                &[&x, &w[1], &w[0], &flags, &lake],
            ),
        ];
        let bg_route = [
            bg(
                "fluvial_route",
                &route_layout,
                &[&x, &w[0], &recv, &p[0], &s[0], &lake],
            ),
            bg(
                "fluvial_route",
                &route_layout,
                &[&x, &w[1], &recv, &p[0], &s[0], &lake],
            ),
        ];
        let bg_jump = [
            bg(
                "fluvial_area_jump",
                &jump_layout,
                &[&p[0], &p[1], &s[0], &s[1], &flags],
            ),
            bg(
                "fluvial_area_jump",
                &jump_layout,
                &[&p[1], &p[0], &s[1], &s[0], &flags],
            ),
        ];
        let bg_scatter = [
            bg(
                "fluvial_area_scatter",
                &scatter_layout,
                &[&p[0], &s[0], &s[1]],
            ),
            bg(
                "fluvial_area_scatter",
                &scatter_layout,
                &[&p[1], &s[1], &s[0]],
            ),
        ];
        let incise_set = |area: &wgpu::Buffer| {
            [
                bg("fluvial_incise", &incise_layout, &[&x, &x, &y, &recv, area]),
                bg("fluvial_incise", &incise_layout, &[&x, &y, &z, &recv, area]),
                bg("fluvial_incise", &incise_layout, &[&x, &z, &y, &recv, area]),
            ]
        };
        let bg_incise = [incise_set(&s[0]), incise_set(&s[1])];
        let thermal_bg = |src: &wgpu::Buffer, dst: &wgpu::Buffer| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("thermal"),
                layout: &thermal_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: band.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: thermal_params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: src.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: dst.as_entire_binding(),
                    },
                ],
            })
        };
        let bg_thermal = [
            thermal_bg(&y, &z),
            thermal_bg(&z, &y),
            thermal_bg(&y, &x),
            thermal_bg(&z, &x),
        ];

        let errors = [
            pollster::block_on(oom.pop()),
            pollster::block_on(validation.pop()),
        ];
        if let Some(e) = errors.into_iter().flatten().next() {
            return Err(self.fail(false, format!("fluvial: {e}")));
        }
        Ok(FluvialRun {
            n,
            bytes,
            fill_cap,
            slot_fill_warm,
            slot_area,
            flag_count,
            stride,
            workgroups: (
                (size.0 as u32).div_ceil(WORKGROUP),
                (size.1 as u32).div_ceil(WORKGROUP),
            ),
            fill_workgroups: (
                (size.0 as u32).div_ceil(FILL_TILE),
                (size.1 as u32).div_ceil(FILL_TILE),
            ),
            fill_init,
            fill,
            route,
            jump,
            scatter,
            incise,
            thermal,
            x,
            y,
            w,
            recv,
            s,
            flags,
            flags_staging,
            h_staging,
            bg_fill_init,
            bg_fill,
            bg_route,
            bg_jump,
            bg_scatter,
            bg_incise,
            bg_thermal,
        })
    }
}

/// the params uniform: one entry per slot, `slot` its own index, `cold` on `SLOT_COLD`,
/// `freeze` on the warm fill slots
fn params_entries(
    size: (usize, usize),
    hmap: &[f32],
    params: &FluvialParams,
    slots: usize,
    warm: std::ops::Range<usize>,
    stride: usize,
) -> Vec<u8> {
    let n_cells = hmap.len() as f32;
    let (min, max) = get_min_max(hmap);
    let relief = if max > min { max - min } else { 1.0 };
    let eps = FILL_EPS * relief * REFERENCE_RES / size.0.max(size.1) as f32;
    let mut bytes = vec![0u8; slots * stride];
    for slot in 0..slots {
        let entry = FluvialGpuParams {
            k: params.k,
            uplift: params.uplift,
            water_level: params.water_level,
            n_cells,
            area_floor: HILLSLOPE_AREA * n_cells,
            eps,
            slot: slot as u32,
            cold: (slot == SLOT_COLD) as u32,
            freeze: warm.contains(&slot) as u32,
            _pad: [0; 3],
        };
        let at = slot * stride;
        bytes[at..at + std::mem::size_of::<FluvialGpuParams>()]
            .copy_from_slice(bytemuck::bytes_of(&entry));
    }
    bytes
}

/// accumulation levels to start with: enough for a path of `2 (w + h)` cells
fn initial_levels(size: (usize, usize)) -> usize {
    let longest = 2 * (size.0 + size.1);
    ((usize::BITS - longest.leading_zeros()) as usize + 1).clamp(LEVELS_MIN, MAX_LEVELS)
}

fn dispatch(
    pass: &mut wgpu::ComputePass,
    pipeline: &wgpu::ComputePipeline,
    bg: &wgpu::BindGroup,
    offsets: &[u32],
    wg: (u32, u32),
) {
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bg, offsets);
    pass.dispatch_workgroups(wg.0, wg.1, 1);
}

/// the fill of one iteration: the init when `init` is `Some(cold)` (reading the previous
/// surface from `w[parity]`, writing the other buffer), then `dispatches` relaxations
/// numbered from `first` (their flag slots, in the warm region when `warm`, which also
/// freezes last iteration's lakes), starting from `w[parity]` when there is no init.
/// Returns the parity of the buffer holding the filled surface. Dispatches of one compute
/// pass see the writes of the previous ones
fn encode_fill(
    run: &FluvialRun,
    pass: &mut wgpu::ComputePass,
    init: Option<bool>,
    parity: usize,
    first: usize,
    dispatches: usize,
    warm: bool,
) -> usize {
    let offset = |slot: usize| (slot as u64 * run.stride) as u32;
    let slot_fill = if warm { run.slot_fill_warm } else { SLOT_FILL };
    let mut cur = parity;
    if let Some(cold) = init {
        let init_slot = if cold { SLOT_COLD } else { SLOT_WARM };
        dispatch(
            pass,
            &run.fill_init,
            &run.bg_fill_init[parity],
            &[offset(init_slot)],
            run.workgroups,
        );
        cur = 1 - parity;
    }
    for d in first..first + dispatches {
        dispatch(
            pass,
            &run.fill,
            &run.bg_fill[cur],
            &[offset(slot_fill + d)],
            run.fill_workgroups,
        );
        cur = 1 - cur;
    }
    cur
}

/// the rest of an iteration on the filled surface `w[parity]`: routing, the accumulation
/// levels, the incision sweeps and the slide passes, the last of which writes the result into
/// `x`, the next iteration's input
fn encode_erosion(
    run: &FluvialRun,
    pass: &mut wgpu::ComputePass,
    parity: usize,
    plan: &IterationPlan,
) {
    let offset = |slot: usize| (slot as u64 * run.stride) as u32;
    dispatch(
        pass,
        &run.route,
        &run.bg_route[parity],
        &[offset(SLOT_WARM)],
        run.workgroups,
    );
    for k in 0..plan.area_levels {
        dispatch(
            pass,
            &run.jump,
            &run.bg_jump[k % 2],
            &[offset(run.slot_area + k)],
            run.workgroups,
        );
        dispatch(
            pass,
            &run.scatter,
            &run.bg_scatter[k % 2],
            &[offset(SLOT_WARM)],
            run.workgroups,
        );
    }
    let area = plan.area_levels % 2;
    let sweeps = plan.sweeps.max(1);
    for sw in 0..sweeps {
        let role = if sw == 0 { 0 } else { 1 + (sw + 1) % 2 };
        dispatch(
            pass,
            &run.incise,
            &run.bg_incise[area][role],
            &[offset(SLOT_WARM)],
            run.workgroups,
        );
    }
    // sweep 1 lands in `y`, sweep 2 in `z`, ...; `cur` is 0 for `y`, 1 for `z`
    let mut cur = (sweeps + 1) % 2;
    let passes = plan.talus_passes.max(1);
    for t in 0..passes {
        let into_x = t + 1 == passes;
        dispatch(
            pass,
            &run.thermal,
            &run.bg_thermal[cur + if into_x { 2 } else { 0 }],
            &[],
            run.workgroups,
        );
        cur = 1 - cur;
    }
}

/// the filled surface, the receivers (`u32::MAX` for a sink) and the drainage areas in cells
/// of one routing pass, for the tests
#[cfg(test)]
pub(super) struct Routing {
    pub w: Vec<f32>,
    pub recv: Vec<u32>,
    pub area: Vec<u32>,
}

#[cfg(test)]
impl GpuContext {
    /// one cold routing pass over `hmap`, read back
    pub(super) fn fluvial_route_gpu(
        &self,
        size: (usize, usize),
        hmap: &[f32],
        params: &FluvialParams,
    ) -> Result<Routing, GpuError> {
        let run = self.prepare_fluvial(size, hmap, params)?;
        let levels = (usize::BITS - run.n.leading_zeros()) as usize + 1;
        let plan = IterationPlan {
            fill_dispatches: run.fill_cap,
            refill_dispatches: run.fill_cap,
            area_levels: levels,
            sweeps: 1,
            talus_passes: 0,
        };
        let staging = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: run.bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let (w_staging, recv_staging, s_staging) = (staging("w"), staging("recv"), staging("s"));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let filled;
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            let parity = encode_fill(&run, &mut pass, Some(true), 0, 0, run.fill_cap, false);
            encode_erosion(&run, &mut pass, parity, &plan);
            filled = parity;
        }
        encoder.copy_buffer_to_buffer(&run.w[filled], 0, &w_staging, 0, run.bytes);
        encoder.copy_buffer_to_buffer(&run.recv, 0, &recv_staging, 0, run.bytes);
        encoder.copy_buffer_to_buffer(&run.s[levels % 2], 0, &s_staging, 0, run.bytes);
        let idx = self.queue.submit([encoder.finish()]);
        Ok(Routing {
            w: self.read_staging(&w_staging, run.bytes, Some(idx))?,
            recv: self.read_staging(&recv_staging, run.bytes, None)?,
            area: self.read_staging(&s_staging, run.bytes, None)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::calib::stock_map;
    use crate::generators::{gen_fluvial_erosion, FlowNet};
    use crate::gpu::test_context;

    const SINK: u32 = u32::MAX;

    /// a cone tilted along x: every cell has one strictly steepest lower neighbour, no ties,
    /// no depressions
    fn tilted_cone(side: usize) -> Vec<f32> {
        let c = (side as f32 - 1.0) / 2.0;
        (0..side * side)
            .map(|i| {
                let (x, y) = ((i % side) as f32, (i / side) as f32);
                let r = ((x - c).powi(2) + (y - c).powi(2)).sqrt();
                (side as f32 - r) * 0.5 + x * 0.013
            })
            .collect()
    }

    /// mean |a - b|, the largest |a - b| and the cell where it occurs
    fn diff_stats(a: &[f32], b: &[f32]) -> (f32, f32, usize) {
        let (sum, max, at) =
            a.iter()
                .zip(b)
                .enumerate()
                .fold((0.0f32, 0.0f32, 0), |(sum, max, at), (i, (x, y))| {
                    let d = (x - y).abs();
                    if d > max {
                        (sum + d, d, i)
                    } else {
                        (sum + d, max, at)
                    }
                });
        (sum / a.len() as f32, max, at)
    }

    /// Pearson correlation of the two height fields
    fn correlation(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len() as f32;
        let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
        let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(b) {
            sab += (x - ma) * (y - mb);
            saa += (x - ma) * (x - ma);
            sbb += (y - mb) * (y - mb);
        }
        sab / (saa * sbb).sqrt()
    }

    fn run_both(
        gpu: &GpuContext,
        size: (usize, usize),
        input: &[f32],
        conf: &FluvialErosionConf,
    ) -> (Vec<f32>, Vec<f32>) {
        let mut cpu = input.to_vec();
        let mut on_gpu = input.to_vec();
        gen_fluvial_erosion(size, &mut cpu, conf, &mut Progress::headless());
        gen_fluvial_erosion_gpu(gpu, size, &mut on_gpu, conf, &mut Progress::headless()).unwrap();
        (cpu, on_gpu)
    }

    #[test]
    fn fluvial_gpu_is_deterministic() {
        let Some(gpu) = test_context() else { return };
        let input = stock_map(5, (64, 64));
        let conf = FluvialErosionConf::default();
        let mut a = input.clone();
        let mut b = input.clone();
        gen_fluvial_erosion_gpu(&gpu, (64, 64), &mut a, &conf, &mut Progress::headless()).unwrap();
        gen_fluvial_erosion_gpu(&gpu, (64, 64), &mut b, &conf, &mut Progress::headless()).unwrap();
        assert_eq!(a, b);
        assert!(a != input, "the twin did nothing");
        assert!(a.iter().all(|v| v.is_finite()));
    }

    /// on a map without depressions or ties the fill is the map, the routing and the areas
    /// are the CPU's exactly, and the incision sweeps converge on the CPU's implicit solve
    #[test]
    fn fluvial_gpu_matches_cpu_on_a_pit_free_map() {
        let Some(gpu) = test_context() else { return };
        let size = (16, 16);
        let input = tilted_cone(16);
        let conf = FluvialErosionConf {
            work_res: 16,
            talus: 0.0,
            iterations: 5,
            ..Default::default()
        };
        let (_, params) = fluvial_plan(size, &conf);
        let Routing { w, recv, area } = gpu.fluvial_route_gpu(size, &input, &params).unwrap();
        let mut net = FlowNet::new();
        net.route(size, &input, params.water_level);
        for i in 0..256 {
            assert_eq!(w[i], input[i], "cell {i} filled");
            if net.is_base_level(i) {
                assert_eq!(recv[i], SINK, "cell {i} receiver");
            } else {
                assert_eq!(recv[i], net.recv[i], "cell {i} receiver");
            }
            assert_eq!(area[i] as f32, net.area[i], "cell {i} area");
        }
        let (cpu, on_gpu) = run_both(&gpu, size, &input, &conf);
        let (mean, max, i) = diff_stats(&cpu, &on_gpu);
        eprintln!("pit-free cone: mean |cpu - gpu| = {mean}, max = {max}");
        assert!(max <= 1e-5, "max |cpu - gpu| = {max} at cell {i}");
        assert!(cpu != input);
    }

    /// the fill gives every land cell of a map with pits a receiver chain that reaches a sink
    #[test]
    fn fluvial_gpu_fills_depressions() {
        let Some(gpu) = test_context() else { return };
        let size = (64, 64);
        let input = stock_map(5, size);
        let conf = FluvialErosionConf::default();
        let (_, params) = fluvial_plan(size, &conf);
        let Routing { w, recv, area } = gpu.fluvial_route_gpu(size, &input, &params).unwrap();
        let mut land = 0;
        for i in 0..64 * 64 {
            assert!(w[i] >= input[i], "cell {i} filled below its height");
            let border = i % 64 == 0 || i / 64 == 0 || i % 64 == 63 || i / 64 == 63;
            if border || input[i] <= params.water_level {
                assert_eq!(recv[i], SINK);
                continue;
            }
            land += 1;
            assert_ne!(recv[i], SINK, "land cell {i} has no receiver");
            let mut c = i;
            let mut steps = 0;
            while recv[c] != SINK {
                assert!(w[recv[c] as usize] < w[c], "cell {c} drains uphill");
                c = recv[c] as usize;
                steps += 1;
                assert!(steps < 64 * 64, "cycle from {i}");
            }
        }
        assert!(land > 1000, "the stock map is mostly land");
        let total: u64 = area.iter().map(|&a| a as u64).sum();
        assert!(total as usize >= 64 * 64, "every cell counts in the areas");
    }

    /// the GPU landscape is the CPU's up to the algorithm differences: same valleys, small
    /// height differences
    #[test]
    fn fluvial_gpu_is_similar_to_cpu() {
        let Some(gpu) = test_context() else { return };
        let size = (64, 64);
        let input = stock_map(5, size);
        for (name, conf) in [
            ("default", FluvialErosionConf::default()),
            (
                "second",
                FluvialErosionConf {
                    water_level: 0.3,
                    uplift: 0.01,
                    ..Default::default()
                },
            ),
        ] {
            let (cpu, on_gpu) = run_both(&gpu, size, &input, &conf);
            let (mean, max, _) = diff_stats(&cpu, &on_gpu);
            let r = correlation(&cpu, &on_gpu);
            let (dmean, dmax, _) = diff_stats(&cpu, &input);
            eprintln!(
                "{name}: mean |cpu - gpu| = {mean}, max = {max}, correlation = {r}; the CPU moved the map by mean {dmean}, max {dmax}"
            );
            assert!(r > 0.99, "{name}: correlation {r}");
            assert!(
                mean < 0.25 * dmean,
                "{name}: mean difference {mean} against a mean change of {dmean}"
            );
        }
    }

    #[test]
    fn fluvial_gpu_works_on_the_working_grid() {
        let Some(gpu) = test_context() else { return };
        let input = stock_map(5, (64, 64));
        let conf = FluvialErosionConf {
            work_res: 16,
            ..Default::default()
        };
        let (cpu, on_gpu) = run_both(&gpu, (64, 64), &input, &conf);
        assert!(on_gpu != input);
        assert!(on_gpu.iter().all(|v| v.is_finite()));
        let r = correlation(&cpu, &on_gpu);
        eprintln!("work_res 16: correlation = {r}");
        assert!(r > 0.99);
    }

    #[test]
    fn fluvial_gpu_cancel_leaves_map_untouched() {
        let Some(gpu) = test_context() else { return };
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut progress = Progress::preview(tx, 1.0, || true);
        let input = stock_map(5, (64, 64));
        let mut h = input.clone();
        let res = gen_fluvial_erosion_gpu(
            &gpu,
            (64, 64),
            &mut h,
            &FluvialErosionConf::default(),
            &mut progress,
        );
        assert!(res.is_ok());
        assert_eq!(h, input);
    }

    /// `cargo test fluvial_kernel_timing_report -- --ignored --nocapture`: the cost of 50
    /// dispatches of each kernel at 512x512, and of 50 empty compute passes, to see where an
    /// iteration's time goes
    #[test]
    #[ignore]
    fn fluvial_kernel_timing_report() {
        let Some(gpu) = test_context() else { return };
        let size = (512, 512);
        let input = stock_map(5, size);
        let (_, params) = fluvial_plan(size, &FluvialErosionConf::default());
        let run = gpu.prepare_fluvial(size, &input, &params).unwrap();
        let offset = |slot: usize| (slot as u64 * run.stride) as u32;
        let time = |label: &str, encode: &dyn Fn(&mut wgpu::CommandEncoder)| {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            for _ in 0..50 {
                encode(&mut encoder);
            }
            encoder.copy_buffer_to_buffer(&run.flags, 0, &run.flags_staging, 0, 4);
            let start = Instant::now();
            let idx = gpu.queue.submit([encoder.finish()]);
            let _: Vec<u32> = gpu.read_staging(&run.flags_staging, 4, Some(idx)).unwrap();
            eprintln!(
                "{label:<28} {:>8.2} ms / 50",
                start.elapsed().as_secs_f64() * 1000.0
            );
        };
        let dispatch = |encoder: &mut wgpu::CommandEncoder,
                        pipeline: &wgpu::ComputePipeline,
                        bg: &wgpu::BindGroup,
                        offsets: &[u32],
                        wg: (u32, u32)| {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bg, offsets);
            pass.dispatch_workgroups(wg.0, wg.1, 1);
        };
        time("empty pass", &|e| {
            e.begin_compute_pass(&Default::default());
        });
        {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                for _ in 0..50 {
                    pass.set_pipeline(&run.incise);
                    pass.set_bind_group(0, &run.bg_incise[0][1], &[offset(SLOT_WARM)]);
                    pass.dispatch_workgroups(run.workgroups.0, run.workgroups.1, 1);
                }
            }
            encoder.copy_buffer_to_buffer(&run.flags, 0, &run.flags_staging, 0, 4);
            let start = Instant::now();
            let idx = gpu.queue.submit([encoder.finish()]);
            let _: Vec<u32> = gpu.read_staging(&run.flags_staging, 4, Some(idx)).unwrap();
            eprintln!(
                "{:<28} {:>8.2} ms / 50",
                "incise, one pass",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        time("fill_init (cold)", &|e| {
            dispatch(
                e,
                &run.fill_init,
                &run.bg_fill_init[0],
                &[offset(SLOT_COLD)],
                run.workgroups,
            )
        });
        time("fill", &|e| {
            dispatch(
                e,
                &run.fill,
                &run.bg_fill[0],
                &[offset(SLOT_FILL)],
                run.fill_workgroups,
            )
        });
        time("route", &|e| {
            dispatch(
                e,
                &run.route,
                &run.bg_route[0],
                &[offset(SLOT_WARM)],
                run.workgroups,
            )
        });
        time("area_jump", &|e| {
            dispatch(
                e,
                &run.jump,
                &run.bg_jump[0],
                &[offset(run.slot_area)],
                run.workgroups,
            )
        });
        time("area_scatter", &|e| {
            dispatch(
                e,
                &run.scatter,
                &run.bg_scatter[0],
                &[offset(SLOT_WARM)],
                run.workgroups,
            )
        });
        time("incise", &|e| {
            dispatch(
                e,
                &run.incise,
                &run.bg_incise[0][1],
                &[offset(SLOT_WARM)],
                run.workgroups,
            )
        });
        time("thermal", &|e| {
            dispatch(e, &run.thermal, &run.bg_thermal[0], &[], run.workgroups)
        });
        time("copy x -> y", &|e| {
            e.copy_buffer_to_buffer(&run.x, 0, &run.y, 0, run.bytes);
        });
    }

    #[test]
    fn initial_levels_cover_the_map() {
        assert_eq!(initial_levels((16, 16)), 8);
        assert_eq!(initial_levels((512, 512)), 13);
        assert_eq!(initial_levels((2048, 2048)), 15);
    }
}
