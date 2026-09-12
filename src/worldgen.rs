use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::Sender;
use std::time::Instant;
use std::{fmt::Display, sync::mpsc::Receiver};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::generators::{
    gen_fbm, gen_hills, gen_island, gen_landmass, gen_mid_point, gen_mudslide, gen_normalize,
    gen_water_erosion, get_min_max, FbmConf, HillsConf, IslandConf, LandMassConf, MidPointConf,
    MudSlideConf, NormalizeConf, WaterErosionConf,
};
use crate::{log, panic_message, ThreadMessage, MASK_SIZE};

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
    /// remove all steps
    Clear,
    /// cancel queued ExecuteStep commands from a specific step (the running step is never interrupted)
    Abort(usize),
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
/// Each value contains its own configuration
pub enum StepType {
    Hills(HillsConf),
    Fbm(FbmConf),
    Normalize(NormalizeConf),
    LandMass(LandMassConf),
    MudSlide(MudSlideConf),
    WaterErosion(WaterErosionConf),
    Island(IslandConf),
    MidPoint(MidPointConf),
}
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Step {
    /// should we skip this step when computing the heightmap ?
    pub disabled: bool,
    /// this step mask
    pub mask: Option<Vec<f32>>,
    /// step type with its configuration
    pub typ: StepType,
}

impl Default for Step {
    fn default() -> Self {
        Self {
            disabled: false,
            mask: None,
            typ: StepType::Normalize(NormalizeConf::default()),
        }
    }
}

impl Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let debug_val = format!("{:?}", self.typ);
        let val: Vec<&str> = debug_val.split('(').collect();
        write!(f, "{}", val[0])
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
) {
    log(&format!("wgen<={:?}", msg));
    match msg {
        WorldGenCommand::Clear => wgen.clear(),
        WorldGenCommand::SetSeed(new_seed) => wgen.seed = new_seed,
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
        WorldGenCommand::SetSize(size) => *wgen = WorldGenerator::new(wgen.seed, (size, size)),
    }
}

pub fn generator_thread(
    seed: u64,
    size: usize,
    rx: Receiver<WorldGenCommand>,
    tx: Sender<ThreadMessage>,
    ctx: egui::Context,
) {
    let mut wgen = WorldGenerator::new(seed, (size, size));
    let mut steps = Vec::new();
    loop {
        if steps.is_empty() {
            // blocking wait; a closed channel means the main thread is gone
            match rx.recv() {
                Ok(msg) => do_command(msg, &mut wgen, &mut steps, &tx),
                Err(_) => return,
            }
        }
        while let Ok(msg) = rx.try_recv() {
            do_command(msg, &mut wgen, &mut steps, &tx);
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
        let result = catch_unwind(AssertUnwindSafe(|| {
            wgen.execute_step(index, &step, tx.clone(), min_progress_step)
        }));
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
                log("wgen=>Done");
                ThreadMessage::GeneratorDone(generation, wgen.get_export_map())
            }
            Ok(()) => {
                log(&format!("wgen=>GeneratorStepDone({})", index));
                ThreadMessage::GeneratorStepDone(
                    generation,
                    index,
                    if live {
                        Some(wgen.get_step_export_map(index))
                    } else {
                        None
                    },
                )
            }
        };
        let _ = tx.send(msg);
        // wake the UI thread so the message is handled without waiting for user input
        ctx.request_repaint();
    }
}

impl WorldGenerator {
    pub fn new(seed: u64, world_size: (usize, usize)) -> Self {
        Self {
            seed,
            world_size,
            hmap: Vec::new(),
        }
    }
    pub fn get_export_map(&self) -> ExportMap {
        self.get_step_export_map(if self.hmap.is_empty() {
            0
        } else {
            self.hmap.len() - 1
        })
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
        *self = WorldGenerator::new(self.seed, self.world_size);
    }


