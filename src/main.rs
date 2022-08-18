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
mod panel_save;
mod worldgen;

use eframe::egui::{self, Visuals};
use exporter::export_heightmap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use panel_2dview::{Panel2dAction, Panel2dView};
use panel_3dview::Panel3dView;
use panel_export::PanelExport;
use panel_generator::{GeneratorAction, PanelGenerator};
use panel_save::{PanelSaveLoad, SaveLoadAction};
use worldgen::{generator_thread, ExportMap, WorldGenCommand, WorldGenerator};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub enum ThreadMessage {
    GeneratorDone(ExportMap),
    GeneratorStepProgress(f32),
    GeneratorStepDone(usize, Option<ExportMap>),
    GeneratorStepMap(usize, ExportMap),
    ExporterStepDone(usize),
    ExporterDone(Result<(), String>),
    ExporterStepProgress(f32),
}

fn main() {
    let options = eframe::NativeOptions {
        maximized: true,
        multisampling: 8,
        depth_buffer: 24,
        ..Default::default()
    };
    eframe::run_native("wgen", options, Box::new(|_cc| Box::new(MyApp::default())));
}

struct MyApp {
    enabled: bool,
    image_size: usize,
    preview_size: usize,
    progress: f32,
    exporter_progress: f32,
    exporter_text: String,
    exporter_cur_step: usize,
    seed: u64,
    gen_panel: PanelGenerator,
    export_panel: PanelExport,
    panel_3d: Panel3dView,
    panel_2d: Panel2dView,
    load_save_panel: PanelSaveLoad,
    thread2main_rx: Receiver<ThreadMessage>,
    main2gen_tx: Sender<WorldGenCommand>,
    thread2main_tx: Sender<ThreadMessage>,
}

impl Default for MyApp {
    fn default() -> Self {
        let preview_size = 128;
        let image_size = 790; //368;
        let seed = 0xdeadbeef;
        let wgen = WorldGenerator::new(seed, (preview_size, preview_size));
        let panel_2d = Panel2dView::new(image_size, preview_size as u32, &wgen.get_export_map());
        // generator -> main channel
        let (thread2main_tx, thread2main_rx) = mpsc::channel();
        // main -> generator channel
        let (main2gen_tx, gen_rx) = mpsc::channel();
        let gen_tx = thread2main_tx.clone();
        thread::spawn(move || {
            generator_thread(seed, preview_size, gen_rx, gen_tx);
        });
        Self {
            enabled: true,
            image_size,
            preview_size,
            seed,
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
            main2gen_tx,
            thread2main_tx,
        }
    }
}

