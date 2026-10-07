use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::generators::{get_min_max, Progress};
use crate::gpu::{Backend, GpuContext};
use crate::mask::{feather_mask, mask_side};
use crate::project::DEFAULT_WATER_LEVEL;
pub use crate::step::{Step, StepType};
use crate::{log, panic_message, ThreadMessage, Waker};

#[derive(Debug)]
/// commands sent by the main thread to the world generator thread
pub enum WorldGenCommand {
    /// recompute a specific step : generation, step index, step conf, live preview, min progress step to report
    ExecuteStep(u64, usize, Step, bool, f32),
    /// remove a step
    DeleteStep(usize),
    /// change the heightmap size
    SetSize(usize),
    /// return the heightmap for a given step : generation, step index
    GetStepMap(u64, usize),
    /// change the random number generator seed
    SetSeed(u64),
    /// change the project's sea level, in raw height units
    SetWaterLevel(f32),
    /// remove all steps
    Clear,
    /// cancel queued ExecuteStep commands from a specific step; the running step stops on its own
    /// through `Invalidation`
    Abort(usize),
    /// run the generators that have a GPU twin on the GPU (true) or on the CPU (false)
    SetBackend(bool),
}

/// the newest `regen`'s (generation, from): a running step of an older generation whose index
/// is >= from will be recomputed anyway, so it stops at its next progress report
#[derive(Clone, Default)]
pub struct Invalidation(Arc<Mutex<(u64, usize)>>);

impl Invalidation {
    pub fn set(&self, generation: u64, from: usize) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = (generation, from);
    }
    pub fn is_stale(&self, generation: u64, index: usize) -> bool {
        let (cur_gen, cur_from) = *self.0.lock().unwrap_or_else(|e| e.into_inner());
        generation < cur_gen && index >= cur_from
    }
}

#[derive(Clone)]
pub struct ExportMap {
    size: (usize, usize),
    h: Vec<f32>,
}

impl ExportMap {
    pub fn get_min_max(&self) -> (f32, f32) {
        get_min_max(&self.h)
    }
    pub fn get_size(&self) -> (usize, usize) {
        self.size
    }
    pub fn height(&self, x: usize, y: usize) -> f32 {
        let off = x + y * self.size.0;
        if off < self.size.0 * self.size.1 {
            return self.h[off];
        }
        0.0
    }
    pub fn borrow(&self) -> &Vec<f32> {
        &self.h
    }
}

#[derive(Clone)]
struct HMap {
    h: Vec<f32>,
}

#[derive(Clone)]
pub struct WorldGenerator {
    seed: u64,
    world_size: (usize, usize),
    /// preview path : one map per step, `hmap[i]` is the output of step i.
    /// export path : a single map, the output of the last step.
    hmap: Vec<HMap>,
    /// where the generators with a GPU twin run
    backend: Backend,
    /// the project's sea level, in raw height units
    water_level: f32,
}

struct InnerStep {
    generation: u64,
    index: usize,
    step: Step,
    live: bool,
    min_progress_step: f32,
}

fn do_command(
    msg: WorldGenCommand,
    wgen: &mut WorldGenerator,
    steps: &mut Vec<InnerStep>,
    tx: &Sender<ThreadMessage>,
    gpu: &Option<Arc<GpuContext>>,
) {
    match msg {
        WorldGenCommand::Clear => wgen.clear(),
        WorldGenCommand::SetSeed(new_seed) => wgen.seed = new_seed,
        WorldGenCommand::SetWaterLevel(level) => wgen.set_water_level(level),
        WorldGenCommand::ExecuteStep(generation, index, step, live, min_progress_step) => {
            steps.push(InnerStep {
                generation,
                index,
                step,
                live,
                min_progress_step,
            });
        }
        WorldGenCommand::DeleteStep(index) => {
            // the step may have been queued but never executed : nothing to drop then
            if index < wgen.hmap.len() {
                wgen.hmap.remove(index);
            }
        }
        WorldGenCommand::GetStepMap(generation, index) => {
            let _ = tx.send(ThreadMessage::GeneratorStepMap(
                generation,
                index,
                wgen.get_step_export_map(index),
            ));
        }
        WorldGenCommand::Abort(from_idx) => steps.retain(|s| s.index < from_idx),
        WorldGenCommand::SetSize(size) => {
            let backend = wgen.backend.clone();
            *wgen = WorldGenerator::new(wgen.seed, (size, size));
            wgen.set_backend(backend);
        }
        WorldGenCommand::SetBackend(on) => wgen.set_backend(match gpu {
            Some(g) if on => Backend::Gpu(g.clone()),
            _ => Backend::Cpu,
        }),
    }
}

