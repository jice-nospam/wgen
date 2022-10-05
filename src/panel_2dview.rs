use eframe::egui;
use egui_extras::RetainedImage;
use epaint::{Color32, ColorImage};

use crate::{fps::FpsCounter, panel_maskedit::PanelMaskEdit, worldgen::ExportMap};

pub enum Panel2dAction {
    /// inform the main program that the preview size has changed. terrain/3d view must be recomputed
    ResizePreview(usize),
    /// inform the main program that mask must be copied to the generator panel
    MaskUpdated,
}
pub struct Panel2dView {
    /// preview image of the heightmap
    img: ColorImage,
    /// minimum value in the heightmap
    min: f32,
    /// maximum value in the heightmap
    max: f32,
    /// are we displaying the mask editor ?
    mask_mode: bool,
    /// size of the preview canvas in pixels
    image_size: usize,
    /// size of the heightmap
    preview_size: usize,
    /// should we update the preview every time a step is computed ?
    pub live_preview: bool,
    /// utility to display FPS
    fps_counter: FpsCounter,
    /// egui renderable image
    ui_img: Option<RetainedImage>,
    /// mask editor subpanel
    mask_editor: PanelMaskEdit,
}

impl Panel2dView {
    pub fn new(image_size: usize, preview_size: u32, hmap: &ExportMap) -> Self {
        let mut panel = Panel2dView {
            img: ColorImage::new([image_size, image_size], Color32::BLACK),
            min: 0.0,
            max: 0.0,
            image_size,
            mask_mode: false,
            live_preview: true,
            preview_size: preview_size as usize,
            fps_counter: FpsCounter::default(),
            ui_img: None,
            mask_editor: PanelMaskEdit::new(image_size),
        };
        panel.refresh(image_size, preview_size, Some(hmap));
        panel
    }
    pub fn get_current_mask(&self) -> Option<Vec<f32>> {
        self.mask_editor.get_mask()
    }
    pub fn display_mask(&mut self, image_size: usize, preview_size: u32, mask: Option<Vec<f32>>) {
        self.image_size = image_size;
        self.preview_size = preview_size as usize;
        self.mask_editor.display_mask(image_size, mask);
        self.mask_mode = true;
    }
    pub fn refresh(&mut self, image_size: usize, preview_size: u32, hmap: Option<&ExportMap>) {
        self.image_size = image_size;
        self.mask_mode = false;
        self.preview_size = preview_size as usize;
        if self.img.width() != image_size {
            self.img = ColorImage::new([self.image_size, self.image_size], Color32::BLACK);
        }
        if let Some(hmap) = hmap {
            let (min, max) = hmap.get_min_max();
            let coef = if max - min > std::f32::EPSILON {
                1.0 / (max - min)
            } else {
                1.0
            };
            self.min = min;
            self.max = max;
            let mut idx = 0;
            for y in 0..image_size {
                let py = ((y * preview_size as usize) as f32 / image_size as f32) as usize;
                for x in 0..image_size {
                    let px = ((x * preview_size as usize) as f32 / image_size as f32) as usize;
                    let mut h = hmap.height(px as usize, py as usize);
                    h = (h - min) * coef;
                    self.img.pixels[idx] = Color32::from_gray((h * 255.0).clamp(0.0, 255.0) as u8);
                    idx += 1;
                }
            }
        };
        self.ui_img = Some(RetainedImage::from_color_image("hmap", self.img.clone()));
    }
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<Panel2dAction> {
        let mut action = None;
        let old_size = self.preview_size;
        self.fps_counter.new_frame();
        if self.mask_mode {
            action = self.mask_editor.render(ui);
        } else {
            ui.vertical(|ui| {
                if let Some(img) = &self.ui_img {
                    img.show(ui);
                }
                ui.horizontal(|ui| {
                    ui.label(format!("Height range : {} - {}", self.min, self.max));
                });
            });
        }
        ui.label(format!("FPS : {}", self.fps_counter.fps()));
        ui.horizontal(|ui| {
            ui.label("Preview size");
            egui::ComboBox::from_label("")
                .selected_text(format!("{}x{}", self.preview_size, self.preview_size))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.preview_size, 64, "64x64");
                    ui.selectable_value(&mut self.preview_size, 128, "128x128");
                    ui.selectable_value(&mut self.preview_size, 256, "256x256");
                    ui.selectable_value(&mut self.preview_size, 512, "512x512");
                });
            ui.label("Live preview");
            ui.checkbox(&mut self.live_preview, "");
        });
        if self.preview_size != old_size {
            action = Some(Panel2dAction::ResizePreview(self.preview_size));
        }
        action
    }
}