impl MyApp {
    fn export(&mut self) {
        let steps = self.gen_panel.steps.clone();
        let disabled = self.gen_panel.disabled.clone();
        let export_panel = self.export_panel.clone();
        let seed = self.seed;
        let tx = self.thread2main_tx.clone();
        let min_progress_step = 0.01 * self.gen_panel.enabled_steps() as f32;
        thread::spawn(move || {
            let res = export_heightmap(
                seed,
                &steps,
                &disabled,
                &export_panel,
                tx.clone(),
                min_progress_step,
            );
            tx.send(ThreadMessage::ExporterDone(res)).unwrap();
        });
    }
    fn regen(&mut self, must_delete: bool, from_idx: usize) {
        self.progress = from_idx as f32 / self.gen_panel.enabled_steps() as f32;
        self.main2gen_tx.send(WorldGenCommand::Abort).unwrap();
        let len = self.gen_panel.steps.len();
        if must_delete {
            self.main2gen_tx
                .send(WorldGenCommand::DeleteStep(from_idx))
                .unwrap();
        }
        if len == 0 {
            return;
        }
        for i in from_idx.min(len - 1)..len {
            self.main2gen_tx
                .send(WorldGenCommand::ExecuteStep(
                    i,
                    self.gen_panel.disabled[i],
                    self.gen_panel.steps[i].clone(),
                    self.panel_2d.live_preview,
                    0.01 * self.gen_panel.enabled_steps() as f32,
                ))
                .unwrap();
        }
        self.gen_panel.is_running = true;
    }
    fn set_seed(&mut self, new_seed: u64) {
        self.seed = new_seed;
        self.main2gen_tx
            .send(WorldGenCommand::SetSeed(new_seed))
            .unwrap();
        self.regen(false, 0);
    }
    fn resize(&mut self, new_size: usize) {
        if self.preview_size == new_size {
            return;
        }
        self.preview_size = new_size;
        self.main2gen_tx
            .send(WorldGenCommand::SetSize(new_size))
            .unwrap();
        self.regen(false, 0);
    }
    fn render_left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("Generation").show(ctx, |ui| {
            ui.add_enabled_ui(self.enabled, |ui| {
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
                        self.gen_panel.load(&self.load_save_panel.get_file_path());
                        self.main2gen_tx.send(WorldGenCommand::Clear).unwrap();
                        self.set_seed(self.gen_panel.seed);
                    }
                    Some(SaveLoadAction::Save) => {
                        self.gen_panel.save(&self.load_save_panel.get_file_path());
                    }
                    None => (),
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    match self.gen_panel.render(ui, self.progress) {
                        Some(GeneratorAction::SetSeed(new_seed)) => {
                            self.set_seed(new_seed);
                        }
                        Some(GeneratorAction::Regen(must_delete, from_idx)) => {
                            self.regen(must_delete, from_idx);
                        }
                        Some(GeneratorAction::Disable(idx)) => {
                            self.main2gen_tx
                                .send(WorldGenCommand::DisableStep(idx))
                                .unwrap();
                            self.regen(false, idx);
                        }
                        Some(GeneratorAction::Enable(idx)) => {
                            self.main2gen_tx
                                .send(WorldGenCommand::EnableStep(idx))
                                .unwrap();
                            self.regen(false, idx);
                        }
                        Some(GeneratorAction::DisplayLayer(step)) => {
                            self.main2gen_tx
                                .send(WorldGenCommand::GetStepMap(step))
                                .unwrap();
                        }
                        _ => (),
                    }
                });
            })
        });
    }
    fn render_central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(self.enabled, |ui| {
                ui.heading("Terrain preview");
                ui.horizontal(|ui| {
                    egui::CollapsingHeader::new("2d preview")
                        .default_open(true)
                        .show(ui, |ui| match self.panel_2d.render(ui) {
                            Some(Panel2dAction::ResizePreview(new_size)) => self.resize(new_size),
                            _ => (),
                        });
                    egui::CollapsingHeader::new("3d preview")
                        .default_open(true)
                        .show(ui, |ui| {
                            self.panel_3d.render(ui);
                        });
                });
            })
        });
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // TODO when next eframe is released, get window size
        //frame.info().window_info
        ctx.set_visuals(Visuals::dark());
        loop {
            match self.thread2main_rx.try_recv() {
                Ok(ThreadMessage::GeneratorStepProgress(progress)) => {
                    let progstep = 1.0 / self.gen_panel.enabled_steps() as f32;
                    self.progress = (self.progress / progstep).floor() * progstep;
                    self.progress += progress * progstep;
                }
                Ok(ThreadMessage::GeneratorDone(hmap)) => {
                    log("main<=Done");
                    self.panel_2d
                        .refresh(self.image_size, self.preview_size as u32, &hmap);
                    self.enabled = true;
                    self.gen_panel.selected_step = self.gen_panel.steps.len() - 1;
                    self.panel_3d.update_mesh(&hmap);
                    self.gen_panel.is_running = false;
                    self.progress = 1.0;
                }
                Ok(ThreadMessage::GeneratorStepDone(step, hmap)) => {
                    log(&format!("main<=GeneratorStepDone({})", step));
                    if let Some(ref hmap) = hmap {
                        self.panel_2d
                            .refresh(self.image_size, self.preview_size as u32, hmap);
                    }
                    self.gen_panel.selected_step = step;
                    self.progress = (step + 1) as f32 / self.gen_panel.enabled_steps() as f32
                }
                Ok(ThreadMessage::GeneratorStepMap(_idx, hmap)) => {
                    self.panel_2d
                        .refresh(self.image_size, self.preview_size as u32, &hmap);
                }
                Ok(ThreadMessage::ExporterStepProgress(progress)) => {
                    let progstep = 1.0 / self.gen_panel.enabled_steps() as f32;
                    self.exporter_progress = (self.exporter_progress / progstep).floor() * progstep;
                    self.exporter_progress += progress * progstep;
                    self.exporter_text = format!(
                        "{}% {}/{} {}",
                        (self.exporter_progress * 100.0) as usize,
                        self.exporter_cur_step + 1,
                        self.gen_panel.steps.len(),
                        self.gen_panel.steps[self.exporter_cur_step]
                    );
                }
                Ok(ThreadMessage::ExporterStepDone(step)) => {
                    log(&format!("main<=ExporterStepDone({})", step));
                    self.exporter_progress =
                        (step + 1) as f32 / self.gen_panel.enabled_steps() as f32;
                    self.exporter_cur_step = step + 1;
                    if step + 1 == self.gen_panel.steps.len() {
                        self.exporter_text = "Saving png...".to_owned();
                    } else {
                        self.exporter_text = format!(
                            "{}% {}/{} {}",
                            (self.exporter_progress * 100.0) as usize,
                            step + 1,
                            self.gen_panel.steps.len(),
                            self.gen_panel.steps[self.exporter_cur_step]
                        );
                    }
                }
                Ok(ThreadMessage::ExporterDone(_res)) => {
                    // TODO popup if _res == Err(_)
                    log("main<=ExporterDone");
                    self.enabled = true;
                    self.exporter_progress = 1.0;
                    self.export_panel.enabled = true;
                    self.exporter_cur_step = 0;
                    self.exporter_text = String::new();
                }
                Err(_) => {
                    break;
                }
            }
        }
        self.render_left_panel(ctx);
        self.render_central_panel(ctx);
    }
}

pub fn log(msg: &str) {
    thread_local! {
        pub static LOGTIME: Instant = Instant::now();
    }
    LOGTIME.with(|log_time| {
        println!(
            "{:03.3} {}",
            log_time.elapsed().as_millis() as f32 / 1000.0,
            msg
        );
    });
}