pub fn generator_thread(
    seed: u64,
    size: usize,
    rx: Receiver<WorldGenCommand>,
    tx: Sender<ThreadMessage>,
    wake: Waker,
    invalidation: Invalidation,
    gpu: Option<Arc<GpuContext>>,
) {
    let mut wgen = WorldGenerator::new(seed, (size, size));
    if let Some(g) = &gpu {
        wgen.set_backend(Backend::Gpu(g.clone()));
    }
    let mut steps = Vec::new();
    loop {
        if steps.is_empty() {
            // blocking wait; a closed channel means the main thread is gone
            match rx.recv() {
                Ok(msg) => do_command(msg, &mut wgen, &mut steps, &tx, &gpu),
                Err(_) => return,
            }
        }
        while let Ok(msg) = rx.try_recv() {
            do_command(msg, &mut wgen, &mut steps, &tx, &gpu);
        }
        if steps.is_empty() {
            continue;
        }
        let InnerStep {
            generation,
            index,
            step,
            live,
            min_progress_step,
        } = steps.remove(0);
        let mut progress = Progress::preview(tx.clone(), min_progress_step, {
            let inv = invalidation.clone();
            move || inv.is_stale(generation, index)
        });
        let result = catch_unwind(AssertUnwindSafe(|| {
            wgen.execute_step(index, &step, &mut progress)
        }));
        if result.is_ok() && invalidation.is_stale(generation, index) {
            // a newer regen recomputes this step: its map is garbage and its result unwanted
            continue;
        }
        let msg = match result {
            Err(payload) => {
                // the maps past this step are unknown : drop the rest of the queue
                steps.clear();
                ThreadMessage::GeneratorError(format!(
                    "step {} ({}) failed : {}",
                    index,
                    step,
                    panic_message(payload.as_ref())
                ))
            }
            Ok(()) if steps.is_empty() => {
                ThreadMessage::GeneratorDone(generation, wgen.get_export_map())
            }
            Ok(()) => ThreadMessage::GeneratorStepDone(
                generation,
                index,
                if live {
                    Some(wgen.get_step_export_map(index))
                } else {
                    None
                },
            ),
        };
        let _ = tx.send(msg);
        // wake the UI thread so the message is handled without waiting for user input
        wake();
    }
}

impl WorldGenerator {
    pub fn new(seed: u64, world_size: (usize, usize)) -> Self {
        Self {
            seed,
            world_size,
            hmap: Vec::new(),
            backend: Backend::Cpu,
            water_level: DEFAULT_WATER_LEVEL,
        }
    }
    pub fn set_backend(&mut self, backend: Backend) {
        self.backend = backend;
    }
    pub fn set_water_level(&mut self, water_level: f32) {
        self.water_level = water_level;
    }
    pub fn get_export_map(&self) -> ExportMap {
        self.get_step_export_map(if self.hmap.is_empty() {
            0
        } else {
            self.hmap.len() - 1
        })
    }
    /// the last step's map, borrowed; empty before any step ran
    pub fn final_map(&self) -> &[f32] {
        self.hmap.last().map_or(&[], |m| &m.h)
    }
    #[cfg(test)]
    pub fn push_map(&mut self, h: Vec<f32>) {
        self.hmap.push(HMap { h });
    }
    pub fn get_step_export_map(&self, step: usize) -> ExportMap {
        ExportMap {
            size: self.world_size,
            h: if step >= self.hmap.len() {
                vec![0.0; self.world_size.0 * self.world_size.1]
            } else {
                self.hmap[step].h.clone()
            },
        }
    }

