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
mod worldgen;

use eframe::egui::{self, Visuals};
use exporter::export_heightmap;
use std::time::Instant;

use panel_2dview::{Panel2dAction, Panel2dView};
use panel_3dview::Panel3dView;
use panel_export::PanelExport;
use panel_generator::{GeneratorAction, PanelGenerator};
use panel_save::{PanelSaveLoad, SaveLoadAction};
use worldgen::{ExportMap, WorldGenerator};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MASK_SIZE: usize = 64;

/// messages sent to the main thread by either world generator or exporter threads
pub enum ThreadMessage {
    /// from world generator : all steps have been computed => update 2D/3D previews
    GeneratorDone(ExportMap),
    /// from world generator : update progress bar
    GeneratorStepProgress(f32),
    /// from world generator : one step has been computed => update 2D preview if live preview enabled
    GeneratorStepDone(usize, Option<ExportMap>),
    /// from world generator : return the heightmap for a specific step
    GeneratorStepMap(usize, ExportMap),
    /// from exporter : one step has been computed
    ExporterStepDone(usize),
    /// from exporter : export is finished
    ExporterDone(Result<(), String>),
    /// from exporter : update progress bar
    ExporterStepProgress(f32),
}

fn main() {
    let options = eframe::NativeOptions {
        maximized: true,
        multisampling: 8,
        depth_buffer: 24,
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };
    println!(
        "wgen v{} - {} cpus {} cores",
        VERSION,
        num_cpus::get(),
        num_cpus::get_physical()
    );
    eframe::run_native(
        "wgen",
        options,
        Box::new(|ctx| Box::new(MyApp::new(&ctx.gl))),
    );
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
    // ui widgets
    gen_panel: PanelGenerator,
    export_panel: PanelExport,
    panel_3d: Panel3dView,
    panel_2d: Panel2dView,
    load_save_panel: PanelSaveLoad,
    /// an error to display in a popup
    err_msg: Option<String>,
    /// are we editing a mask ?
    mask_step: Option<usize>,
    /// last time the mask was updated
    last_mask_updated: f64,
    wgen: WorldGenerator,
    gl: Option<std::sync::Arc<glow::Context>>,
}

impl MyApp {
    fn new(gl: &Option<std::sync::Arc<glow::Context>>) -> Self {
        let preview_size = 128;
        let image_size = 790; //368;
        let seed = 0xdeadbeef;
        let wgen = WorldGenerator::new(seed, (preview_size, preview_size));
        let panel_2d = Panel2dView::new(image_size, preview_size as u32, &wgen.get_export_map());
        Self {
            image_size,
            preview_size,
            seed,
            panel_2d,
            panel_3d: Panel3dView::new(image_size as f32),
            progress: 1.0,
            exporter_progress: 1.0,
            exporter_text: String::new(),
            exporter_cur_step: 0,
            mask_step: None,
            gen_panel: PanelGenerator::default(),
            export_panel: PanelExport::default(),
            load_save_panel: PanelSaveLoad::default(),
            err_msg: None,
            last_mask_updated: 0.0,
            wgen,
            gl: gl.clone(),
        }
    }
}

