use std::sync::mpsc::Sender;
use std::time::Instant;
use std::{fmt::Display, sync::mpsc::Receiver};

use serde::{Deserialize, Serialize};

use crate::generators::{
    gen_fbm, gen_hills, gen_island, gen_landmass, gen_mid_point, gen_mudslide, gen_normalize,
    gen_water_erosion, get_min_max, FbmConf, HillsConf, IslandConf, LandMassConf, MidPointConf,
    MudSlideConf, NormalizeConf, WaterErosionConf,
};
use crate::{log, ThreadMessage};

#[derive(Debug)]
pub enum WorldGenCommand {
    /// step index, disabled, step conf, live preview, min progress step
    ExecuteStep(usize, bool, Step, bool, f32),
    DeleteStep(usize),
    EnableStep(usize),
    DisableStep(usize),
    SetSize(usize),
    GetStepMap(usize),
    SetSeed(u64),
    Abort,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum Step {
    Hills(HillsConf),
    Fbm(FbmConf),
    Normalize(NormalizeConf),
    LandMass(LandMassConf),
    MudSlide(MudSlideConf),
    WaterErosion(WaterErosionConf),
    Island(IslandConf),
    MidPoint(MidPointConf),
}

impl Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let debug_val = format!("{:?}", self);
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
    disabled: bool,
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
        WorldGenCommand::SetSeed(new_seed) => {
            wgen.seed = new_seed;
        }
        WorldGenCommand::ExecuteStep(index, disabled, step, live, min_progress_step) => {
            steps.push(InnerStep {
                index,
                disabled,
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
            if let Some(msg) = rx.recv().ok() {
                let tx = tx.clone();
                do_command(msg, &mut wgen, &mut steps, tx);
            }
        }
        while let Some(msg) = rx.try_recv().ok() {
            let tx = tx.clone();
            do_command(msg, &mut wgen, &mut steps, tx);
        }
        if !steps.is_empty() {
            let InnerStep {
                index,
                disabled,
                step,
                live,
                min_progress_step,
            } = steps.remove(0);
            let tx2 = tx.clone();
            wgen.execute_step(index, disabled, &step, false, tx2, min_progress_step);
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
        ExportMap {
            size: self.world_size,
            h: if self.hmap.is_empty() {
                vec![0.0; self.world_size.0 * self.world_size.1]
            } else {
                self.hmap[self.hmap.len() - 1].h.clone()
            },
        }
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
        if !self.hmap.is_empty() {
            if off < self.world_size.0 * self.world_size.1 {
                return self.hmap[self.hmap.len() - 1].h[off];
            }
        }
        0.0
    }
    pub fn clear(&mut self) {
        *self = WorldGenerator::new(self.seed, self.world_size);
    }

    fn execute_step(
        &mut self,
        index: usize,
        disabled: bool,
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
        } else {
            if index > 0 {
                self.hmap[index].h = self.hmap[index - 1].h.clone();
            } else {
                self.hmap[index].h.fill(0.0);
            }
        }
        let hmap = &mut self.hmap[index];
        hmap.disabled = disabled;
        if !hmap.disabled {
            match step {
                Step::Hills(conf) => gen_hills(
                    self.seed,
                    self.world_size,
                    &mut hmap.h,
                    conf,
                    export,
                    tx,
                    min_progress_step,
                ),
                Step::Fbm(conf) => gen_fbm(
                    self.seed,
                    self.world_size,
                    &mut hmap.h,
                    conf,
                    export,
                    tx,
                    min_progress_step,
                ),
                Step::MidPoint(conf) => gen_mid_point(
                    self.seed,
                    self.world_size,
                    &mut hmap.h,
                    conf,
                    export,
                    tx,
                    min_progress_step,
                ),
                Step::Normalize(conf) => gen_normalize(&mut hmap.h, conf),
                Step::LandMass(conf) => gen_landmass(
                    self.world_size,
                    &mut hmap.h,
                    conf,
                    export,
                    tx,
                    min_progress_step,
                ),
                Step::MudSlide(conf) => gen_mudslide(
                    self.world_size,
                    &mut hmap.h,
                    conf,
                    export,
                    tx,
                    min_progress_step,
                ),
                Step::WaterErosion(conf) => gen_water_erosion(
                    self.world_size,
                    &mut hmap.h,
                    conf,
                    export,
                    tx,
                    min_progress_step,
                ),
                Step::Island(conf) => gen_island(
                    self.world_size,
                    &mut hmap.h,
                    conf,
                    export,
                    tx,
                    min_progress_step,
                ),
            }
            log(&format!(
                "Executed {} in {:.2}s",
                step,
                now.elapsed().as_secs_f32()
            ));
        }
    }

    pub fn generate(
        &mut self,
        steps: &[Step],
        disabled: &[bool],
        tx: Sender<ThreadMessage>,
        min_progress_step: f32,
    ) {
        self.clear();
        for (i, step) in steps.iter().enumerate() {
            let tx2 = tx.clone();
            self.execute_step(i, disabled[i], step, true, tx2, min_progress_step);
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