    pub fn combined_height(&self, x: usize, y: usize) -> f32 {
        let off = x + y * self.world_size.0;
        if !self.hmap.is_empty() && off < self.world_size.0 * self.world_size.1 {
            return self.hmap[self.hmap.len() - 1].h[off];
        }
        0.0
    }
    pub fn clear(&mut self) {
        self.hmap.clear();
    }

    /// preview path : (re)computes step `index` from the output of step `index - 1`, keeping every
    /// step's map so a later step can be recomputed alone
    fn execute_step(&mut self, index: usize, step: &Step, progress: &mut Progress) {
        let now = Instant::now();
        let vecsize = self.world_size.0 * self.world_size.1;
        while self.hmap.len() <= index {
            let h = match self.hmap.last() {
                Some(last) => last.h.clone(),
                None => vec![0.0; vecsize],
            };
            self.hmap.push(HMap { h });
        }
        let mut cur = std::mem::take(&mut self.hmap[index].h);
        {
            let prev = if index > 0 {
                Some(self.hmap[index - 1].h.as_slice())
            } else {
                None
            };
            match prev {
                Some(prev) => cur.copy_from_slice(prev),
                None => cur.fill(0.0),
            }
            self.run_step(step, &mut cur, prev, progress);
        }
        self.hmap[index].h = cur;
        log(&format!(
            "Executed {} in {:.2}s",
            step,
            now.elapsed().as_secs_f32()
        ));
    }

    /// export path : runs the whole stack on a single map; a masked step keeps one transient copy
    /// of the previous output to blend with
    pub fn generate(&mut self, steps: &[Step], tx: Sender<ThreadMessage>, min_progress_step: f32) {
        self.clear();
        let mut cur = vec![0.0; self.world_size.0 * self.world_size.1];
        for (i, step) in steps.iter().enumerate() {
            let prev = if step.mask.is_some() && i > 0 {
                Some(cur.clone())
            } else {
                None
            };
            // one reporter per step: the throttle restarts from 0 for each
            let mut progress = Progress::export(tx.clone(), min_progress_step);
            self.run_step(step, &mut cur, prev.as_deref(), &mut progress);
            let _ = tx.send(ThreadMessage::ExporterStepDone(i));
        }
        self.hmap.push(HMap { h: cur });
    }

    /// runs one step's generator on `h` (already holding the previous step's output), then blends
    /// the result with `prev` through the step's mask, feathered, if any
    fn run_step(&self, step: &Step, h: &mut [f32], prev: Option<&[f32]>, progress: &mut Progress) {
        let (seed, size) = (self.seed, self.world_size);
        if !step.disabled {
            step.typ
                .run(seed, size, h, progress, &self.backend, self.water_level);
        }
        if let Some(ref mask) = step.mask {
            let mask = feather_mask(mask, step.mask_feather);
            apply_mask(size, &mask, step.mask_smooth, prev, h);
        }
    }

    pub fn get_min_max(&self) -> (f32, f32) {
        if self.hmap.is_empty() {
            (0.0, 0.0)
        } else {
            get_min_max(&self.hmap[self.hmap.len() - 1].h)
        }
    }
}

/// blends `h` with `prev` (or with its own minimum) by the mask, sampled at the mask's own
/// side, bilinearly or, with `smooth`, by a uniform cubic B-spline
fn apply_mask(
    world_size: (usize, usize),
    mask: &[f32],
    smooth: bool,
    prev: Option<&[f32]>,
    h: &mut [f32],
) {
    let n = mask_side(mask);
    let mut off = 0;
    let (min, _) = if prev.is_none() {
        get_min_max(h)
    } else {
        (0.0, 0.0)
    };
    for y in 0..world_size.1 {
        let myf = (y * n) as f32 / world_size.1 as f32;
        for x in 0..world_size.0 {
            let mxf = (x * n) as f32 / world_size.0 as f32;
            let mask_value = if smooth {
                mask_bspline(mask, n, mxf, myf)
            } else {
                mask_bilinear(mask, n, mxf, myf)
            };
            if let Some(prev) = prev {
                h[off] = (1.0 - mask_value) * prev[off] + mask_value * h[off];
            } else {
                h[off] = (1.0 - mask_value) * min + mask_value * (h[off] - min);
            }
            off += 1;
        }
    }
}

