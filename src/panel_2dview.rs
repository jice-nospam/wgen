use eframe::egui;
use egui_extras::RetainedImage;
use epaint::{Color32, ColorImage};
use image::{imageops::FilterType, GrayImage, Luma};

use crate::{fps::FpsCounter, generators::get_min_max, worldgen::ExportMap};

pub enum Panel2dAction {
    ResizePreview(usize),
}
pub struct Panel2dView {
    buff: GrayImage,
    min: f32,
    max: f32,
    mask_mode: bool,
    image_size: usize,
    preview_size: usize,
    pub live_preview: bool,
    fps_counter: FpsCounter,
    img: Option<RetainedImage>,
}

impl Panel2dView {
    pub fn new(image_size: usize, preview_size: u32, hmap: &ExportMap) -> Self {
        let mut panel = Panel2dView {
            buff: GrayImage::new(1, 1),
            min: 0.0,
            max: 0.0,
            image_size,
            mask_mode: false,
            live_preview: true,
            preview_size: preview_size as usize,
            fps_counter: FpsCounter::default(),
            img: None,
        };
        panel.refresh(image_size, preview_size, Some(hmap));
        panel
    }
    pub fn display_mask(&mut self, image_size: usize, preview_size: u32, mask: Option<Vec<f32>>) {
        self.image_size = image_size;
        self.mask_mode = true;
        self.preview_size = preview_size as usize;
        let buff = if let Some(mask) = mask {
            let (min, max) = get_min_max(&mask);
            let coef = if max - min > std::f32::EPSILON {
                1.0 / (max - min)
            } else {
                1.0
            };
            GrayImage::from_fn(preview_size, preview_size, |x, y| {
                let mut h = mask[x as usize + y as usize * preview_size as usize];
                h = (h - min) * coef;
                Luma([(h * 255.0).clamp(0.0, 255.0) as u8])
            })
        } else {
            let mut img = GrayImage::new(1, 1);
            img.fill(255);
            img
        };
        self.buff = image::imageops::resize(
            &buff,
            self.image_size as u32,
            self.image_size as u32,
            FilterType::Nearest,
        );
        let mut img = ColorImage::new([self.image_size, self.image_size], Color32::BLACK);
        for y in 0..self.image_size {
            for x in 0..self.image_size {
                let rgb = self.buff.get_pixel(x as u32, y as u32)[0];
                img[(x, y)][0] = rgb;
                img[(x, y)][1] = rgb;
                img[(x, y)][2] = rgb;
            }
        }
        self.img = Some(RetainedImage::from_color_image("mask", img));
    }
    pub fn refresh(&mut self, image_size: usize, preview_size: u32, hmap: Option<&ExportMap>) {
        self.image_size = image_size;
        self.mask_mode = false;
        self.preview_size = preview_size as usize;
        let buff = if let Some(hmap) = hmap {
            let (min, max) = hmap.get_min_max();
            let coef = if max - min > std::f32::EPSILON {
                1.0 / (max - min)
            } else {
                1.0
            };
            self.min = min;
            self.max = max;
            GrayImage::from_fn(preview_size, preview_size, |x, y| {
                let mut h = hmap.height(x as usize, y as usize);
                h = (h - min) * coef;
                Luma([(h * 255.0).clamp(0.0, 255.0) as u8])
            })
        } else {
            GrayImage::new(1, 1)
        };
        self.buff = image::imageops::resize(
            &buff,
            self.image_size as u32,
            self.image_size as u32,
            FilterType::Nearest,
        );
        let mut img = ColorImage::new([self.image_size, self.image_size], Color32::BLACK);
        for y in 0..self.image_size {
            for x in 0..self.image_size {
                let rgb = self.buff.get_pixel(x as u32, y as u32)[0];
                img[(x, y)][0] = rgb;
                img[(x, y)][1] = rgb;
                img[(x, y)][2] = rgb;
            }
        }
        self.img = Some(RetainedImage::from_color_image("hmap", img));
    }
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<Panel2dAction> {
        self.fps_counter.new_frame();
        let old_size = self.preview_size;
        ui.vertical(|ui| {
            if let Some(img) = &self.img {
                img.show(ui);
            }
            ui.label(format!("FPS : {}", self.fps_counter.fps()));
            if self.mask_mode {
                ui.label("Use left and right mouse buttons to edit the mask");
            } else {
                ui.label(format!("Height range : {} - {}", self.min, self.max));
            }
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
        });
        if self.preview_size != old_size {
            return Some(Panel2dAction::ResizePreview(self.preview_size));
        }
        None
    }
}
