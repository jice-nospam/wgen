use eframe::egui::{self, CursorIcon, Id, LayerId, Order, Sense};
use epaint::Color32;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
};

use crate::{
    generators::{
        render_fbm, render_hills, render_island, render_landmass, render_mid_point,
        render_mudslide, render_water_erosion, FbmConf, HillsConf, IslandConf, LandMassConf,
        MidPointConf, MudSlideConf, NormalizeConf, WaterErosionConf,
    },
    worldgen::{Step, StepType},
    VERSION,
};

pub enum GeneratorAction {
    /// deleteStep, stepIndex
    Regen(bool, usize),
    Disable(usize),
    Enable(usize),
    DisplayLayer(usize),
    DisplayMask(usize),
    SetSeed(u64),
}

#[derive(Serialize, Deserialize)]
pub struct PanelGenerator {
    version: String,
    #[serde(skip)]
    pub is_running: bool,
    #[serde(skip)]
    pub mask_selected: bool,
    pub steps: Vec<Step>,
    cur_step: Step,
    pub selected_step: usize,
    move_to_pos: usize,
    hovered: bool,
    pub seed: u64,
}

impl Default for PanelGenerator {
    fn default() -> Self {
        Self {
            version: VERSION.to_owned(),
            is_running: false,
            mask_selected: false,
            steps: Vec::new(),
            cur_step: Step {
                typ: StepType::Hills(HillsConf::default()),
                ..Default::default()
            },
            selected_step: 0,
            move_to_pos: 0,
            hovered: false,
            seed: 0xdeadbeef,
        }
    }
}

fn render_step_gui(ui: &mut egui::Ui, id: Id, body: impl FnOnce(&mut egui::Ui)) -> Option<f32> {
    let is_being_dragged = ui.memory().is_being_dragged(id);
    if !is_being_dragged {
        ui.scope(body);
    } else {
        let layer_id = LayerId::new(Order::Tooltip, id);
        let response = ui.with_layer_id(layer_id, body).response;
        ui.output().cursor_icon = CursorIcon::Grabbing;
        if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
            let mut delta = pointer_pos - response.rect.center();
            delta.x += 60.0;
            ui.ctx().translate_layer(layer_id, delta);
            return Some(delta.y);
        }
    }
    None
}