/// the mask at mask position `(mxf, myf)` by straight-line blending of the 2 × 2 cells
fn mask_bilinear(mask: &[f32], n: usize, mxf: f32, myf: f32) -> f32 {
    let (mx, my) = (mxf as usize, myf as usize);
    let (xalpha, yalpha) = (mxf.fract(), myf.fract());
    let mut mask_value = mask[mx + my * n];
    if mx + 1 < n {
        mask_value = (1.0 - xalpha) * mask_value + xalpha * mask[mx + 1 + my * n];
        if my + 1 < n {
            let bottom_left_mask = mask[mx + (my + 1) * n];
            let bottom_right_mask = mask[mx + 1 + (my + 1) * n];
            let bottom_mask = (1.0 - xalpha) * bottom_left_mask + xalpha * bottom_right_mask;
            mask_value = (1.0 - yalpha) * mask_value + yalpha * bottom_mask;
        }
    }
    mask_value
}

/// the uniform cubic B-spline weights of the cells `i - 1 … i + 2` at fraction `t`
fn bspline_weights(t: f32) -> [f32; 4] {
    let s = 1.0 - t;
    [
        s * s * s / 6.0,
        (3.0 * t * t * t - 6.0 * t * t + 4.0) / 6.0,
        (-3.0 * t * t * t + 3.0 * t * t + 3.0 * t + 1.0) / 6.0,
        t * t * t / 6.0,
    ]
}

