extern crate exr;
extern crate image;
extern crate noise;
extern crate rand;

mod exporter;
mod fps;
mod generators;
mod panel_2dview;
mod panel_3dview;
mod panel_export;
mod panel_generator;
mod panel_maskedit;
mod panel_save;
mod project;
mod step;
mod worldgen;

use eframe::egui::{self, Visuals};
use epaint::emath;
use exporter::export_heightmap;
use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use panel_2dview::{Panel2dAction, Panel2dView};
use panel_3dview::Panel3dView;
use panel_export::PanelExport;
use panel_generator::{GeneratorAction, PanelGenerator};
use panel_save::{PanelSaveLoad, SaveLoadAction};
use project::Project;
use worldgen::{generator_thread, ExportMap, Invalidation, WorldGenCommand, WorldGenerator};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MASK_SIZE: usize = 64;

/// messages sent to the main thread by either world generator or exporter threads
pub enum ThreadMessage {
    /// from world generator : all steps of this generation have been computed => update 2D/3D previews
    GeneratorDone(u64, ExportMap),
    /// from world generator : update progress bar
    GeneratorStepProgress(f32),
    /// from world generator : one step of this generation has been computed => update 2D preview if live preview enabled
    GeneratorStepDone(u64, usize, Option<ExportMap>),
    /// from world generator : the heightmap of a specific step, tagged with the generation that asked for it
    GeneratorStepMap(u64, usize, ExportMap),
    /// from world generator : a step panicked, the generation is abandoned
    GeneratorError(String),
    /// from exporter : one step has been computed
    ExporterStepDone(usize),
    /// from exporter : export is finished
    ExporterDone(Result<(), String>),
    /// from exporter : update progress bar
    ExporterStepProgress(f32),
}

fn main() {
    let options = eframe::NativeOptions {
        multisampling: 8,
        depth_buffer: 24,
        renderer: eframe::Renderer::Glow,
        vsync: true,
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };
    log(&format!(
        "wgen v{} - {} cpus {} cores",
        VERSION,
        num_cpus::get(),
        num_cpus::get_physical()
    ));
    eframe::run_native(
        "wgen",
        options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
    .or_else(|e| {
        eprintln!("Error: {}", e);
        Ok::<(), ()>(())
    })
    .ok();
}

struct MyApp {
    /// size in pixels of the 2D preview canvas
    image_size: usize,
    /// size of the preview heightmap (from 64x64 to 512x512)
    preview_size: usize,
    /// current world generator progress
    progress: f32,
    /// exporter progress
    exporter_progress: f32,
    /// exporter progress bar text
    exporter_text: String,
    /// exporter current step
    exporter_cur_step: usize,
    /// random number generator's seed
    seed: u64,
    /// bumped by every `regen`; generator messages tagged with an older value are stale and dropped
    generation: u64,
    /// shared with the generator thread: tells the running step whether a regen made it stale
    invalidation: Invalidation,
    /// labels of the steps being exported, captured when the export started
    export_step_names: Vec<String>,
    // ui widgets
    gen_panel: PanelGenerator,
    export_panel: PanelExport,
    panel_3d: Panel3dView,
    panel_2d: Panel2dView,
    load_save_panel: PanelSaveLoad,
    // thread communication
    /// channel to receive messages from either world generator or exporter
    thread2main_rx: Receiver<ThreadMessage>,
    /// channel to send messages to the world generator thread
    main2wgen_tx: Sender<WorldGenCommand>,
    /// channel to send messages to the main thread from the exporter thread
    exp2main_tx: Sender<ThreadMessage>,
    /// an error to display in a popup
    err_msg: Option<String>,
}

impl MyApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let preview_size = 128;
        let image_size = 790; //368;
        let seed = 0xdeadbeef;
        let wgen = WorldGenerator::new(seed, (preview_size, preview_size));
        let panel_2d = Panel2dView::new(image_size, preview_size as u32, &wgen.get_export_map());
        // generator -> main channel
        let (exp2main_tx, thread2main_rx) = mpsc::channel();
        // main -> generator channel
        let (main2gen_tx, gen_rx) = mpsc::channel();
        let gen_tx = exp2main_tx.clone();
        let ctx = cc.egui_ctx.clone();
        let invalidation = Invalidation::default();
        let thread_invalidation = invalidation.clone();
        thread::spawn(move || {
            generator_thread(seed, preview_size, gen_rx, gen_tx, ctx, thread_invalidation);
        });
        Self {
            image_size,
            preview_size,
            seed,
            generation: 0,
            invalidation,
            export_step_names: Vec::new(),
            panel_2d,
            panel_3d: Panel3dView::new(image_size as f32),
            progress: 1.0,
            exporter_progress: 1.0,
            exporter_text: String::new(),
            exporter_cur_step: 0,
            gen_panel: PanelGenerator::default(),
            export_panel: PanelExport::default(),
            load_save_panel: PanelSaveLoad::default(),
            thread2main_rx,
            main2wgen_tx: main2gen_tx,
            exp2main_tx,
            err_msg: None,
        }
    }
}

