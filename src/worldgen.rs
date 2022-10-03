use std::sync::mpsc::Sender;
use std::time::Instant;
use std::{fmt::Display, sync::mpsc::Receiver};

use serde::{Deserialize, Serialize};

use crate::generators::{
    gen_fbm, gen_hills, gen_island, gen_landmass, gen_mid_point, gen_mudslide, gen_normalize,
    gen_water_erosion, get_min_max, FbmConf, HillsConf, IslandConf, LandMassConf, MidPointConf,
    MudSlideConf, NormalizeConf, WaterErosionConf,
};
use crate::{log, ThreadMessage, MASK_SIZE};

#[derive(Debug)]
pub enum WorldGenCommand {
    /// step index, disabled, step conf, live preview, min progress step
    ExecuteStep(usize, Step, bool, f32),
    DeleteStep(usize),
    EnableStep(usize),
    DisableStep(usize),
    SetSize(usize),
    GetStepMap(usize),
    SetSeed(u64),
    Clear,
    Abort,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
/// Each value contains its own configuration, whether this step is disabled and an optional mask
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
    pub disabled: bool,
    pub mask: Option<Vec<f32>>,
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
    disabled: bool,
}

#[derive(Clone)]
pub struct WorldGenerator {
    seed: u64,
    world_size: (usize, usize),
    hmap: Vec<HMap>,
}

struct InnerStep {
    index: usize,
    step: Step,
    live: bool,
    min_progress_step: f32,
}

fn do_command(
    msg: WorldGenCommand,
    wgen: &mut WorldGenerator,
    steps: &mut Vec<InnerStep>,
    tx: Sender<ThreadMessage>,
) {
    log(&format!("wgen<={:?}", msg));
    match msg {
        WorldGenCommand::Clear => {
            wgen.clear();
        }
        WorldGenCommand::SetSeed(new_seed) => {
            wgen.seed = new_seed;
        }
        WorldGenCommand::ExecuteStep(index, step, live, min_progress_step) => {
            steps.push(InnerStep {
                index,
                step,
                live,
                min_progress_step,
            });
        }
        WorldGenCommand::DeleteStep(index) => {
            wgen.hmap.remove(index);
        }
        WorldGenCommand::DisableStep(index) => {
            wgen.hmap[index].disabled = true;
        }
        WorldGenCommand::EnableStep(index) => {
            wgen.hmap[index].disabled = false;
        }
        WorldGenCommand::GetStepMap(index) => tx
            .send(ThreadMessage::GeneratorStepMap(
                index,
                wgen.get_step_export_map(index),
            ))
            .unwrap(),
        WorldGenCommand::Abort => {
            steps.clear();
        }
        WorldGenCommand::SetSize(size) => {
            *wgen = WorldGenerator::new(wgen.seed, (size, size));
        }
    }
}

