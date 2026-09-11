use eframe::egui::{self, CursorIcon, Id, LayerId, Order, Sense};
use epaint::Color32;

use crate::{
    generators::{
        render_fbm, render_hills, render_island, render_landmass, render_mid_point,
        render_mudslide, render_water_erosion, FbmConf, HillsConf, IslandConf, LandMassConf,
        MidPointConf, MudSlideConf, NormalizeConf, WaterErosionConf,
    },
    project::Project,
    worldgen::{Step, StepType},
    MASK_SIZE,
};

/// actions to do by the main program
pub enum GeneratorAction {
    /// recompute the heightmap from step `from`, after removing step `delete` from the generator
    Regen { delete: Option<usize>, from: usize },
    /// display a specific step heightmap in the 2D preview
    DisplayLayer(usize),
    /// edit this mask in the 2D preview (a full mask when the step has none yet)
    DisplayMask(Vec<f32>),
    /// change the RNG seed
    SetSeed(u64),
    /// remove all steps
    Clear,
}

pub struct PanelGenerator {
    /// is the world generator currently computing the heightmap?
    pub is_running: bool,
    /// generator steps with their configuration and masks
    pub steps: Vec<Step>,
    /// current selected step. used for combo box. must be outside of steps in case steps is empty
    cur_step: Step,
    /// current selected step index
    pub selected_step: usize,
    /// step whose mask is being painted in the 2D preview, if any
    mask_step: Option<usize>,
    /// step whose mask changed since the last recompute; recomputed once mask editing ends
    mask_dirty: Option<usize>,
    /// current drag n drop destination
    move_to_pos: usize,
    /// is the drag n drop zone currently hovered by the mouse cursor?
    hovered: bool,
    /// random number generator's seed
    pub seed: u64,
}

impl Default for PanelGenerator {
    fn default() -> Self {
        Self {
            is_running: false,
            steps: Vec::new(),
            cur_step: Step {
                typ: StepType::Hills(HillsConf::default()),
                ..Default::default()
            },
            selected_step: 0,
            mask_step: None,
            mask_dirty: None,
            move_to_pos: 0,
            hovered: false,
            seed: 0xdeadbeef,
        }
    }
}