impl MyApp {
    fn export(&mut self) {
        let steps = self.gen_panel.steps.clone();
        self.export_step_names = steps.iter().map(|s| s.to_string()).collect();
        let export_panel = self.export_panel.clone();
        let seed = self.seed;
        let tx = self.exp2main_tx.clone();
        let min_progress_step = 0.01 * self.gen_panel.enabled_steps() as f32;
        thread::spawn(move || {
            // a panic must still end the export, or the export panel stays disabled forever
            let res = catch_unwind(AssertUnwindSafe(|| {
                export_heightmap(seed, &steps, &export_panel, tx.clone(), min_progress_step)
            }))
            .unwrap_or_else(|payload| Err(panic_message(payload.as_ref())));
            let _ = tx.send(ThreadMessage::ExporterDone(res));
        });
    }
    /// the single entry point to recompute the stack from a step; every edit routes through it.
    /// `delete` is a step the generator still holds and the panel no longer has.
    fn regen(&mut self, delete: Option<usize>, from_idx: usize) {
        self.generation += 1;
        self.invalidation.set(self.generation, from_idx);
        self.main2wgen_tx
            .send(WorldGenCommand::Abort(from_idx))
            .unwrap();
        let len = self.gen_panel.steps.len();
        if let Some(index) = delete {
            self.main2wgen_tx
                .send(WorldGenCommand::DeleteStep(index))
                .unwrap();
        }
        if len == 0 {
            // nothing to run : no GeneratorDone will come back for this generation
            self.gen_panel.is_running = false;
            self.progress = 1.0;
            return;
        }
        let enabled = self.gen_panel.enabled_steps().max(1) as f32;
        self.progress = from_idx as f32 / enabled;
        for i in from_idx.min(len - 1)..len {
            self.main2wgen_tx
                .send(WorldGenCommand::ExecuteStep(
                    self.generation,
                    i,
                    self.gen_panel.steps[i].clone(),
                    self.panel_2d.live_preview,
                    0.01 * enabled,
                ))
                .unwrap();
        }
        self.gen_panel.is_running = true;
    }
    fn set_seed(&mut self, new_seed: u64) {
        self.seed = new_seed;
        self.main2wgen_tx
            .send(WorldGenCommand::SetSeed(new_seed))
            .unwrap();
        self.regen(None, 0);
    }
    fn resize(&mut self, new_size: usize) {
        if self.preview_size == new_size {
            return;
        }
        self.preview_size = new_size;
        self.main2wgen_tx
            .send(WorldGenCommand::SetSize(new_size))
            .unwrap();
        self.regen(None, 0);
    }
    fn render_left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("Generation").show(ctx, |ui| {
            ui.label(format!("wgen {}", VERSION));
            ui.separator();
            if self
                .export_panel
                .render(ui, self.exporter_progress, &self.exporter_text)
            {
                self.export_panel.enabled = false;
                self.exporter_progress = 0.0;
                self.exporter_cur_step = 0;
                self.export();
            }
            ui.separator();
            match self.load_save_panel.render(ui) {
                Some(SaveLoadAction::Load) => {
                    match Project::load(self.load_save_panel.get_file_path()) {
                        Ok(project) => {
                            self.gen_panel.load_project(project);
                            self.main2wgen_tx.send(WorldGenCommand::Clear).unwrap();
                            self.set_seed(self.gen_panel.seed);
                        }
                        Err(msg) => {
                            let err_msg = format!(
                                "Error while reading project {} : {}",
                                self.load_save_panel.get_file_path(),
                                msg
                            );
                            log(&err_msg);
                            self.err_msg = Some(err_msg);
                        }
                    }
                }
                Some(SaveLoadAction::Save) => {
                    if let Err(msg) = self
                        .gen_panel
                        .project()
                        .save(self.load_save_panel.get_file_path())
                    {
                        let err_msg = format!(
                            "Error while writing project {} : {}",
                            self.load_save_panel.get_file_path(),
                            msg
                        );
                        log(&err_msg);
                        self.err_msg = Some(err_msg);
                    }
                }
                None => (),
            }
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                match self.gen_panel.render(ui, self.progress) {
                    Some(GeneratorAction::Clear) => {
                        // drop queued steps too; the running one stops at its next progress report
                        self.generation += 1;
                        self.invalidation.set(self.generation, 0);
                        self.main2wgen_tx.send(WorldGenCommand::Abort(0)).unwrap();
                        self.main2wgen_tx.send(WorldGenCommand::Clear).unwrap();
                        self.gen_panel.is_running = false;
                        self.progress = 1.0;
                    }
                    Some(GeneratorAction::SetSeed(new_seed)) => {
                        self.set_seed(new_seed);
                    }
                    Some(GeneratorAction::Regen { delete, from }) => {
                        self.regen(delete, from);
                    }
                    Some(GeneratorAction::DisplayLayer(step)) => {
                        self.main2wgen_tx
                            .send(WorldGenCommand::GetStepMap(self.generation, step))
                            .unwrap();
                    }
                    Some(GeneratorAction::DisplayMask(mask)) => {
                        self.panel_2d
                            .display_mask(self.image_size, self.preview_size as u32, mask);
                    }
                    None => (),
                }
            });
        });
        // the generator panel owns the mask session; the 2D preview follows it
        if !self.gen_panel.is_editing_mask() {
            self.panel_2d.exit_mask_mode();
        }
    }
    fn render_central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Terrain preview");
            ui.horizontal(|ui| {
                egui::CollapsingHeader::new("2d preview")
                    .default_open(true)
                    .show(ui, |ui| match self.panel_2d.render(ui) {
                        Some(Panel2dAction::ResizePreview(new_size)) => {
                            self.gen_panel.exit_mask_mode();
                            self.resize(new_size);
                        }
                        Some(Panel2dAction::MaskCommitted(mask)) => {
                            if let Some(from) = self.gen_panel.commit_mask(mask) {
                                self.regen(None, from);
                            }
                        }
                        Some(Panel2dAction::MaskDelete) => {
                            if let Some(from) = self.gen_panel.delete_mask() {
                                self.regen(None, from);
                            }
                        }
                        None => (),
                    });
                egui::CollapsingHeader::new("3d preview")
                    .default_open(true)
                    .show(ui, |ui| {
                        self.panel_3d.render(ui);
                    });
            });
        });
    }
    fn handle_threads_messages(&mut self) {
        while let Ok(msg) = self.thread2main_rx.try_recv() {
            self.handle_thread_message(msg);
        }
    }
    fn handle_thread_message(&mut self, msg: ThreadMessage) {
        match msg {
            ThreadMessage::GeneratorStepProgress(progress) => {
                let progstep = 1.0 / self.gen_panel.enabled_steps().max(1) as f32;
                self.progress = (self.progress / progstep).floor() * progstep;
                self.progress += progress * progstep;
            }
            ThreadMessage::GeneratorDone(generation, hmap) => {
                if generation != self.generation {
                    return;
                }
                log("main<=Done");
                self.panel_2d
                    .refresh(self.image_size, self.preview_size as u32, Some(&hmap));
                self.gen_panel.selected_step = self.gen_panel.steps.len().saturating_sub(1);
                self.panel_3d.update_mesh(&hmap);
                self.gen_panel.is_running = false;
                self.progress = 1.0;
            }
            ThreadMessage::GeneratorStepDone(generation, step, hmap) => {
                if generation != self.generation {
                    return;
                }
                log(&format!("main<=GeneratorStepDone({})", step));
                if let Some(ref hmap) = hmap {
                    self.panel_2d
                        .refresh(self.image_size, self.preview_size as u32, Some(hmap));
                }
                self.gen_panel.selected_step =
                    step.min(self.gen_panel.steps.len().saturating_sub(1));
                self.progress = (step + 1) as f32 / self.gen_panel.enabled_steps().max(1) as f32
            }
            ThreadMessage::GeneratorStepMap(generation, _idx, hmap) => {
                if generation != self.generation {
                    return;
                }
                // display heightmap from a specific step in the 2d preview
                self.panel_2d
                    .refresh(self.image_size, self.preview_size as u32, Some(&hmap));
            }
            ThreadMessage::GeneratorError(msg) => {
                let err_msg = format!("Error while generating heightmap : {}", msg);
                log(&err_msg);
                self.err_msg = Some(err_msg);
                self.gen_panel.is_running = false;
                self.progress = 1.0;
            }
            ThreadMessage::ExporterStepProgress(progress) => {
                let progstep = 1.0 / self.export_step_names.len().max(1) as f32;
                self.exporter_progress = (self.exporter_progress / progstep).floor() * progstep;
                self.exporter_progress += progress * progstep;
                self.exporter_text = self.export_progress_text();
            }
            ThreadMessage::ExporterStepDone(step) => {
                log(&format!("main<=ExporterStepDone({})", step));
                self.exporter_progress =
                    (step + 1) as f32 / self.export_step_names.len().max(1) as f32;
                self.exporter_cur_step = step + 1;
                if step + 1 >= self.export_step_names.len() {
                    self.exporter_text = format!("Saving {}...", self.export_panel.file_type);
                } else {
                    self.exporter_text = self.export_progress_text();
                }
            }
            ThreadMessage::ExporterDone(res) => {
                if let Err(msg) = res {
                    let err_msg = format!("Error while exporting heightmap : {}", msg);
                    log(&err_msg);
                    self.err_msg = Some(err_msg);
                }
                log("main<=ExporterDone");
                self.exporter_progress = 1.0;
                self.export_panel.enabled = true;
                self.exporter_cur_step = 0;
                self.exporter_text = String::new();
                self.export_step_names.clear();
            }
        }
    }
    /// progress bar text during export, from the step list captured when the export started
    fn export_progress_text(&self) -> String {
        let name = self
            .export_step_names
            .get(self.exporter_cur_step)
            .map_or("", String::as_str);
        format!(
            "{}% {}/{} {}",
            (self.exporter_progress * 100.0) as usize,
            self.exporter_cur_step + 1,
            self.export_step_names.len(),
            name
        )
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let wsize = ctx.input(|i| {
            if let Some(rect) = i.viewport().inner_rect {
                rect.size()
            } else {
                emath::Vec2::new(0.0, 0.0)
            }
        });
        let new_size = ((wsize.x - 340.0) * 0.5) as usize;
        if new_size != self.image_size && new_size != 0 {
            // handle window resizing : both previews keep their data and only change size
            self.image_size = new_size;
            self.panel_2d
                .refresh(self.image_size, self.preview_size as u32, None);
            self.panel_3d.set_size(self.image_size as f32);
        }
        ctx.set_visuals(Visuals::dark());
        self.handle_threads_messages();
        if self.gen_panel.is_running || !self.export_panel.enabled {
            // poll the channel while a worker thread runs, whatever widgets are on screen
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.render_left_panel(ctx);
        self.render_central_panel(ctx);

        if let Some(ref err_msg) = self.err_msg {
            // display error popup
            let mut open = true;
            egui::Window::new("Error")
                .resizable(false)
                .collapsible(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.scope(|ui| {
                        ui.visuals_mut().override_text_color = Some(egui::Color32::RED);
                        ui.label(err_msg);
                    });
                });
            if !open {
                self.err_msg = None;
            }
        }
    }
}

/// timestamped stdout log; the clock starts at the first call, on any thread
pub fn log(msg: &str) {
    static LOGTIME: OnceLock<Instant> = OnceLock::new();
    let elapsed = LOGTIME.get_or_init(Instant::now).elapsed();
    println!("{:03.3} {}", elapsed.as_millis() as f32 / 1000.0, msg);
}

/// the text of a panic payload, for error popups
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}