/// the mask at mask position `(mxf, myf)` by B-spline weights over the 4 × 4 cells round it,
/// indices clamped at the mask's edge
fn mask_bspline(mask: &[f32], n: usize, mxf: f32, myf: f32) -> f32 {
    let (mx, my) = (mxf as i64, myf as i64);
    let (wx, wy) = (bspline_weights(mxf.fract()), bspline_weights(myf.fract()));
    let last = n as i64 - 1;
    let mut value = 0.0;
    for (j, wyj) in wy.iter().enumerate() {
        let cy = (my + j as i64 - 1).clamp(0, last) as usize;
        for (i, wxi) in wx.iter().enumerate() {
            let cx = (mx + i as i64 - 1).clamp(0, last) as usize;
            value += wyj * wxi * mask[cx + cy * n];
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::{FbmConf, HillsConf, NormalizeConf};
    use crate::MASK_SIZE;
    use std::sync::mpsc;

    fn masked_stack() -> Vec<Step> {
        let mut mask = vec![1.0; MASK_SIZE * MASK_SIZE];
        for (i, m) in mask.iter_mut().enumerate() {
            if i % 3 == 0 {
                *m = 0.25;
            }
        }
        vec![
            Step {
                typ: StepType::Hills(HillsConf::default()),
                mask: Some(mask.clone()),
                mask_feather: 0.5,
                ..Default::default()
            },
            Step {
                typ: StepType::Fbm(FbmConf::default()),
                ..Default::default()
            },
            Step {
                typ: StepType::Normalize(NormalizeConf::default()),
                disabled: true,
                ..Default::default()
            },
            Step {
                typ: StepType::Normalize(NormalizeConf::default()),
                mask: Some(mask),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn export_generate_matches_preview_steps() {
        let (tx, _) = mpsc::channel();
        let steps = masked_stack();
        let mut preview = WorldGenerator::new(42, (32, 32));
        for (i, step) in steps.iter().enumerate() {
            preview.execute_step(i, step, &mut Progress::headless());
        }
        let mut export = WorldGenerator::new(42, (32, 32));
        export.generate(&steps, tx, 1.0);
        assert_eq!(
            preview.get_export_map().borrow(),
            export.get_export_map().borrow()
        );
        assert_eq!(export.hmap.len(), 1);
    }

    #[test]
    fn execute_step_survives_index_gap() {
        let mut wgen = WorldGenerator::new(1, (8, 8));
        let step = Step::default();
        wgen.execute_step(2, &step, &mut Progress::headless());
        assert_eq!(wgen.hmap.len(), 3);
    }

    #[test]
    fn delete_step_past_end_is_ignored() {
        let (tx, _) = mpsc::channel();
        let mut wgen = WorldGenerator::new(1, (8, 8));
        let mut queue = Vec::new();
        do_command(
            WorldGenCommand::DeleteStep(3),
            &mut wgen,
            &mut queue,
            &tx,
            &None,
        );
        assert!(wgen.hmap.is_empty());
        for i in 0..3 {
            do_command(
                WorldGenCommand::ExecuteStep(1, i, Step::default(), false, 1.0),
                &mut wgen,
                &mut queue,
                &tx,
                &None,
            );
        }
        do_command(WorldGenCommand::Abort(1), &mut wgen, &mut queue, &tx, &None);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn invalidation_marks_only_newer_from_range() {
        let inv = Invalidation::default();
        inv.set(2, 3);
        assert!(inv.is_stale(1, 3));
        assert!(inv.is_stale(1, 7));
        assert!(!inv.is_stale(1, 2));
        assert!(!inv.is_stale(2, 3));
        assert!(!inv.is_stale(2, 7));
        let fresh = Invalidation::default();
        fresh.set(1, 0);
        assert!(!fresh.is_stale(1, 0));
    }

    #[test]
    fn progress_stops_after_invalidation() {
        let (tx, _) = mpsc::channel();
        let inv = Invalidation::default();
        inv.set(1, 0);
        let watched = inv.clone();
        let mut progress = Progress::preview(tx, 1.0, move || watched.is_stale(1, 0));
        assert!(progress.report(0.0));
        inv.set(2, 0);
        assert!(!progress.report(0.5));
        assert!(!progress.report(1.0));
    }

    #[test]
    fn ridged_keeps_the_previous_map_where_its_mask_is_zero() {
        let size = (MASK_SIZE, MASK_SIZE);
        let mut backends = vec![Backend::Cpu];
        if let Some(gpu) = crate::gpu::test_context() {
            backends.push(Backend::Gpu(gpu));
        }
        for backend in backends {
            let mut generator = WorldGenerator::new(7, size);
            generator.set_backend(backend);
            let steps = [
                Step {
                    typ: StepType::Fbm(FbmConf::default()),
                    ..Default::default()
                },
                Step {
                    typ: StepType::Ridged(crate::generators::RidgedConf::default()),
                    mask: Some(crate::mask::tests::half_black_mask()),
                    ..Default::default()
                },
            ];
            for (i, step) in steps.iter().enumerate() {
                generator.execute_step(i, step, &mut Progress::headless());
            }
            let (before, after) = (&generator.hmap[0].h, &generator.hmap[1].h);
            let mask = crate::mask::tests::half_black_mask();
            for (i, m) in mask.iter().enumerate() {
                if *m == 0.0 {
                    assert_eq!(before[i], after[i], "cell {i}");
                }
            }
        }
    }

    #[test]
    fn run_step_feathers_the_mask() {
        // one map pixel per mask cell, so apply_mask samples the mask exactly
        let size = (MASK_SIZE, MASK_SIZE);
        let generator = WorldGenerator::new(1, size);
        let prev = vec![0.0; MASK_SIZE * MASK_SIZE];
        let run = |feather: f32| {
            let step = Step {
                disabled: true,
                mask: Some(crate::mask::tests::half_black_mask()),
                mask_feather: feather,
                mask_smooth: false,
                typ: StepType::Normalize(NormalizeConf::default()),
            };
            let mut h = vec![1.0; MASK_SIZE * MASK_SIZE];
            generator.run_step(&step, &mut h, Some(&prev), &mut Progress::headless());
            h
        };
        let row = 10 * MASK_SIZE;
        let hard = run(0.0);
        assert_eq!(hard[32 + row], 1.0);
        let soft = run(0.5);
        assert!((soft[32 + row] - 0.125).abs() < 1e-6, "{}", soft[32 + row]);
        assert!((soft[39 + row] - 1.0).abs() < 1e-6, "{}", soft[39 + row]);
    }

    #[test]
    fn apply_mask_samples_by_the_mask_side() {
        // a 32² mask on a 64² map equals the 64² mask it upsamples to, applied cell for cell;
        // blending ones over zeros writes the sampled mask itself
        let small: Vec<f32> = (0..32 * 32)
            .map(|i| ((i * 37) % 11) as f32 / 10.0)
            .collect();
        let mut big = vec![1.0; 64 * 64];
        apply_mask((64, 64), &small, false, Some(&vec![0.0; 64 * 64]), &mut big);
        let mut via_small: Vec<f32> = (0..64 * 64).map(|i| (i % 13) as f32).collect();
        let mut via_big = via_small.clone();
        let prev: Vec<f32> = (0..64 * 64).map(|i| (i % 5) as f32).collect();
        apply_mask((64, 64), &small, false, Some(&prev), &mut via_small);
        apply_mask((64, 64), &big, false, Some(&prev), &mut via_big);
        for (a, b) in via_small.iter().zip(&via_big) {
            assert!((a - b).abs() < 1e-5, "{a} vs {b}");
        }
    }

    #[test]
    fn apply_mask_handles_non_square_map() {
        // mask: top half fully applied, bottom half fully masked out
        let mut mask = vec![0.0; MASK_SIZE * MASK_SIZE];
        for m in mask.iter_mut().take(MASK_SIZE * MASK_SIZE / 2) {
            *m = 1.0;
        }
        for &(w, h) in &[(16usize, 32usize), (32, 16), (32, 32)] {
            let mut hmap: Vec<f32> = (0..w * h).map(|i| 1.0 + (i % 7) as f32).collect();
            let expected: Vec<f32> = hmap.iter().map(|v| v - 1.0).collect();
            apply_mask((w, h), &mask, false, None, &mut hmap);
            // first row is fully inside the white half: height shifted down by min
            assert_eq!(&hmap[..w], &expected[..w], "top row at {w}x{h}");
            // last row is fully inside the black half: flattened to min
            assert!(
                hmap[w * (h - 1)..].iter().all(|&v| v == 1.0),
                "bottom row at {w}x{h}: {:?}",
                &hmap[w * (h - 1)..]
            );
        }
    }

    #[test]
    fn smooth_mask_reads_constants_and_ramps() {
        let n = 16;
        let constant = vec![0.4; n * n];
        assert!((mask_bspline(&constant, n, 7.3, 2.9) - 0.4).abs() < 1e-6);
        let ramp: Vec<f32> = (0..n * n).map(|i| (i % n) as f32 / n as f32).collect();
        for x in [3.0f32, 5.25, 8.5, 11.75] {
            let v = mask_bspline(&ramp, n, x, 6.4);
            assert!((v - x / n as f32).abs() < 1e-5, "{x}: {v}");
        }
    }

    #[test]
    fn smooth_mask_stays_in_range_without_creases() {
        let n = 16;
        let mask: Vec<f32> = (0..n * n)
            .map(|i| if (i % n) * 3 % 7 < 3 { 1.0 } else { 0.0 })
            .collect();
        for k in 0..400 {
            let x = k as f32 * 0.04;
            let v = mask_bspline(&mask, n, x, 5.5);
            assert!((-1e-6..=1.0 + 1e-6).contains(&v), "{x}: {v}");
        }
        // slopes on both sides of the cell boundary x = 6
        let e = 1e-3;
        let slope = |f: &dyn Fn(f32) -> f32, a: f32| (f(a + e) - f(a)) / e;
        let smooth = |x: f32| mask_bspline(&mask, n, x, 5.5);
        let linear = |x: f32| mask_bilinear(&mask, n, x, 5.5);
        let (sl, sr) = (slope(&smooth, 6.0 - e), slope(&smooth, 6.0));
        assert!((sl - sr).abs() < 1e-2 * sl.abs().max(1.0), "{sl} vs {sr}");
        let (ll, lr) = (slope(&linear, 6.0 - e), slope(&linear, 6.0));
        assert!(
            (ll - lr).abs() > 0.1,
            "bilinear should crease: {ll} vs {lr}"
        );
    }

    #[test]
    fn step_without_smooth_field_loads_false() {
        let step = Step::default();
        let text = ron::to_string(&step)
            .unwrap()
            .replace(",mask_smooth:false", "");
        assert!(!text.contains("mask_smooth"));
        let loaded: Step = ron::from_str(&text).unwrap();
        assert!(!loaded.mask_smooth);
    }
}
