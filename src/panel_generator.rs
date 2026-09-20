use egui::{emath::TSTransform, Color32, CursorIcon, Id, LayerId, Order, Sense, UiBuilder};

use crate::{
    generators::HillsConf,
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
    /// run the generators that have a GPU twin on the GPU (true) or on the CPU (false)
    SetBackend(bool),
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
    /// current drag n drop destination
    move_to_pos: usize,
    /// is the drag n drop zone currently hovered by the mouse cursor?
    hovered: bool,
    /// random number generator's seed
    pub seed: u64,
    /// the compute device's adapter name; `None` when the generators have no GPU
    pub gpu_name: Option<String>,
    /// the `Use GPU` checkbox
    pub use_gpu: bool,
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
            move_to_pos: 0,
            hovered: false,
            seed: 0xdeadbeef,
            gpu_name: None,
            use_gpu: false,
        }
    }
}

fn render_step_gui(ui: &mut egui::Ui, id: Id, body: impl FnOnce(&mut egui::Ui)) -> Option<f32> {
    let is_being_dragged = ui.ctx().is_being_dragged(id);
    if !is_being_dragged {
        ui.scope(body);
    } else {
        let layer_id = LayerId::new(Order::Tooltip, id);
        let response = ui
            .scope_builder(UiBuilder::new().layer_id(layer_id), body)
            .response;
        ui.ctx()
            .output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
        if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
            let mut delta = pointer_pos - response.rect.center();
            delta.x += 60.0;
            ui.ctx()
                .transform_layer_shapes(layer_id, TSTransform::from_translation(delta));
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
    }
    /// stores a painted mask on the step being edited; returns that step's index, to recompute from
    pub fn commit_mask(&mut self, mask: Vec<f32>) -> Option<usize> {
        self.set_mask(Some(mask))
    }
    /// removes the mask of the step being edited; returns that step's index, to recompute from
    pub fn delete_mask(&mut self) -> Option<usize> {
        self.set_mask(None)
    }
    fn set_mask(&mut self, mask: Option<Vec<f32>>) -> Option<usize> {
        let i = self.mask_step?;
        let step = self.steps.get_mut(i)?;
        step.mask = mask;
        Some(i)
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
                crate::spinner::spinner(ui);
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
        action.or(self.render_gpu_row(ui))
    }
    /// the compute device and the `Use GPU` checkbox; nothing when there is no GPU
    fn render_gpu_row(&mut self, ui: &mut egui::Ui) -> Option<GeneratorAction> {
        let label = format!("GPU: {}", self.gpu_name.as_ref()?);
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(label)
                .on_hover_text("Compute device used by the generators");
            if ui
                .checkbox(&mut self.use_gpu, "Use GPU")
                .on_hover_text(
                    "Run the generators that have a GPU version on the GPU (Fbm); off = CPU",
                )
                .changed()
            {
                self.exit_mask_mode();
                action = Some(GeneratorAction::SetBackend(self.use_gpu));
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
                    for typ in StepType::all() {
                        let (name, desc) = (typ.name(), typ.description());
                        ui.selectable_value(
                            &mut self.cur_step,
                            Step {
                                typ,
                                ..Default::default()
                            },
                            name,
                        )
                        .on_hover_text(desc);
                    }
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
        let dragging = ui.ctx().dragged_id().is_some() && self.hovered;
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
                                ui.ctx().output_mut(|o| o.cursor_icon = CursorIcon::Grab);
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
        step.typ.render(ui);
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
            self.selected_step = self.selected_step.min(self.steps.len().saturating_sub(1));
            self.mask_step = None;
            action = Some(GeneratorAction::Regen {
                delete: Some(i),
                from: i,
            });
        }
        if ui.ctx().input(|i| i.pointer.any_released()) {
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
        action
    }
}
