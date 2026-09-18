use eframe::{
    egui::{self, PointerButton, TextureId, TextureOptions},
    emath,
};
use epaint::{Color32, ColorImage, Pos2, Rect, Stroke, TextureHandle};

use crate::{panel_2dview::Panel2dAction, MASK_SIZE};

/// maximum size of the brush relative to the canvas
const MAX_BRUSH_SIZE: f32 = 0.25;

#[derive(Clone, Copy)]
pub struct BrushConfig {
    /// value painted with middle mouse button
    pub value: f32,
    /// brush radius from a single 'pixel' in the heightmap (0.0) to 25% of heightmap's size (1.0)
    pub size: f32,
    /// brush radius where the opacity starts to falloff from no falloff(0.0) to center of the brush (1.0)
    pub falloff: f32,
    /// how fast the brush updates the mask 0.0: slow, 1.0: fast
    pub opacity: f32,
}
pub struct PanelMaskEdit {
    /// preview canvas size in pixels
    image_size: usize,
    /// the mask as a MASK_SIZE x MASK_SIZE f32 matrix
    mask: Option<Vec<f32>>,
    /// the brush parameters
    conf: BrushConfig,
    /// GPU texture of `mask`, a MASK_SIZE x MASK_SIZE grayscale image
    mask_tex: Option<TextureHandle>,
    /// `mask` changed since the last upload to `mask_tex`
    mask_dirty: bool,
    /// are we currently modifying the mask (cursor is in canvas and one mouse button is pressed)
    is_painting: bool,
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
            conf: BrushConfig {
                value: 0.5,
                size: 0.5,
                falloff: 0.5,
                opacity: 0.5,
            },
            mask_tex: None,
            mask_dirty: false,
            is_painting: false,
            prev_frame_time: -1.0,
            heightmap_transparency: 0.5,
        }
    }
    pub fn display_mask(&mut self, image_size: usize, mask: Vec<f32>) {
        self.image_size = image_size;
        self.mask_dirty = true;
        self.is_painting = false;
        self.mask = Some(mask);
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
                    action = self.mask.clone().map(Panel2dAction::MaskCommitted);
                }
            }
            ui.label("mouse buttons : left increase, right decrease, middle set brush value");
            ui.horizontal(|ui| {
                ui.label("brush size");
                ui.add(
                    egui::DragValue::new(&mut self.conf.size)
                        .speed(0.01)
                        .range(1.0 / (MASK_SIZE as f32)..=1.0),
                );
                ui.label("falloff");
                ui.add(
                    egui::DragValue::new(&mut self.conf.falloff)
                        .speed(0.01)
                        .range(0.0..=1.0),
                );
                ui.label("value");
                ui.add(
                    egui::DragValue::new(&mut self.conf.value)
                        .speed(0.01)
                        .range(0.0..=1.0),
                );
                ui.label("opacity");
                ui.add(
                    egui::DragValue::new(&mut self.conf.opacity)
                        .speed(0.01)
                        .range(0.0..=1.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("heightmap opacity");
                ui.add(
                    egui::DragValue::new(&mut self.heightmap_transparency)
                        .speed(0.01)
                        .range(0.0..=1.0),
                );
            });
            if ui
                .button("Clear mask")
                .on_hover_text("Delete this mask")
                .clicked()
            {
                action = Some(Panel2dAction::MaskDelete);
                if let Some(ref mut mask) = self.mask {
                    mask.fill(1.0);
                    self.mask_dirty = true;
                }
            }
        });
        action
    }
    /// allocates the canvas, applies the brush under the pointer, then paints mask, heightmap and brush
    fn paint_canvas(&mut self, ui: &mut egui::Ui, heightmap_id: TextureId) {
        let (rect, response) = ui.allocate_exact_size(
            egui::Vec2::splat(self.image_size as f32),
            egui::Sense::drag(),
        );
        let lbutton = ui.input(|i| i.pointer.button_down(PointerButton::Primary));
        let rbutton = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
        let mbutton = ui.input(|i| i.pointer.button_down(PointerButton::Middle));
        let mouse_pos = ui.input(|i| i.pointer.hover_pos());
        let to_screen = emath::RectTransform::from_to(
            Rect::from_min_size(Pos2::ZERO, response.rect.square_proportions()),
            response.rect,
        );
        let from_screen = to_screen.inverse();
        let brush_config = self.conf;
        let time = if self.prev_frame_time == -1.0 {
            self.prev_frame_time = ui.input(|i| i.time);
            0.0
        } else {
            let t = ui.input(|i| i.time);
            let elapsed = t - self.prev_frame_time;
            self.prev_frame_time = t;
            elapsed
        };
        // pointer position in canvas from 0.0,0.0 (top left) to 1.0,1.0 (bottom right)
        let canvas_pos = mouse_pos.map(|pos| from_screen * pos);
        if let Some(canvas_pos) = canvas_pos {
            self.is_painting = (lbutton || rbutton || mbutton) && in_canvas(canvas_pos);
            if self.is_painting && time > 0.0 {
                self.update_mask(canvas_pos, lbutton, rbutton, brush_config, time as f32);
                self.mask_dirty = true;
            }
        } else {
            // the pointer left the window : the stroke is over
            self.is_painting = false;
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
            let r_px = brush_config.size * MAX_BRUSH_SIZE * rect.width();
            painter.circle_stroke(pos, r_px, Stroke::new(1.5_f32, Color32::RED));
            painter.circle_stroke(
                pos,
                r_px * (1.0 - brush_config.falloff),
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 0, 0, 110)),
            );
        }
    }
    /// uploads `mask` to `mask_tex` when it changed since the last frame
    fn upload_mask(&mut self, ctx: &egui::Context) {
        if !self.mask_dirty {
            return;
        }
        if let Some(mask) = &self.mask {
            let img = mask_image(mask);
            match &mut self.mask_tex {
                Some(handle) => handle.set(img, TextureOptions::LINEAR),
                None => self.mask_tex = Some(ctx.load_texture("mask", img, TextureOptions::LINEAR)),
            }
        }
        self.mask_dirty = false;
    }

    fn update_mask(
        &mut self,
        canvas_pos: Pos2,
        lbutton: bool,
        rbutton: bool,
        brush_config: BrushConfig,
        time: f32,
    ) {
        if let Some(ref mut mask) = self.mask {
            let mx = canvas_pos.x * MASK_SIZE as f32;
            let my = canvas_pos.y * MASK_SIZE as f32;
            let brush_radius = brush_config.size * MASK_SIZE as f32 * MAX_BRUSH_SIZE;
            let falloff_dist = (1.0 - brush_config.falloff) * brush_radius;
            let minx = (mx - brush_radius).max(0.0) as usize;
            let maxx = ((mx + brush_radius) as usize).min(MASK_SIZE);
            let miny = (my - brush_radius).max(0.0) as usize;
            let maxy = ((my + brush_radius) as usize).min(MASK_SIZE);
            let opacity_factor = 0.5 + brush_config.opacity;
            let (target_value, time_coef) = if lbutton {
                (0.0, 10.0)
            } else if rbutton {
                // for some unknown reason, white color is faster than black!
                (1.0, 3.0)
            } else {
                // mbutton
                (brush_config.value, 5.0)
            };
            let brush_coef = 1.0 / (brush_radius - falloff_dist);
            let coef = time * time_coef * opacity_factor;
            for y in miny..maxy {
                let dy = y as f32 - my;
                let yoff = y * MASK_SIZE;
                for x in minx..maxx {
                    let dx = x as f32 - mx;
                    // distance from brush center
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist >= brush_radius {
                        // out of the brush
                        continue;
                    }
                    let alpha = if dist < falloff_dist {
                        1.0
                    } else {
                        1.0 - (dist - falloff_dist) * brush_coef
                    };
                    let current_value = mask[x + yoff];
                    mask[x + yoff] = current_value + coef * alpha * (target_value - current_value);
                }
            }
        }
    }
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
    ColorImage::from_gray([MASK_SIZE, MASK_SIZE], &bytes)
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

    #[test]
    fn update_mask_darkens_centre_only() {
        let mut panel = PanelMaskEdit::new(256);
        panel.display_mask(256, vec![1.0; MASK_SIZE * MASK_SIZE]);
        let conf = BrushConfig {
            value: 0.5,
            size: 0.5,
            falloff: 0.5,
            opacity: 0.5,
        };
        panel.update_mask(Pos2::new(0.5, 0.5), true, false, conf, 0.1);
        let mask = panel.mask.as_ref().unwrap();
        let centre = MASK_SIZE / 2;
        assert!(mask[centre + centre * MASK_SIZE] < 1.0);
        assert_eq!(mask[0], 1.0);
    }
}