impl PanelGenerator {
    pub fn enabled_steps(&self) -> usize {
        self.steps.iter().filter(|s| !s.disabled).count()
    }
    fn render_header(&mut self, ui: &mut egui::Ui, progress: f32) -> Option<GeneratorAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading("Generators");
            if self.is_running {
                ui.add(egui::Spinner::new());
            }
        });
        ui.add(egui::ProgressBar::new(progress).show_percentage());
        ui.horizontal(|ui| {
            if ui.button("Clear").clicked() {
                self.steps.clear();
                action = Some(GeneratorAction::Regen(false, 0))
            }
            ui.label("Seed");
            let old_seed = self.seed;
            let old_size = ui.spacing().interact_size.x;
            ui.spacing_mut().interact_size.x = 100.0;
            ui.add(egui::DragValue::new(&mut self.seed).speed(1.0));
            ui.spacing_mut().interact_size.x = old_size;
            if self.seed != old_seed {
                action = Some(GeneratorAction::SetSeed(self.seed));
            }
        });
        action
    }
    /// render UI to add a new step
    fn render_new_step(&mut self, ui: &mut egui::Ui) -> Option<GeneratorAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            if ui.button("New step").clicked() {
                self.steps.push(self.cur_step.clone());
                self.selected_step = self.steps.len() - 1;
                action = Some(GeneratorAction::Regen(false, self.selected_step))
            }
            egui::ComboBox::from_label("")
                .selected_text(format!("{}", self.cur_step))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.cur_step,
                        Step {
                            typ: StepType::Hills(HillsConf::default()),
                            ..Default::default()
                        },
                        "Hills",
                    )
                    .on_hover_text("Add round hills to generate a smooth land");
                    ui.selectable_value(
                        &mut self.cur_step,
                        Step {
                            typ: StepType::Fbm(FbmConf::default()),
                            ..Default::default()
                        },
                        "Fbm",
                    )
                    .on_hover_text("Add fractional brownian motion to generate a mountainous land");
                    ui.selectable_value(
                        &mut self.cur_step,
                        Step {
                            typ: StepType::MidPoint(MidPointConf::default()),
                            ..Default::default()
                        },
                        "MidPoint",
                    )
                    .on_hover_text("Use mid point deplacement to generate a mountainous land");
                    ui.selectable_value(
                        &mut self.cur_step,
                        Step {
                            typ: StepType::Normalize(NormalizeConf::default()),
                            ..Default::default()
                        },
                        "Normalize",
                    )
                    .on_hover_text("Scale the terrain back to the 0.0-1.0 range");
                    ui.selectable_value(
                        &mut self.cur_step,
                        Step {
                            typ: StepType::LandMass(LandMassConf::default()),
                            ..Default::default()
                        },
                        "LandMass",
                    )
                    .on_hover_text(
                        "Scale the terrain so that only a proportion of land is above water level",
                    );
                    ui.selectable_value(
                        &mut self.cur_step,
                        Step {
                            typ: StepType::MudSlide(MudSlideConf::default()),
                            ..Default::default()
                        },
                        "MudSlide",
                    )
                    .on_hover_text("Simulate mud sliding and smoothing the terrain");
                    ui.selectable_value(
                        &mut self.cur_step,
                        Step {
                            typ: StepType::WaterErosion(WaterErosionConf::default()),
                            ..Default::default()
                        },
                        "WaterErosion",
                    )
                    .on_hover_text("Simulate rain falling and carving rivers");
                    ui.selectable_value(
                        &mut self.cur_step,
                        Step {
                            typ: StepType::Island(IslandConf::default()),
                            ..Default::default()
                        },
                        "Island",
                    )
                    .on_hover_text("Lower height on the map borders");
                });
        });
        action
    }
    /// render the list of steps of current project
    fn render_step_list(
        &mut self,
        ui: &mut egui::Ui,
        to_remove: &mut Option<usize>,
        to_move: &mut Option<usize>,
    ) -> Option<GeneratorAction> {
        let mut action = None;
        let len = self.steps.len();
        let dragging = ui.memory().is_anything_being_dragged() && self.hovered;
        let response = ui
            .scope(|ui| {
                for (i, step) in self.steps.iter_mut().enumerate() {
                    if dragging && self.move_to_pos == i {
                        ui.separator();
                    }
                    let item_id = Id::new("wgen").with(i);
                    if let Some(dy) = render_step_gui(ui, item_id, |ui| {
                        ui.horizontal(|ui| {
                            let response = ui
                                .button("▓")
                                .on_hover_text("Drag this to change step order");
                            let response = ui.interact(response.rect, item_id, Sense::drag());
                            if response.hovered() {
                                ui.output().cursor_icon = CursorIcon::Grab;
                            }
                            if ui.button("⊗").on_hover_text("Delete this step").clicked() {
                                *to_remove = Some(i);
                            }
                            if ui
                                .button(egui::RichText::new("👁").color(if step.disabled {
                                    Color32::from_rgb(0, 0, 0)
                                } else {
                                    Color32::from_rgb(200, 200, 200)
                                }))
                                .on_hover_text(if step.disabled {
                                    "Enable this step"
                                } else {
                                    "Disable this step"
                                })
                                .clicked()
                            {
                                step.disabled = !step.disabled;
                                if step.disabled {
                                    action = Some(GeneratorAction::Disable(i));
                                } else {
                                    action = Some(GeneratorAction::Enable(i));
                                }
                            }
                            if ui
                                .button(if step.mask.is_none() { "⬜" } else { "⬛" })
                                .on_hover_text("Add a mask to this step")
                                .clicked()
                            {
                                self.mask_selected = true;
                                self.selected_step = i;
                            }
                            if ui
                                .selectable_label(
                                    self.selected_step == i && !self.mask_selected,
                                    step.to_string(),
                                )
                                .clicked()
                            {
                                self.selected_step = i;
                                self.mask_selected = false;
                            }
                        });
                    }) {
                        *to_move = Some(i);
                        let dest = i as i32 + (dy / 20.0) as i32;
                        self.move_to_pos = dest.clamp(0, len as i32) as usize;
                    }
                }
            })
            .response;
        self.hovered = response.hovered();
        action
    }
    /// render the configuration UI for currently selected step
    fn render_curstep_conf(&mut self, ui: &mut egui::Ui) -> Option<GeneratorAction> {
        let mut action = None;
        match &mut self.steps[self.selected_step] {
            Step {
                typ: StepType::Hills(conf),
                ..
            } => render_hills(ui, conf),
            Step {
                typ: StepType::LandMass(conf),
                ..
            } => render_landmass(ui, conf),
            Step {
                typ: StepType::MudSlide(conf),
                ..
            } => render_mudslide(ui, conf),
            Step {
                typ: StepType::Fbm(conf),
                ..
            } => render_fbm(ui, conf),
            Step {
                typ: StepType::WaterErosion(conf),
                ..
            } => render_water_erosion(ui, conf),
            Step {
                typ: StepType::Island(conf),
                ..
            } => render_island(ui, conf),
            Step {
                typ: StepType::MidPoint(conf),
                ..
            } => render_mid_point(ui, conf),
            Step {
                typ: StepType::Normalize(_),
                ..
            } => (),
        }
        if ui.button("Refresh").clicked() {
            action = Some(GeneratorAction::Regen(false, self.selected_step))
        }
        action
    }
    pub fn render(&mut self, ui: &mut egui::Ui, progress: f32) -> Option<GeneratorAction> {
        let previous_selected_step = self.selected_step;
        let previous_mask_selected = self.mask_selected;
        let mut action = self.render_header(ui, progress);
        action = action.or(self.render_new_step(ui));
        ui.end_row();
        let mut to_remove = None;
        let mut to_move = None;
        action = action.or(self.render_step_list(ui, &mut to_remove, &mut to_move));
        ui.separator();
        if !self.steps.is_empty() {
            action = action.or(self.render_curstep_conf(ui));
        }
        if action.is_none()
            && (previous_selected_step != self.selected_step
                || previous_mask_selected != self.mask_selected)
        {
            if self.mask_selected {
                action = Some(GeneratorAction::DisplayMask(self.selected_step));
            } else {
                action = Some(GeneratorAction::DisplayLayer(self.selected_step));
            }
        }
        if let Some(i) = to_remove {
            self.steps.remove(i);
            if self.selected_step >= self.steps.len() {
                self.selected_step = if self.steps.is_empty() {
                    0
                } else {
                    self.steps.len() - 1
                };
            }
            action = Some(GeneratorAction::Regen(true, i));
        }
        if ui.input().pointer.any_released() {
            if let Some(i) = to_move {
                if i != self.move_to_pos {
                    let step = self.steps.remove(i);
                    let dest = if self.move_to_pos > i {
                        self.move_to_pos - 1
                    } else {
                        self.move_to_pos
                    };
                    self.steps.insert(dest, step);
                    action = Some(GeneratorAction::Regen(false, i));
                }
            }
        }
        action
    }
    pub fn load(&mut self, file_path: &str) -> Result<(), String> {
        let mut file = File::open(file_path).map_err(|_| "Unable to open the file")?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|_| "Unable to read the file")?;
        let gen_data: PanelGenerator =
            ron::from_str(&contents).map_err(|e| format!("Cannot parse the file : {}", e))?;
        if gen_data.version != VERSION {
            return Err(format!(
                "Bad file version. Expected {}, found {}",
                VERSION, gen_data.version
            ));
        }
        *self = gen_data;
        Ok(())
    }
    pub fn save(&self, file_path: &str) -> Result<(), String> {
        let data = ron::to_string(self).unwrap();
        let mut buffer = File::create(file_path).map_err(|_| "Unable to create the file")?;
        write!(buffer, "{}", data).map_err(|_| "Unable to write to the file")?;
        Ok(())
    }
}