pub fn generator_thread(
    seed: u64,
    size: usize,
    rx: Receiver<WorldGenCommand>,
    tx: Sender<ThreadMessage>,
) {
    let mut wgen = WorldGenerator::new(seed, (size, size));
    let mut steps = Vec::new();
    loop {
        if steps.is_empty() {
            // blocking wait
            if let Ok(msg) = rx.recv() {
                let tx = tx.clone();
                do_command(msg, &mut wgen, &mut steps, tx);
            }
        }
        while let Ok(msg) = rx.try_recv() {
            let tx = tx.clone();
            do_command(msg, &mut wgen, &mut steps, tx);
        }
        if !steps.is_empty() {
            let InnerStep {
                index,
                step,
                live,
                min_progress_step,
            } = steps.remove(0);
            let tx2 = tx.clone();
            wgen.execute_step(index, &step, false, tx2, min_progress_step);
            if steps.is_empty() {
                log("wgen=>Done");
                tx.send(ThreadMessage::GeneratorDone(wgen.get_export_map()))
                    .unwrap();
            } else {
                log(&format!("wgen=>GeneratorStepDone({})", index));
                tx.send(ThreadMessage::GeneratorStepDone(
                    index,
                    if live {
                        Some(wgen.get_step_export_map(index))
                    } else {
                        None
                    },
                ))
                .unwrap();
            }
        }
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

    fn execute_step(
        &mut self,
        index: usize,
        step: &Step,
        export: bool,
        tx: Sender<ThreadMessage>,
        min_progress_step: f32,
    ) {
        let now = Instant::now();
        let len = self.hmap.len();
        if index >= len {
            let vecsize = self.world_size.0 * self.world_size.1;
            self.hmap.push(if len == 0 {
                HMap {
                    h: vec![0.0; vecsize],
                    disabled: false,
                }
            } else {
                HMap {
                    h: self.hmap[len - 1].h.clone(),
                    disabled: false,
                }
            });
        } else if index > 0 {
            self.hmap[index].h = self.hmap[index - 1].h.clone();
        } else {
            self.hmap[index].h.fill(0.0);
        }
        {
            let hmap = &mut self.hmap[index];
            match step {
                Step {
                    typ: StepType::Hills(conf),
                    disabled,
                    ..
                } => {
                    if !*disabled {
                        gen_hills(
                            self.seed,
                            self.world_size,
                            &mut hmap.h,
                            conf,
                            export,
                            tx,
                            min_progress_step,
                        );
                    }
                }
                Step {
                    typ: StepType::Fbm(conf),
                    disabled,
                    ..
                } => {
                    if !*disabled {
                        gen_fbm(
                            self.seed,
                            self.world_size,
                            &mut hmap.h,
                            conf,
                            export,
                            tx,
                            min_progress_step,
                        );
                    }
                }
                Step {
                    typ: StepType::MidPoint(conf),
                    disabled,
                    ..
                } => {
                    if !*disabled {
                        gen_mid_point(
                            self.seed,
                            self.world_size,
                            &mut hmap.h,
                            conf,
                            export,
                            tx,
                            min_progress_step,
                        );
                    }
                }
                Step {
                    typ: StepType::Normalize(conf),
                    disabled,
                    ..
                } => {
                    if !*disabled {
                        gen_normalize(&mut hmap.h, conf);
                    }
                }
                Step {
                    typ: StepType::LandMass(conf),
                    disabled,
                    ..
                } => {
                    if !*disabled {
                        gen_landmass(
                            self.world_size,
                            &mut hmap.h,
                            conf,
                            export,
                            tx,
                            min_progress_step,
                        );
                    }
                }
                Step {
                    typ: StepType::MudSlide(conf),
                    disabled,
                    ..
                } => {
                    if !*disabled {
                        gen_mudslide(
                            self.world_size,
                            &mut hmap.h,
                            conf,
                            export,
                            tx,
                            min_progress_step,
                        );
                    }
                }
                Step {
                    typ: StepType::WaterErosion(conf),
                    disabled,
                    ..
                } => {
                    if !*disabled {
                        gen_water_erosion(
                            self.seed,
                            self.world_size,
                            &mut hmap.h,
                            conf,
                            export,
                            tx,
                            min_progress_step,
                        );
                    }
                }
                Step {
                    typ: StepType::Island(conf),
                    disabled,
                    ..
                } => {
                    if !*disabled {
                        gen_island(
                            self.world_size,
                            &mut hmap.h,
                            conf,
                            export,
                            tx,
                            min_progress_step,
                        );
                    }
                }
            }
        }
        if let Some(ref mask) = step.mask {
            if index > 0 {
                let prev = self.hmap[index - 1].h.clone();
                apply_mask(self.world_size, mask, Some(&prev), &mut self.hmap[index].h);
            } else {
                apply_mask(self.world_size, mask, None, &mut self.hmap[index].h);
            }
        }

        log(&format!(
            "Executed {} in {:.2}s",
            step,
            now.elapsed().as_secs_f32()
        ));
    }

    pub fn generate(&mut self, steps: &[Step], tx: Sender<ThreadMessage>, min_progress_step: f32) {
        self.clear();
        for (i, step) in steps.iter().enumerate() {
            let tx2 = tx.clone();
            self.execute_step(i, step, true, tx2, min_progress_step);
            tx.send(ThreadMessage::ExporterStepDone(i)).unwrap();
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
        let myf = (y * MASK_SIZE) as f32 / world_size.0 as f32;
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