fn render_step_gui(ui: &mut egui::Ui, id: Id, body: impl FnOnce(&mut egui::Ui)) -> Option<f32> {
    let is_being_dragged = ui.ctx().is_being_dragged(id);
    if !is_being_dragged {
        ui.scope(body);
    } else {
        let layer_id = LayerId::new(Order::Tooltip, id);
        let response = ui.with_layer_id(layer_id, body).response;
        ui.output_mut(|i| i.cursor_icon = CursorIcon::Grabbing);
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
    pub fn is_editing_mask(&self) -> bool {
        self.mask_step.is_some()
    }
    /// ends mask editing without recomputing anything : the caller recomputes the whole stack
    pub fn exit_mask_mode(&mut self) {
        self.mask_step = None;
        self.mask_dirty = None;
    }
    /// stores a painted mask on the step being edited
    pub fn commit_mask(&mut self, mask: Vec<f32>) {
        self.set_mask(Some(mask));
    }
    /// removes the mask of the step being edited
    pub fn delete_mask(&mut self) {
        self.set_mask(None);
    }
    fn set_mask(&mut self, mask: Option<Vec<f32>>) {
        let Some(i) = self.mask_step else { return };
        if let Some(step) = self.steps.get_mut(i) {
            step.mask = mask;
            self.mask_dirty = Some(i);
        }
    }
    pub fn load_project(&mut self, project: Project) {
        self.steps = project.steps;
        self.seed = project.seed;
        self.selected_step = 0;
        self.exit_mask_mode();
    }
    pub fn project(&self) -> Project {
        Project::new(self.seed, self.steps.clone())
    }
    fn render_header(&mut self, ui: &mut egui::Ui, progress: f32) -> Option<GeneratorAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading("Generators");
            if self.is_running {
                ui.spinner();
            }
        });
        ui.add(egui::ProgressBar::new(progress).show_percentage());
        ui.horizontal(|ui| {
            if ui.button("Clear").clicked() {
                self.steps.clear();
                self.selected_step = 0;
                self.exit_mask_mode();
                action = Some(GeneratorAction::Clear)
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
                self.mask_step = None;
                action = Some(GeneratorAction::Regen {
                    delete: None,
                    from: self.selected_step,
                })
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
        // let dragging = ui.ctx().dragged_id.is_some()
        let dragging = ui.memory(|m| m.is_anything_being_dragged()) && self.hovered;
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
                                ui.output_mut(|o| o.cursor_icon = CursorIcon::Grab);
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
                                self.mask_step = None;
                                action = Some(GeneratorAction::Regen {
                                    delete: None,
                                    from: i,
                                });
                            }
                            if ui
                                .selectable_label(
                                    self.mask_step == Some(i),
                                    if step.mask.is_none() { "⬜" } else { "⬛" },
                                )
                                .on_hover_text("Edit this step's mask")
                                .clicked()
                            {
                                self.mask_step = Some(i);
                                self.selected_step = i;
                            }
                            if ui
                                .selectable_label(
                                    self.selected_step == i && self.mask_step.is_none(),
                                    step.to_string(),
                                )
                                .clicked()
                            {
                                self.selected_step = i;
                                self.mask_step = None;
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
        let step = self.steps.get_mut(self.selected_step)?;
        match step {
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
            action = Some(GeneratorAction::Regen {
                delete: None,
                from: self.selected_step,
            });
            self.mask_step = None;
        }
        action
    }
    pub fn render(&mut self, ui: &mut egui::Ui, progress: f32) -> Option<GeneratorAction> {
        let previous_selected_step = self.selected_step;
        let previous_mask_step = self.mask_step;
        let mut action = self.render_header(ui, progress);
        action = action.or(self.render_new_step(ui));
        ui.end_row();
        let mut to_remove = None;
        let mut to_move = None;
        action = action.or(self.render_step_list(ui, &mut to_remove, &mut to_move));
        ui.separator();
        action = action.or(self.render_curstep_conf(ui));
        if action.is_none()
            && (previous_selected_step != self.selected_step
                || previous_mask_step != self.mask_step)
        {
            action = Some(match self.mask_step.and_then(|i| self.steps.get(i)) {
                Some(step) => GeneratorAction::DisplayMask(
                    step.mask
                        .clone()
                        .unwrap_or_else(|| vec![1.0; MASK_SIZE * MASK_SIZE]),
                ),
                None => GeneratorAction::DisplayLayer(self.selected_step),
            });
        }
        if let Some(i) = to_remove {
            self.steps.remove(i);
            self.selected_step = self
                .selected_step
                .min(self.steps.len().saturating_sub(1));
            self.mask_step = None;
            action = Some(GeneratorAction::Regen {
                delete: Some(i),
                from: i,
            });
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(i) = to_move {
                if i != self.move_to_pos {
                    let step = self.steps.remove(i);
                    let dest = if self.move_to_pos > i {
                        self.move_to_pos - 1
                    } else {
                        self.move_to_pos
                    };
                    self.steps.insert(dest, step);
                    self.mask_step = None;
                    // every step between the old and the new position changed
                    action = Some(GeneratorAction::Regen {
                        delete: None,
                        from: i.min(dest),
                    });
                }
            }
        }
        self.merge_dirty_mask(action)
    }
    /// once mask editing ends, a mask painted earlier is recomputed with whatever else is pending
    fn merge_dirty_mask(&mut self, action: Option<GeneratorAction>) -> Option<GeneratorAction> {
        if self.mask_step.is_some() {
            return action;
        }
        let Some(dirty) = self.mask_dirty else {
            return action;
        };
        match action {
            Some(GeneratorAction::Regen { delete, from }) => {
                self.mask_dirty = None;
                Some(GeneratorAction::Regen {
                    delete,
                    from: from.min(dirty),
                })
            }
            None | Some(GeneratorAction::DisplayLayer(_)) => {
                self.mask_dirty = None;
                Some(GeneratorAction::Regen {
                    delete: None,
                    from: dirty,
                })
            }
            // SetSeed recomputes everything, Clear drops everything, DisplayMask cannot happen here
            other => {
                self.mask_dirty = None;
                other
            }
        }
    }
}