    /// preview path : (re)computes step `index` from the output of step `index - 1`, keeping every
    /// step's map so a later step can be recomputed alone
    fn execute_step(
        &mut self,
        index: usize,
        step: &Step,
        tx: Sender<ThreadMessage>,
        min_progress_step: f32,
    ) {
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
            self.run_step(step, &mut cur, prev, false, tx, min_progress_step);
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
            self.run_step(
                step,
                &mut cur,
                prev.as_deref(),
                true,
                tx.clone(),
                min_progress_step,
            );
            let _ = tx.send(ThreadMessage::ExporterStepDone(i));
        }
        self.hmap.push(HMap { h: cur });
    }

    /// runs one step's generator on `h` (already holding the previous step's output), then blends
    /// the result with `prev` through the step's mask, if any
    fn run_step(
        &self,
        step: &Step,
        h: &mut [f32],
        prev: Option<&[f32]>,
        export: bool,
        tx: Sender<ThreadMessage>,
        min_progress_step: f32,
    ) {
        let (seed, size) = (self.seed, self.world_size);
        if !step.disabled {
            match &step.typ {
                StepType::Hills(conf) => gen_hills(seed, size, h, conf, export, tx, min_progress_step),
                StepType::Fbm(conf) => gen_fbm(seed, size, h, conf, export, tx, min_progress_step),
                StepType::MidPoint(conf) => {
                    gen_mid_point(seed, size, h, conf, export, tx, min_progress_step)
                }
                StepType::Normalize(conf) => gen_normalize(h, conf),
                StepType::LandMass(conf) => {
                    gen_landmass(size, h, conf, export, tx, min_progress_step)
                }
                StepType::MudSlide(conf) => {
                    gen_mudslide(size, h, conf, export, tx, min_progress_step)
                }
                StepType::WaterErosion(conf) => {
                    gen_water_erosion(seed, size, h, conf, export, tx, min_progress_step)
                }
                StepType::Island(conf) => gen_island(size, h, conf, export, tx, min_progress_step),
            }
        }
        if let Some(ref mask) = step.mask {
            apply_mask(size, mask, prev, h);
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


fn apply_mask(world_size: (usize, usize), mask: &[f32], prev: Option<&[f32]>, h: &mut [f32]) {
    let mut off = 0;
    let (min, _) = if prev.is_none() {
        get_min_max(h)
    } else {
        (0.0, 0.0)
    };
    for y in 0..world_size.1 {
        let myf = (y * MASK_SIZE) as f32 / world_size.1 as f32;
        let my = myf as usize;
        let yalpha = myf.fract();
        for x in 0..world_size.0 {
            let mxf = (x * MASK_SIZE) as f32 / world_size.0 as f32;
            let mx = mxf as usize;
            let xalpha = mxf.fract();
            let mut mask_value = mask[mx + my * MASK_SIZE];
            if mx + 1 < MASK_SIZE {
                mask_value = (1.0 - xalpha) * mask_value + xalpha * mask[mx + 1 + my * MASK_SIZE];
                if my + 1 < MASK_SIZE {
                    let bottom_left_mask = mask[mx + (my + 1) * MASK_SIZE];
                    let bottom_right_mask = mask[mx + 1 + (my + 1) * MASK_SIZE];
                    let bottom_mask =
                        (1.0 - xalpha) * bottom_left_mask + xalpha * bottom_right_mask;
                    mask_value = (1.0 - yalpha) * mask_value + yalpha * bottom_mask;
                }
            }
            if let Some(prev) = prev {
                h[off] = (1.0 - mask_value) * prev[off] + mask_value * h[off];
            } else {
                h[off] = (1.0 - mask_value) * min + mask_value * (h[off] - min);
            }
            off += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::{FbmConf, HillsConf, NormalizeConf};
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
            preview.execute_step(i, step, tx.clone(), 1.0);
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
        let (tx, _) = mpsc::channel();
        let mut wgen = WorldGenerator::new(1, (8, 8));
        let step = Step::default();
        wgen.execute_step(2, &step, tx, 1.0);
        assert_eq!(wgen.hmap.len(), 3);
    }

    #[test]
    fn delete_step_past_end_is_ignored() {
        let (tx, _) = mpsc::channel();
        let mut wgen = WorldGenerator::new(1, (8, 8));
        let mut queue = Vec::new();
        do_command(WorldGenCommand::DeleteStep(3), &mut wgen, &mut queue, &tx);
        assert!(wgen.hmap.is_empty());
        for i in 0..3 {
            do_command(
                WorldGenCommand::ExecuteStep(1, i, Step::default(), false, 1.0),
                &mut wgen,
                &mut queue,
                &tx,
            );
        }
        do_command(WorldGenCommand::Abort(1), &mut wgen, &mut queue, &tx);
        assert_eq!(queue.len(), 1);
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
            apply_mask((w, h), &mask, None, &mut hmap);
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
}
