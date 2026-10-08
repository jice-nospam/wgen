use egui::{
    emath, Color32, ColorImage, PointerButton, Pos2, Rect, Stroke, TextureHandle, TextureId,
    TextureOptions,
};

use crate::{
    mask::{feather_mask, mask_side},
    panel_2dview::Panel2dAction,
    MASK_SIZE,
};

/// maximum size of the brush relative to the canvas
const MAX_BRUSH_SIZE: f32 = 0.25;
/// mask change per second at opacity 1.0, as a share of the distance to the target
const BRUSH_SPEED: f32 = 10.0;
/// smallest brush radius in mask cells: always covers the centre of the cell under the cursor
const MIN_BRUSH_RADIUS: f32 = 0.75;
/// number of strokes, feather changes or clears that can be undone
const UNDO_DEPTH: usize = 20;

#[derive(Clone, Copy)]
pub struct BrushConfig {
    /// mask value the left button paints; the right button paints `1.0 - value`
    pub value: f32,
    /// brush radius, from one mask cell (0.0) to a quarter of the map (1.0)
    pub size: f32,
    /// share of the radius over which the brush fades out: 0.0 hard edge, 1.0 fades from the centre
    pub falloff: f32,
    /// how fast a stroke reaches its value, 0.05..=1.0
    pub opacity: f32,
}
pub struct PanelMaskEdit {
    /// preview canvas size in pixels
    image_size: usize,
    /// the mask as a square f32 matrix of any side
    mask: Option<Vec<f32>>,
    /// the step's edge feather, 0.0..=1.0, committed with the mask
    feather: f32,
    /// the feather slider moved and the new value is not handed over yet
    feather_pending: bool,
    /// the brush parameters
    conf: BrushConfig,
    /// GPU texture of `mask`, a grayscale image of the mask's side
    mask_tex: Option<TextureHandle>,
    /// `mask` changed since the last upload to `mask_tex`
    mask_dirty: bool,
    /// a stroke is under way: the left or right button went down on the canvas and is still held
    is_painting: bool,
    /// previous `(mask, feather)` pairs, the most recent last, at most `UNDO_DEPTH`
    undo: Vec<(Vec<f32>, f32)>,
    /// used to compute the brush impact on the mask depending on elapsed time
    prev_frame_time: f64,
    /// how transparent we want the heightmap to appear on top of the mask
    pub heightmap_transparency: f32,
}