impl MyApp {
    fn export(&mut self) {
        let steps = self.gen_panel.steps.clone();
        let seed = self.seed;
        if let Err(msg) = export_heightmap(seed, &steps, &self.export_panel, &self.gl) {
            let err_msg = format!("Error while exporting heightmap : {}", msg);
            println!("{}", err_msg);
            self.err_msg = Some(err_msg);
        }
        self.exporter_progress = 1.0;
        self.export_panel.enabled = true;
        self.exporter_cur_step = 0;
        self.exporter_text = String::new();
    }
    fn regen(&mut self, must_delete: bool, from_idx: usize) {
        self.progress = from_idx as f32 / self.gen_panel.enabled_steps() as f32;
        let len = self.gen_panel.steps.len();
        if must_delete {
            self.wgen.remove_step(from_idx);
        }
        if len == 0 {
            return;
        }
        for i in from_idx.min(len - 1)..len {
            self.wgen
                .execute_step(i, &self.gen_panel.steps[i], &self.gl);
        }
        let hmap = self.wgen.get_export_map();
        self.panel_2d
            .refresh(self.image_size, self.preview_size as u32, Some(&hmap));
        self.gen_panel.selected_step = self.gen_panel.steps.len() - 1;
        self.panel_3d.update_mesh(&hmap);
    }
    fn set_seed(&mut self, new_seed: u64) {
        self.seed = new_seed;
        self.wgen.set_seed(new_seed);
        self.regen(false, 0);
    }
    fn resize(&mut self, new_size: usize) {
        if self.preview_size == new_size {
            return;
        }
        self.preview_size = new_size;
        self.wgen = WorldGenerator::new(self.seed, (new_size, new_size));
        self.regen(false, 0);
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
                    if let Err(msg) = self.gen_panel.load(self.load_save_panel.get_file_path()) {
                        let err_msg = format!(
                            "Error while reading project {} : {}",
                            self.load_save_panel.get_file_path(),
                            msg
                        );
                        println!("{}", err_msg);
                        self.err_msg = Some(err_msg);
                    } else {
                        self.wgen.clear();
                        self.set_seed(self.gen_panel.seed);
                    }
                }
                Some(SaveLoadAction::Save) => {
                    if let Err(msg) = self.gen_panel.save(self.load_save_panel.get_file_path()) {
                        let err_msg = format!(
                            "Error while writing project {} : {}",
                            self.load_save_panel.get_file_path(),
                            msg
                        );
                        println!("{}", err_msg);
                        self.err_msg = Some(err_msg);
                    }
                }
                None => (),
            }
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                match self.gen_panel.render(ui, self.progress) {
                    Some(GeneratorAction::Clear) => {
                        self.wgen.clear();
                    }
                    Some(GeneratorAction::SetSeed(new_seed)) => {
                        self.set_seed(new_seed);
                    }
                    Some(GeneratorAction::Regen(must_delete, from_idx)) => {
                        self.regen(must_delete, from_idx);
                    }
                    Some(GeneratorAction::Disable(idx)) => {
                        self.wgen.disable_step(idx);
                        self.regen(false, idx);
                    }
                    Some(GeneratorAction::Enable(idx)) => {
                        self.wgen.enable_step(idx);
                        self.regen(false, idx);
                    }
                    Some(GeneratorAction::DisplayLayer(step)) => {
                        let map = self.wgen.get_step_export_map(step);
                        // display heightmap from a specific step in the 2d preview
                        if let Some(step) = self.mask_step {
                            // mask was updated, recompute terrain
                            self.regen(false, step);
                            self.mask_step = None;
                        }
                        self.panel_2d.refresh(
                            self.image_size,
                            self.preview_size as u32,
                            Some(&map),
                        );
                    }
                    Some(GeneratorAction::DisplayMask(step)) => {
                        self.mask_step = Some(step);
                        let mask = if let Some(ref mask) = self.gen_panel.steps[step].mask {
                            Some(mask.clone())
                        } else {
                            Some(vec![1.0; MASK_SIZE * MASK_SIZE])
                        };
                        self.panel_2d
                            .display_mask(self.image_size, self.preview_size as u32, mask);
                    }
                    None => (),
                }
            });
        });
    }
    fn render_central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Terrain preview");
            ui.horizontal(|ui| {
                egui::CollapsingHeader::new("2d preview")
                    .default_open(true)
                    .show(ui, |ui| match self.panel_2d.render(ui) {
                        Some(Panel2dAction::ResizePreview(new_size)) => {
                            self.resize(new_size);
                            self.mask_step = None;
                            self.gen_panel.mask_selected = false;
                        }
                        Some(Panel2dAction::MaskUpdated) => {
                            self.last_mask_updated = ui.input().time;
                        }
                        Some(Panel2dAction::MaskDelete) => {
                            if let Some(step) = self.mask_step {
                                self.gen_panel.steps[step].mask = None;
                            }
                            self.last_mask_updated = 0.0;
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
    /*
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
        self.exporter_progress = (step + 1) as f32 / self.gen_panel.enabled_steps() as f32;
        self.exporter_cur_step = step + 1;
        if step + 1 == self.gen_panel.steps.len() {
            self.exporter_text =
                format!("Saving {}...", self.export_panel.file_type.to_string());
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
    Ok(ThreadMessage::ExporterDone(res)) => {
        if let Err(msg) = res {
            let err_msg = format!("Error while exporting heightmap : {}", msg);
            println!("{}", err_msg);
            self.err_msg = Some(err_msg);
        }
        log("main<=ExporterDone");
        self.exporter_progress = 1.0;
        self.export_panel.enabled = true;
        self.exporter_cur_step = 0;
        self.exporter_text = String::new();
    }
    */
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let wsize = frame.info().window_info.size;
        let new_size = ((wsize.x - 340.0) * 0.5) as usize;
        if new_size != self.image_size && new_size != 0 {
            // handle window resizing
            self.image_size = new_size;
            self.panel_2d
                .refresh(self.image_size, self.preview_size as u32, None);
            self.panel_3d = Panel3dView::new(self.image_size as f32);
            self.regen(false, 0);
        }
        ctx.set_visuals(Visuals::dark());
        self.render_left_panel(ctx);
        self.render_central_panel(ctx);
        if self.last_mask_updated > 0.0 && ctx.input().time - self.last_mask_updated >= 0.5 {
            if let Some(step) = self.mask_step {
                // mask was updated, copy mask to generator step
                if let Some(mask) = self.panel_2d.get_current_mask() {
                    self.gen_panel.steps[step].mask = Some(mask);
                }
            }
            self.last_mask_updated = 0.0;
        }

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