impl PanelMaskEdit {
    pub fn new(image_size: usize) -> Self {
        PanelMaskEdit {
            image_size,
            mask: None,
            feather: 0.0,
            feather_pending: false,
            conf: BrushConfig {
                value: 0.5,
                size: 0.5,
                falloff: 0.5,
                opacity: 0.5,
            },
            mask_tex: None,
            mask_dirty: false,
            is_painting: false,
            undo: Vec::new(),
            prev_frame_time: -1.0,
            heightmap_transparency: 0.5,
        }
    }
    pub fn display_mask(&mut self, image_size: usize, mask: Vec<f32>, feather: f32) {
        self.image_size = image_size;
        self.mask_dirty = true;
        self.is_painting = false;
        self.mask = Some(mask);
        self.feather = feather;
        self.feather_pending = false;
        self.undo.clear();
    }
    /// the canvas size changed (the heightmap texture itself belongs to the 2D panel)
    pub fn heightmap_changed(&mut self, image_size: usize) {
        self.image_size = image_size;
    }
    /// `heightmap_id` is the 2D panel's heightmap texture, drawn over the mask
    pub fn render(&mut self, ui: &mut egui::Ui, heightmap_id: TextureId) -> Option<Panel2dAction> {
        let mut action = None;
        ui.vertical(|ui| {
            let was_painting = self.is_painting;
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                self.paint_canvas(ui, heightmap_id);
            });
            if self.is_painting {
                ui.ctx().request_repaint();
            } else {
                self.prev_frame_time = -1.0;
                if was_painting {
                    // the brush stroke ended : hand the mask over to its step
                    action = self.commit();
                } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                    action = self.undo();
                }
            }
            ui.label("left: paint value · right: paint 1 − value");
            ui.label("middle click: pick value · Ctrl+Z: undo");
            self.render_brush_row(ui);
            ui.horizontal(|ui| {
                let old_feather = self.feather;
                let response = ui
                    .add(egui::Slider::new(&mut self.feather, 0.0..=1.0).text("feather"))
                    .on_hover_text(
                        "Softens the mask edges: white areas darken near black ones, \
                         over up to a quarter of the map at 1.0",
                    );
                if response.changed() {
                    if !self.feather_pending {
                        let new_feather = self.feather;
                        self.feather = old_feather;
                        self.push_undo();
                        self.feather = new_feather;
                    }
                    self.feather_pending = true;
                    // the texture is re-uploaded by the next frame's canvas
                    self.mask_dirty = true;
                    ui.ctx().request_repaint();
                }
                // one recompute per gesture : commit once the slider is released
                if self.feather_pending && !response.dragged() {
                    action = self.commit();
                }
            });
            ui.horizontal(|ui| {
                ui.label("heightmap opacity");
                ui.add(
                    egui::DragValue::new(&mut self.heightmap_transparency)
                        .speed(0.01)
                        .range(0.0..=1.0),
                )
                .on_hover_text("How visible the heightmap is over the mask");
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                    .on_hover_text("Undo the last stroke, feather change or clear (Ctrl+Z)")
                    .clicked()
                {
                    action = self.undo();
                }
                if ui
                    .button("Clear mask")
                    .on_hover_text("Fill the whole mask with the brush value (1.0 removes the mask)")
                    .clicked()
                {
                    action = Some(self.clear());
                }
            });
        });
        action
    }
    /// fills the mask with the brush value and resets the feather, undoably; a mask filled
    /// with ones is deleted from its step
    fn clear(&mut self) -> Panel2dAction {
        self.push_undo();
        self.feather = 0.0;
        self.feather_pending = false;
        let value = self.conf.value;
        if let Some(ref mut mask) = self.mask {
            mask.fill(value);
            self.mask_dirty = true;
        }
        if value >= 1.0 {
            return Panel2dAction::MaskDelete;
        }
        self.commit().unwrap_or(Panel2dAction::MaskDelete)
    }
    /// brush size, falloff, value and opacity fields
    fn render_brush_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("brush size");
            ui.add(
                egui::DragValue::new(&mut self.conf.size)
                    .speed(0.01)
                    .range(0.0..=1.0),
            )
            .on_hover_text("Brush radius, up to a quarter of the map at 1.0");
            ui.label("falloff");
            ui.add(
                egui::DragValue::new(&mut self.conf.falloff)
                    .speed(0.01)
                    .range(0.0..=1.0),
            )
            .on_hover_text(
                "Share of the radius over which the brush fades out: 0 hard edge, 1 fades from the centre",
            );
            ui.label("value");
            ui.add(
                egui::DragValue::new(&mut self.conf.value)
                    .speed(0.01)
                    .range(0.0..=1.0),
            )
            .on_hover_text(
                "Mask value the left button paints; the right button paints 1 − value.                  1 keeps the step, 0 removes it. Middle-click the mask to pick a value",
            );
            ui.label("opacity");
            ui.add(
                egui::DragValue::new(&mut self.conf.opacity)
                    .speed(0.01)
                    .range(0.05..=1.0),
            )
            .on_hover_text("How fast a stroke reaches its value");
        });
    }
    /// saves the current mask and feather on the undo stack
    fn push_undo(&mut self) {
        if let Some(mask) = &self.mask {
            self.undo.push((mask.clone(), self.feather));
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
        }
    }
    /// restores the last saved mask and feather and hands them over to the step
    fn undo(&mut self) -> Option<Panel2dAction> {
        let (mask, feather) = self.undo.pop()?;
        self.mask = Some(mask);
        self.feather = feather;
        self.mask_dirty = true;
        self.commit()
    }
    /// hands the mask and its feather over to the step being edited
    fn commit(&mut self) -> Option<Panel2dAction> {
        self.feather_pending = false;
        self.mask.clone().map(|mask| Panel2dAction::MaskCommitted {
            mask,
            feather: self.feather,
        })
    }
    /// the mask as the editor shows it: raw while a stroke is painted, feathered between strokes
    fn displayed_mask(&self) -> Option<Vec<f32>> {
        let mask = self.mask.as_ref()?;
        Some(if self.is_painting {
            mask.clone()
        } else {
            feather_mask(mask, self.feather)
        })
    }
    /// allocates the canvas, applies the brush under the pointer, then paints mask, heightmap and brush
    fn paint_canvas(&mut self, ui: &mut egui::Ui, heightmap_id: TextureId) {
        let (rect, response) = ui.allocate_exact_size(
            egui::Vec2::splat(self.image_size as f32),
            egui::Sense::drag(),
        );
        let (lbutton, rbutton, mpressed, mouse_pos) = ui.ctx().input(|i| {
            (
                i.pointer.button_down(PointerButton::Primary),
                i.pointer.button_down(PointerButton::Secondary),
                i.pointer.button_pressed(PointerButton::Middle),
                i.pointer.hover_pos(),
            )
        });
        let to_screen = emath::RectTransform::from_to(
            Rect::from_min_size(Pos2::ZERO, response.rect.square_proportions()),
            response.rect,
        );
        let from_screen = to_screen.inverse();
        let brush_config = self.conf;
        // pointer position in canvas from 0.0,0.0 (top left) to 1.0,1.0 (bottom right)
        let canvas_pos = mouse_pos.map(|pos| from_screen * pos);
        let was_painting = self.is_painting;
        self.is_painting = response.is_pointer_button_down_on() && (lbutton || rbutton);
        if self.is_painting {
            if !was_painting {
                self.push_undo();
            }
            let dt = self.frame_dt(ui.ctx());
            if let Some(canvas_pos) = canvas_pos {
                let target = if lbutton {
                    brush_config.value
                } else {
                    1.0 - brush_config.value
                };
                self.update_mask(canvas_pos, target, brush_config, dt);
                self.mask_dirty = true;
            }
        } else if let Some(canvas_pos) = canvas_pos.filter(|&p| mpressed && in_canvas(p)) {
            self.pick_value(canvas_pos);
        }
        if self.is_painting != was_painting {
            // the texture switches between the raw and the feathered mask
            self.mask_dirty = true;
        }
        self.upload_mask(ui.ctx());
        let painter = ui.painter_at(rect);
        let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
        if let Some(mask_tex) = &self.mask_tex {
            painter.image(mask_tex.id(), rect, uv, Color32::WHITE);
        }
        let alpha = (self.heightmap_transparency * 255.0) as u8;
        painter.image(heightmap_id, rect, uv, Color32::from_white_alpha(alpha));
        if let Some(pos) = mouse_pos.filter(|_| canvas_pos.is_some_and(in_canvas)) {
            let side = self.mask.as_deref().map_or(MASK_SIZE, mask_side);
            let r_px = (brush_config.size * MAX_BRUSH_SIZE * rect.width())
                .max(MIN_BRUSH_RADIUS * rect.width() / side as f32);
            painter.circle_stroke(pos, r_px, Stroke::new(1.5_f32, Color32::RED));
            painter.circle_stroke(
                pos,
                r_px * (1.0 - brush_config.falloff),
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 0, 0, 110)),
            );
        }
    }
    /// seconds since the previous painting frame; the first frame of a stroke counts as 1/60 s
    fn frame_dt(&mut self, ctx: &egui::Context) -> f32 {
        let t = ctx.input(|i| i.time);
        let dt = if self.prev_frame_time == -1.0 {
            1.0 / 60.0
        } else {
            (t - self.prev_frame_time) as f32
        };
        self.prev_frame_time = t;
        dt
    }
    /// sets the brush value to the raw mask value of the cell under `canvas_pos`
    fn pick_value(&mut self, canvas_pos: Pos2) {
        if let Some(mask) = &self.mask {
            if let Some(cell) = mask_cell(mask_side(mask), canvas_pos) {
                self.conf.value = mask[cell];
            }
        }
    }
    /// uploads the displayed mask to `mask_tex` when it changed since the last frame
    fn upload_mask(&mut self, ctx: &egui::Context) {
        if !self.mask_dirty {
            return;
        }
        if let Some(mask) = self.displayed_mask() {
            let img = mask_image(&mask);
            match &mut self.mask_tex {
                Some(handle) => handle.set(img, TextureOptions::LINEAR),
                None => self.mask_tex = Some(ctx.load_texture("mask", img, TextureOptions::LINEAR)),
            }
        }
        self.mask_dirty = false;
    }

    /// moves the mask cells under the brush toward `target`, never past it
    fn update_mask(&mut self, canvas_pos: Pos2, target: f32, brush_config: BrushConfig, dt: f32) {
        if let Some(ref mut mask) = self.mask {
            let side = mask_side(mask);
            let mx = canvas_pos.x * side as f32;
            let my = canvas_pos.y * side as f32;
            let brush_radius =
                (brush_config.size * side as f32 * MAX_BRUSH_SIZE).max(MIN_BRUSH_RADIUS);
            let falloff_dist = (1.0 - brush_config.falloff) * brush_radius;
            let minx = (mx - brush_radius).floor().max(0.0) as usize;
            let maxx = (((mx + brush_radius).ceil()).max(0.0) as usize).min(side);
            let miny = (my - brush_radius).floor().max(0.0) as usize;
            let maxy = (((my + brush_radius).ceil()).max(0.0) as usize).min(side);
            let brush_coef = 1.0 / (brush_radius - falloff_dist);
            let coef = (dt * BRUSH_SPEED * brush_config.opacity).min(1.0);
            for y in miny..maxy {
                let dy = y as f32 + 0.5 - my;
                let yoff = y * side;
                for x in minx..maxx {
                    let dx = x as f32 + 0.5 - mx;
                    // distance from the brush centre to the cell centre
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist >= brush_radius {
                        continue;
                    }
                    let alpha = if dist < falloff_dist {
                        1.0
                    } else {
                        (1.0 - (dist - falloff_dist) * brush_coef).clamp(0.0, 1.0)
                    };
                    let current_value = mask[x + yoff];
                    mask[x + yoff] = current_value + coef * alpha * (target - current_value);
                }
            }
        }
    }
}

/// index of the mask cell containing `canvas_pos`, `None` outside `[0,1)²`
fn mask_cell(side: usize, canvas_pos: Pos2) -> Option<usize> {
    let inside = |v: f32| (0.0..1.0).contains(&v);
    if !inside(canvas_pos.x) || !inside(canvas_pos.y) {
        return None;
    }
    let x = ((canvas_pos.x * side as f32) as usize).min(side - 1);
    let y = ((canvas_pos.y * side as f32) as usize).min(side - 1);
    Some(x + y * side)
}

fn in_canvas(canvas_pos: Pos2) -> bool {
    canvas_pos.x >= 0.0 && canvas_pos.x <= 1.0 && canvas_pos.y >= 0.0 && canvas_pos.y <= 1.0
}

/// the mask as a top-down grayscale image, row `y` of the mask on row `y` of the image
fn mask_image(mask: &[f32]) -> ColorImage {
    let bytes: Vec<u8> = mask
        .iter()
        .map(|v| (v * 255.0).clamp(0.0, 255.0) as u8)
        .collect();
    let side = mask_side(mask);
    ColorImage::from_gray([side, side], &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_image_keeps_row_order() {
        let mut mask = vec![0.0; MASK_SIZE * MASK_SIZE];
        mask[3 + 5 * MASK_SIZE] = 1.0;
        let img = mask_image(&mask);
        assert_eq!(img.size, [MASK_SIZE, MASK_SIZE]);
        assert_eq!(img.pixels[3 + 5 * MASK_SIZE], Color32::from_gray(255));
        assert_eq!(img.pixels[5 + 3 * MASK_SIZE], Color32::from_gray(0));
    }

    fn panel_with(mask: Vec<f32>) -> PanelMaskEdit {
        let mut panel = PanelMaskEdit::new(256);
        panel.display_mask(256, mask, 0.0);
        panel
    }

    fn brush(size: f32) -> BrushConfig {
        BrushConfig {
            value: 1.0,
            size,
            falloff: 0.5,
            opacity: 0.5,
        }
    }

    #[test]
    fn strokes_move_towards_the_target_only() {
        let side = MASK_SIZE;
        let mut panel = panel_with(vec![0.5; side * side]);
        let conf = brush(0.5);
        let radius = conf.size * side as f32 * MAX_BRUSH_SIZE;
        let falloff_dist = (1.0 - conf.falloff) * radius;
        for target in [1.0, 0.0] {
            panel.mask.as_mut().unwrap().fill(0.5);
            panel.update_mask(Pos2::new(0.5, 0.5), target, conf, 10.0);
            let mask = panel.mask.as_ref().unwrap();
            for y in 0..side {
                for x in 0..side {
                    let (dx, dy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
                    let dist = (dx * dx + dy * dy).sqrt();
                    let v = mask[x + y * side];
                    assert!((0.0..=1.0).contains(&v), "{v} at ({x},{y})");
                    if dist < falloff_dist {
                        assert_eq!(v, target, "inside at ({x},{y})");
                    } else if dist >= radius {
                        assert_eq!(v, 0.5, "outside at ({x},{y})");
                    }
                }
            }
        }
    }

    #[test]
    fn pick_value_reads_the_cell_under_the_cursor() {
        let mut mask = vec![0.2; MASK_SIZE * MASK_SIZE];
        mask[3 + 5 * MASK_SIZE] = 0.7;
        let mut panel = panel_with(mask.clone());
        panel.pick_value(Pos2::new(3.5 / 64.0, 5.5 / 64.0));
        assert_eq!(panel.conf.value, 0.7);
        assert_eq!(panel.mask.as_ref().unwrap(), &mask);
    }

    #[test]
    fn smallest_brush_paints_one_cell() {
        let mut panel = panel_with(vec![1.0; MASK_SIZE * MASK_SIZE]);
        panel.update_mask(Pos2::new(10.5 / 64.0, 10.5 / 64.0), 0.0, brush(0.0), 0.1);
        let mask = panel.mask.as_ref().unwrap();
        for (i, &v) in mask.iter().enumerate() {
            if i == 10 + 10 * MASK_SIZE {
                assert!(v < 1.0);
            } else {
                assert_eq!(v, 1.0, "cell {i}");
            }
        }
    }

    #[test]
    fn brush_is_centred_on_the_cursor() {
        let mut panel = panel_with(vec![1.0; MASK_SIZE * MASK_SIZE]);
        panel.update_mask(Pos2::new(32.5 / 64.0, 32.5 / 64.0), 0.0, brush(0.1), 0.05);
        let mask = panel.mask.as_ref().unwrap();
        for d in 1..4 {
            assert_eq!(mask[(32 - d) + 32 * 64], mask[(32 + d) + 32 * 64], "d={d}");
        }
    }

    #[test]
    fn undo_restores_the_previous_mask_and_feather() {
        let mut panel = panel_with(vec![1.0; MASK_SIZE * MASK_SIZE]);
        panel.feather = 0.3;
        panel.push_undo();
        panel.mask.as_mut().unwrap().fill(0.0);
        panel.feather = 0.8;
        match panel.undo() {
            Some(Panel2dAction::MaskCommitted { mask, feather }) => {
                assert!(mask.iter().all(|&v| v == 1.0));
                assert_eq!(feather, 0.3);
            }
            _ => panic!("expected MaskCommitted"),
        }
        assert!(panel.undo().is_none());
    }

    #[test]
    fn undo_depth_is_bounded() {
        let mut panel = panel_with(vec![0.0; 4]);
        for k in 0..25 {
            panel.feather = k as f32;
            panel.push_undo();
        }
        assert_eq!(panel.undo.len(), UNDO_DEPTH);
        assert_eq!(panel.undo[0].1, 5.0);
    }

    #[test]
    fn display_mask_clears_undo() {
        let mut panel = panel_with(vec![0.0; 4]);
        panel.push_undo();
        panel.display_mask(256, vec![1.0; 4], 0.0);
        assert!(panel.undo.is_empty());
    }

    #[test]
    fn displayed_mask_is_feathered_between_strokes() {
        let mut panel = PanelMaskEdit::new(256);
        panel.display_mask(256, crate::mask::tests::half_black_mask(), 0.5);
        let i = 32 + 10 * MASK_SIZE;
        assert!((panel.displayed_mask().unwrap()[i] - 0.125).abs() < 1e-6);
        panel.is_painting = true;
        assert_eq!(panel.displayed_mask().unwrap()[i], 1.0);
    }
}
